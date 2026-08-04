use super::{clash_api, logger::Logger};
use crate::log_err;
use crate::{config::*, utils::dirs};
use anyhow::{bail, Context, Result};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
#[cfg(target_os = "linux")]
use std::path::Path;
use std::{
    fs,
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use sysinfo::{Pid, System};
use tauri::api::process::{Command, CommandChild, CommandEvent};
use tokio::sync::Mutex as TokioMutex;
use tokio::time::sleep;

#[derive(Debug)]
pub struct CoreManager {
    sidecar: Arc<Mutex<Option<CommandChild>>>,

    #[cfg(target_os = "windows")]
    use_service_mode: Arc<Mutex<bool>>,

    core_operation: TokioMutex<()>,
    generation: AtomicU64,
    active_generation: AtomicU64,
    desired_running: AtomicBool,
    core_ready: AtomicBool,
    recovery_scheduled: AtomicBool,
    proxy_initialized: AtomicBool,
}

impl CoreManager {
    pub fn global() -> &'static CoreManager {
        static CORE_MANAGER: OnceCell<CoreManager> = OnceCell::new();

        CORE_MANAGER.get_or_init(|| CoreManager {
            sidecar: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "windows")]
            use_service_mode: Arc::new(Mutex::new(false)),
            core_operation: TokioMutex::new(()),
            generation: AtomicU64::new(0),
            active_generation: AtomicU64::new(0),
            desired_running: AtomicBool::new(false),
            core_ready: AtomicBool::new(false),
            recovery_scheduled: AtomicBool::new(false),
            proxy_initialized: AtomicBool::new(false),
        })
    }

    pub fn init(&self) -> Result<()> {
        // kill old clash process
        let _ = dirs::clash_pid_path()
            .and_then(|path| fs::read(path).map(|p| p.to_vec()).context(""))
            .and_then(|pid| String::from_utf8_lossy(&pid).parse().context(""))
            .map(|pid| {
                let mut system = System::new();
                system.refresh_all();
                if let Some(proc) = system.process(Pid::from_u32(pid)) {
                    if proc.name().contains("clash") {
                        log::debug!(target: "app", "kill old clash process");
                        proc.kill();
                    }
                }
            });

        tauri::async_runtime::spawn(async {
            // 启动clash
            let manager = Self::global();
            match manager.run_core().await {
                Ok(()) => manager.initialize_sysproxy_after_core_ready().await,
                Err(err) => {
                    log::error!(target: "app", "initial core start failed: {err}");
                    manager.schedule_core_recovery();
                }
            }
        });

        Ok(())
    }

    /// 检查订阅是否正确
    pub fn check_config(&self) -> Result<()> {
        let config_path = Config::generate_file(ConfigType::Check)?;
        let config_path = dirs::path_to_str(&config_path)?;

        let clash_core = { Config::verge().latest().clash_core.clone() };
        let clash_core = clash_core.unwrap_or_else(|| MIHOMO_CORE.into());

        let app_dir = dirs::app_home_dir()?;
        let app_dir = dirs::path_to_str(&app_dir)?;

        let output = Command::new_sidecar(clash_core)?
            .args(["-t", "-d", app_dir, "-f", config_path])
            .output()?;

        if !output.status.success() {
            let error = clash_api::parse_check_output(output.stdout.clone());
            let error = match !error.is_empty() {
                true => error,
                false => output.stdout.clone(),
            };
            Logger::global().set_log(output.stdout);
            bail!("{error}");
        }

        Ok(())
    }

    /// 启动核心
    pub async fn run_core(&self) -> Result<()> {
        self.desired_running.store(true, Ordering::SeqCst);
        let result = self.run_core_inner().await;
        if result.is_err() && self.desired_running.load(Ordering::SeqCst) && !self.is_core_ready() {
            Self::global().schedule_core_recovery();
        }
        result
    }

    async fn run_core_inner(&self) -> Result<()> {
        let _operation = self.core_operation.lock().await;
        self.start_core_locked().await
    }

    async fn recover_core_if_needed(&self) -> Result<bool> {
        let _operation = self.core_operation.lock().await;
        if !self.desired_running.load(Ordering::SeqCst) || self.is_core_ready() {
            return Ok(false);
        }
        self.start_core_locked().await?;
        Ok(true)
    }

    async fn start_core_locked(&self) -> Result<()> {
        let config_path = Config::generate_file(ConfigType::Run)?;

        self.core_ready.store(false, Ordering::SeqCst);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.active_generation.store(generation, Ordering::SeqCst);
        log::info!(target: "app", "starting core with runtime config: {}", dirs::path_to_str(&config_path)?);
        self.log_tun_prerequisites();

        let killed_sidecar = match self.sidecar.lock().take() {
            Some(child) => {
                log::debug!(target: "app", "stop the core by sidecar");
                let _ = child.kill();
                true
            }
            None => false,
        };

        #[cfg(target_os = "windows")]
        let should_kill = if *self.use_service_mode.lock() {
            log::debug!(target: "app", "stop the core by service");
            log_err!(super::win_service::stop_core_by_service().await);
            true
        } else {
            killed_sidecar
        };
        #[cfg(not(target_os = "windows"))]
        let should_kill = killed_sidecar;

        // 这里得等一会儿
        if should_kill {
            sleep(Duration::from_millis(500)).await;
        }

        #[cfg(target_os = "windows")]
        {
            use super::win_service;

            // 服务模式
            let enable = { Config::verge().latest().enable_service_mode };
            let enable = enable.unwrap_or(false);

            *self.use_service_mode.lock() = enable;

            if enable {
                // 服务模式启动失败直接报错，避免误判为服务托管
                log::debug!(target: "app", "try to run core in service mode");
                let tun_enabled = Config::verge().latest().enable_tun_mode.unwrap_or(false);

                match (|| async {
                    win_service::ensure_service_ready().await?;
                    win_service::run_core_by_service(&config_path).await
                })()
                .await
                {
                    Ok(_) => {
                        self.core_ready.store(true, Ordering::SeqCst);
                        Self::global().start_service_monitor(generation);
                        return Ok(());
                    }
                    Err(err) => {
                        self.active_generation
                            .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
                            .ok();
                        self.core_ready.store(false, Ordering::SeqCst);
                        log_err!(win_service::stop_core_by_service().await);
                        log::error!(target: "app", "Service Mode failed; service could not start Mihomo core. {err}");
                        if tun_enabled {
                            bail!(
                                "Tun mode requires a working clash-verge-service on Windows: {err}"
                            );
                        }
                        bail!("Service Mode failed; service could not start Mihomo core. {err}");
                    }
                }
            }
        }

        let app_dir = dirs::app_home_dir()?;
        let app_dir = dirs::path_to_str(&app_dir)?;

        let clash_core = { Config::verge().latest().clash_core.clone() };
        let clash_core = clash_core.unwrap_or_else(|| MIHOMO_CORE.into());

        let config_path = dirs::path_to_str(&config_path)?;

        let args = vec!["-d", app_dir, "-f", config_path];

        let spawn_result = (|| -> Result<_> {
            let cmd = Command::new_sidecar(clash_core)?;
            Ok(cmd.args(args).spawn()?)
        })();
        let (mut rx, cmd_child) = match spawn_result {
            Ok(child) => child,
            Err(err) => {
                self.active_generation
                    .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
                    .ok();
                return Err(err);
            }
        };

        // 将pid写入文件中
        crate::log_err!((|| {
            let pid = cmd_child.pid();
            let path = dirs::clash_pid_path()?;
            fs::File::create(path)
                .context("failed to create the pid file")?
                .write(format!("{pid}").as_bytes())
                .context("failed to write pid to the file")?;
            <Result<()>>::Ok(())
        })());

        {
            let mut sidecar = self.sidecar.lock();
            *sidecar = Some(cmd_child);
        }

        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        log::info!(target: "app", "[mihomo]: {line}");
                        Logger::global().set_log(line);
                    }
                    CommandEvent::Stderr(err) => {
                        log::error!(target: "app", "[mihomo]: {err}");
                        Logger::global().set_log(err);
                    }
                    CommandEvent::Error(err) => {
                        log::error!(target: "app", "[mihomo]: {err}");
                        Logger::global().set_log(err);
                    }
                    CommandEvent::Terminated(_) => {
                        log::info!(target: "app", "Mihomo core terminated");
                        CoreManager::global().handle_core_terminated(generation);
                        break;
                    }
                    _ => {}
                }
            }
        });

        if let Err(err) = clash_api::wait_for_core_ready(Duration::from_secs(12)).await {
            if self
                .active_generation
                .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                if let Some(child) = self.sidecar.lock().take() {
                    let _ = child.kill();
                }
            }
            self.core_ready.store(false, Ordering::SeqCst);
            return Err(err).context("wait for Mihomo core readiness");
        }

        if self.active_generation.load(Ordering::SeqCst) != generation {
            bail!("Mihomo core terminated while readiness was being confirmed");
        }
        self.core_ready.store(true, Ordering::SeqCst);

        Ok(())
    }

    fn log_tun_prerequisites(&self) {
        let tun_enabled = Config::verge().latest().enable_tun_mode.unwrap_or(false);
        if !tun_enabled {
            return;
        }
        #[cfg(target_os = "windows")]
        {
            use deelevate::{PrivilegeLevel, Token};
            let service_mode = Config::verge()
                .latest()
                .enable_service_mode
                .unwrap_or(false);
            let privilege = Token::with_current_process()
                .ok()
                .and_then(|t| t.privilege_level().ok());
            let is_admin = matches!(privilege, Some(PrivilegeLevel::Elevated));
            if !service_mode {
                log::error!(target: "app", "Tun mode is enabled but service mode is disabled on Windows. This usually fails without admin/wintun permissions.");
                super::handle::Handle::emit_log(
                    "error",
                    "[service] Tun mode is enabled but service mode is disabled on Windows.",
                );
            } else {
                log::info!(target: "app", "Tun mode enabled on Windows with service mode.");
                super::handle::Handle::emit_log(
                    "info",
                    "[service] Tun mode enabled on Windows with service mode.",
                );
            }
            if !is_admin {
                log::warn!(target: "app", "Current process is not elevated. If service mode is unavailable, Tun setup may fail due to missing admin privileges/wintun route permissions.");
                super::handle::Handle::emit_log("warn", "[service] Current process is not elevated. Tun setup may fail due to missing admin privileges/wintun route permissions.");
            }
            log::info!(target: "app", "Windows Tun diagnostics: ensure clash-verge-service is active, wintun driver can be loaded, and firewall allows route/DNS hijack operations.");
            super::handle::Handle::emit_log("info", "[tun] Windows Tun diagnostics: ensure service active, wintun loadable, and firewall allows route/DNS hijack operations.");
        }
        #[cfg(target_os = "linux")]
        {
            if !Path::new("/dev/net/tun").exists() {
                log::error!(target: "app", "Tun mode requires /dev/net/tun on Linux, but it does not exist.");
            }
            log::info!(target: "app", "Tun mode on Linux requires CAP_NET_ADMIN and iptables/nftables permissions.");
        }
        #[cfg(target_os = "macos")]
        {
            log::info!(target: "app", "Tun mode on macOS requires network extension / route permissions.");
        }
    }

    pub fn is_core_ready(&self) -> bool {
        self.core_ready.load(Ordering::SeqCst)
    }

    async fn initialize_sysproxy_after_core_ready(&'static self) {
        if self.proxy_initialized.load(Ordering::SeqCst) {
            return;
        }

        let mut retry_delay = Duration::from_secs(1);
        while self.desired_running.load(Ordering::SeqCst) && self.is_core_ready() {
            match super::sysopt::Sysopt::global().init_sysproxy() {
                Ok(()) => {
                    self.proxy_initialized.store(true, Ordering::SeqCst);
                    return;
                }
                Err(err) => {
                    log::error!(target: "app", "system proxy initialization failed; retrying: {err}");
                }
            }
            sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(Duration::from_secs(15));
        }
    }

    fn handle_core_terminated(&'static self, generation: u64) {
        if self
            .active_generation
            .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log::debug!(target: "app", "ignore stale core termination event for generation {generation}");
            return;
        }

        self.sidecar.lock().take();
        self.core_ready.store(false, Ordering::SeqCst);
        if self.desired_running.load(Ordering::SeqCst) {
            self.schedule_core_recovery();
        }
    }

    fn schedule_core_recovery(&'static self) {
        if self
            .recovery_scheduled
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        tauri::async_runtime::spawn(async move {
            let mut retry_delay = Duration::from_secs(2);
            while self.desired_running.load(Ordering::SeqCst) && !self.is_core_ready() {
                sleep(retry_delay).await;
                if !self.desired_running.load(Ordering::SeqCst) || self.is_core_ready() {
                    break;
                }

                match self.recover_core_if_needed().await {
                    Ok(true) => break,
                    Ok(false) => break,
                    Err(err) => {
                        log::error!(target: "app", "failed to recover Mihomo core: {err}");
                        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                    }
                }
            }
            self.recovery_scheduled.store(false, Ordering::SeqCst);
            if self.desired_running.load(Ordering::SeqCst) {
                if self.is_core_ready() {
                    self.initialize_sysproxy_after_core_ready().await;
                } else {
                    self.schedule_core_recovery();
                }
            }
        });
    }

    #[cfg(target_os = "windows")]
    fn start_service_monitor(&'static self, generation: u64) {
        tauri::async_runtime::spawn(async move {
            let mut consecutive_failures = 0u8;
            loop {
                sleep(Duration::from_secs(2)).await;
                if !self.desired_running.load(Ordering::SeqCst)
                    || self.active_generation.load(Ordering::SeqCst) != generation
                    || !*self.use_service_mode.lock()
                {
                    return;
                }

                if super::win_service::is_service_core_running().await {
                    consecutive_failures = 0;
                    continue;
                }

                consecutive_failures += 1;
                if consecutive_failures < 3 {
                    continue;
                }

                if self
                    .active_generation
                    .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    log::error!(target: "app", "service-managed Mihomo core stopped; scheduling recovery");
                    self.core_ready.store(false, Ordering::SeqCst);
                    self.schedule_core_recovery();
                }
                return;
            }
        });
    }

    async fn stop_core_async(&self) -> Result<()> {
        let _operation = self.core_operation.lock().await;
        self.desired_running.store(false, Ordering::SeqCst);
        self.core_ready.store(false, Ordering::SeqCst);
        self.active_generation.store(0, Ordering::SeqCst);

        #[cfg(target_os = "windows")]
        if *self.use_service_mode.lock() {
            log::debug!(target: "app", "stop the core by service");
            super::win_service::stop_core_by_service().await?;
            return Ok(());
        }

        if let Some(child) = self.sidecar.lock().take() {
            log::debug!(target: "app", "stop the core by sidecar");
            let _ = child.kill();
        }
        Ok(())
    }

    /// 停止核心运行
    pub fn stop_core(&self) -> Result<()> {
        tauri::async_runtime::block_on(self.stop_core_async())
    }

    /// 切换核心
    pub async fn change_core(&self, clash_core: Option<String>) -> Result<()> {
        let clash_core = clash_core.ok_or(anyhow::anyhow!("Mihomo core is null"))?;
        if !VALID_MIHOMO_CORES.contains(&clash_core.as_str()) {
            bail!("invalid Mihomo core name \"{clash_core}\"");
        }

        log::debug!(target: "app", "change core to `{clash_core}`");

        Config::verge().draft().clash_core = Some(clash_core);
        let mut restart_attempted = false;
        let change_result = async {
            // 更新订阅
            Config::generate()?;
            self.check_config()?;

            // 清掉旧日志
            Logger::global().clear_log();
            restart_attempted = true;
            self.run_core().await?;
            Config::verge().latest().save_file()?;
            <Result<()>>::Ok(())
        }
        .await;

        match change_result {
            Ok(_) => {
                Config::verge().apply();
                Config::runtime().apply();
                Ok(())
            }
            Err(err) => {
                Config::verge().discard();
                Config::runtime().discard();

                if restart_attempted {
                    let rollback = async {
                        Config::generate()?;
                        self.run_core().await?;
                        Config::runtime().apply();
                        <Result<()>>::Ok(())
                    }
                    .await;
                    if let Err(rollback_err) = rollback {
                        return Err(anyhow::anyhow!(
                            "{err}; restoring the previous core also failed: {rollback_err}"
                        ));
                    }
                }
                Err(err)
            }
        }
    }

    /// 更新proxies那些
    /// 如果涉及端口和外部控制则需要重启
    pub async fn update_config(&self) -> Result<()> {
        log::debug!(target: "app", "try to update clash config");

        // 更新订阅
        Config::generate()?;

        // 检查订阅是否正常
        self.check_config()?;

        // 更新运行时订阅
        let path = Config::generate_file(ConfigType::Run)?;
        let path = dirs::path_to_str(&path)?;

        // 发送请求 发送5次
        for i in 0..5 {
            match clash_api::put_configs(path).await {
                Ok(_) => break,
                Err(err) => {
                    if i < 4 {
                        log::info!(target: "app", "{err}");
                    } else {
                        bail!(err);
                    }
                }
            }
            sleep(Duration::from_millis(250)).await;
        }

        Ok(())
    }
}
