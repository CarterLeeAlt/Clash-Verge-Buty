use crate::{config::Config, core::CoreManager, log_err};
use anyhow::{anyhow, Context, Result};
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::{
    net::{SocketAddr, TcpStream},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use sysproxy::Sysproxy;
#[cfg(target_os = "windows")]
use sysproxy::WindowsProxySnapshot;
use tauri::{async_runtime::Mutex as TokioMutex, utils::platform::current_exe};

pub struct Sysopt {
    /// current system proxy setting
    cur_sysproxy: Arc<Mutex<Option<Sysproxy>>>,

    /// record the original system proxy
    /// recover it when exit
    old_sysproxy: Arc<Mutex<Option<Sysproxy>>>,

    /// exact Windows registry state before this app owns the proxy
    #[cfg(target_os = "windows")]
    old_windows_proxy: Arc<Mutex<Option<WindowsProxySnapshot>>>,

    /// Windows registry state immediately after this app set the proxy
    #[cfg(target_os = "windows")]
    owned_windows_proxy: Arc<Mutex<Option<WindowsProxySnapshot>>>,

    /// helps to auto launch the app
    auto_launch: Arc<Mutex<Option<AutoLaunch>>>,

    /// record whether the guard async is running or not
    guard_state: Arc<TokioMutex<bool>>,

    /// serialize proxy writes, restores, and guard refreshes
    proxy_operation: Arc<Mutex<()>>,
}

#[cfg(target_os = "windows")]
static DEFAULT_BYPASS: &str = "localhost;127.*;192.168.*;10.*;172.16.*;<local>";
#[cfg(target_os = "linux")]
static DEFAULT_BYPASS: &str = "localhost,127.0.0.1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12,::1";
#[cfg(target_os = "macos")]
static DEFAULT_BYPASS: &str =
    "127.0.0.1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12,localhost,*.local,*.crashlytics.com,<local>";

fn is_sysproxy_match(actual: &Sysproxy, expected: &Sysproxy) -> bool {
    if actual.enable != expected.enable {
        return false;
    }

    if !expected.enable {
        return true;
    }

    actual.host.eq_ignore_ascii_case(&expected.host)
        && actual.port == expected.port
        && actual.bypass == expected.bypass
}

fn wait_for_local_proxy(port: u16) -> Result<()> {
    const READY_TIMEOUT: Duration = Duration::from_secs(5);
    const CONNECT_TIMEOUT: Duration = Duration::from_millis(200);
    const RETRY_DELAY: Duration = Duration::from_millis(100);

    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + READY_TIMEOUT;

    loop {
        if !CoreManager::global().is_core_ready() {
            return Err(anyhow!(
                "Clash core is not ready; refuse to use an unrelated listener on 127.0.0.1:{port}"
            ));
        }

        let last_err = match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            Ok(stream) => {
                drop(stream);
                return Ok(());
            }
            Err(err) => err,
        };

        if Instant::now() >= deadline {
            return Err(anyhow!(
                "local proxy listener 127.0.0.1:{port} was not ready within {}s: {last_err}",
                READY_TIMEOUT.as_secs()
            ));
        }
        thread::sleep(RETRY_DELAY);
    }
}

fn set_system_proxy_once(
    target: &Sysproxy,
    action: &str,
    allow_registry_fallback: bool,
) -> Result<()> {
    match target.set_system_proxy() {
        Ok(()) => Ok(()),
        Err(primary_err) => {
            #[cfg(target_os = "windows")]
            if allow_registry_fallback {
                log::warn!(target: "app", "{action}: WinINet failed; use reversible registry fallback: {primary_err}");
                return target.set_system_proxy_registry().with_context(|| {
                    format!("{action}: registry fallback failed after WinINet error: {primary_err}")
                });
            }

            #[cfg(not(target_os = "windows"))]
            let _ = (action, allow_registry_fallback);
            Err(primary_err.into())
        }
    }
}

fn set_system_proxy_with_retry(
    target: &Sysproxy,
    action: &str,
    allow_registry_fallback: bool,
) -> Result<()> {
    const BACKOFF_MS: [u64; 3] = [150, 400, 800];
    let mut last_err = None;

    for (idx, backoff) in BACKOFF_MS.iter().enumerate() {
        match set_system_proxy_once(target, action, allow_registry_fallback) {
            Ok(()) => match Sysproxy::get_system_proxy() {
                Ok(actual) => {
                    if is_sysproxy_match(&actual, target) {
                        return Ok(());
                    }

                    let err = anyhow!(
                        "{action} write verification mismatch (attempt {}/{})",
                        idx + 1,
                        BACKOFF_MS.len()
                    );
                    log::warn!(target: "app", "{err}");
                    last_err = Some(err);
                }
                Err(err) => {
                    log::warn!(target: "app", "{action}: readback failed after set succeeded, accept success: {err}");
                    return Ok(());
                }
            },
            Err(err) => {
                log::warn!(target: "app", "{action}: set system proxy failed on attempt {}/{}: {err}", idx + 1, BACKOFF_MS.len());
                last_err = Some(err);
            }
        }

        if idx + 1 < BACKOFF_MS.len() {
            thread::sleep(Duration::from_millis(*backoff));
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("{action} failed for unknown reason")))
        .with_context(|| format!("failed to {action} after {} attempts", BACKOFF_MS.len()))
}

#[cfg(target_os = "windows")]
fn clear_windows_proxy_recovery(action: &str) {
    if let Err(err) = WindowsProxySnapshot::clear_recovery() {
        log::warn!(target: "app", "{action}: could not clear Windows proxy recovery journal: {err}");
    }
}

#[cfg(target_os = "windows")]
fn rearm_windows_proxy_recovery(
    original: Option<&WindowsProxySnapshot>,
    owned: &WindowsProxySnapshot,
    target: Option<&Sysproxy>,
    action: &str,
) {
    let Some(target) = target.filter(|target| target.enable) else {
        clear_windows_proxy_recovery(action);
        return;
    };
    let original = original.unwrap_or(owned);
    if let Err(err) = original.prepare_recovery(target) {
        log::warn!(target: "app", "{action}: could not restore prepared Windows proxy recovery journal: {err}");
        return;
    }
    if let Err(err) = owned.record_owned_recovery(target) {
        log::warn!(target: "app", "{action}: could not restore owned Windows proxy recovery journal: {err}");
    }
}

impl Sysopt {
    pub fn global() -> &'static Sysopt {
        static SYSOPT: OnceCell<Sysopt> = OnceCell::new();

        SYSOPT.get_or_init(|| Sysopt {
            cur_sysproxy: Arc::new(Mutex::new(None)),
            old_sysproxy: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "windows")]
            old_windows_proxy: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "windows")]
            owned_windows_proxy: Arc::new(Mutex::new(None)),
            auto_launch: Arc::new(Mutex::new(None)),
            guard_state: Arc::new(TokioMutex::new(false)),
            proxy_operation: Arc::new(Mutex::new(())),
        })
    }

    /// Recover a Windows proxy session left by an unclean shutdown before the
    /// rest of application startup can perform network access.
    pub fn recover_pending_sysproxy(&self) -> Result<()> {
        let _operation = self.proxy_operation.lock();
        #[cfg(target_os = "windows")]
        if WindowsProxySnapshot::recover_pending()
            .context("recover pending Windows proxy session")?
        {
            log::info!(target: "app", "recovered pending Windows proxy session from a previous unclean shutdown");
        }
        Ok(())
    }

    /// init the sysproxy
    pub fn init_sysproxy(&self) -> Result<()> {
        let _operation = self.proxy_operation.lock();
        #[cfg(target_os = "windows")]
        if WindowsProxySnapshot::recover_pending()
            .context("recover pending Windows proxy session")?
        {
            log::info!(target: "app", "recovered pending Windows proxy session from a previous unclean shutdown");
        }

        let port = Config::verge()
            .latest()
            .verge_mixed_port
            .unwrap_or(Config::clash().data().get_mixed_port());

        let (enable, bypass) = {
            let verge = Config::verge();
            let verge = verge.latest();
            (
                verge.enable_system_proxy.unwrap_or(false),
                verge.system_proxy_bypass.clone(),
            )
        };

        let current = Sysproxy {
            enable,
            host: String::from("127.0.0.1"),
            port,
            bypass: bypass.unwrap_or(DEFAULT_BYPASS.into()),
        };

        if enable {
            wait_for_local_proxy(port)
                .context("wait for local proxy before enabling system proxy")?;

            #[cfg(target_os = "windows")]
            let windows_snapshot = WindowsProxySnapshot::capture()
                .context("capture Windows proxy registry before enable")?;
            let old = Sysproxy::get_system_proxy().ok();
            let should_save_old = old
                .as_ref()
                .map(|proxy| !is_sysproxy_match(proxy, &current))
                .unwrap_or(true);

            #[cfg(target_os = "windows")]
            windows_snapshot
                .prepare_recovery(&current)
                .context("persist Windows proxy recovery journal before enable")?;

            let set_result = set_system_proxy_with_retry(
                &current,
                "enable system proxy",
                cfg!(target_os = "windows"),
            );
            if let Err(err) = set_result {
                #[cfg(target_os = "windows")]
                if let Err(rollback_err) = windows_snapshot.restore() {
                    log::error!(target: "app", "initial system proxy rollback failed: {rollback_err}");
                }
                #[cfg(target_os = "windows")]
                clear_windows_proxy_recovery("initial proxy enable rollback");
                return Err(err).context("init system proxy");
            }

            #[cfg(target_os = "windows")]
            let owned_snapshot = match WindowsProxySnapshot::capture() {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    if let Err(rollback_err) = windows_snapshot.restore() {
                        log::error!(target: "app", "initial system proxy rollback failed after ownership capture error: {rollback_err}");
                    }
                    clear_windows_proxy_recovery("initial ownership capture rollback");
                    return Err(err).context("capture Windows proxy ownership after enable");
                }
            };
            #[cfg(target_os = "windows")]
            if let Err(err) = owned_snapshot.record_owned_recovery(&current) {
                log::warn!(target: "app", "could not finalize Windows proxy recovery journal; prepared recovery remains usable: {err}");
            }

            if should_save_old {
                *self.old_sysproxy.lock() = old;
                #[cfg(target_os = "windows")]
                {
                    *self.old_windows_proxy.lock() = Some(windows_snapshot);
                }
            }
            #[cfg(target_os = "windows")]
            {
                *self.owned_windows_proxy.lock() = Some(owned_snapshot);
            }
            *self.cur_sysproxy.lock() = Some(current);
        } else {
            *self.cur_sysproxy.lock() = Some(current);
        }

        // run the system proxy guard
        self.guard_proxy();
        Ok(())
    }

    /// update the system proxy
    pub fn update_sysproxy(&self) -> Result<()> {
        let _operation = self.proxy_operation.lock();
        let (enable, bypass) = {
            let verge = Config::verge();
            let verge = verge.latest();
            (
                verge.enable_system_proxy.unwrap_or(false),
                verge.system_proxy_bypass.clone(),
            )
        };

        let port = Config::verge()
            .latest()
            .verge_mixed_port
            .unwrap_or(Config::clash().data().get_mixed_port());

        let target_proxy = Sysproxy {
            enable,
            host: "127.0.0.1".into(),
            port,
            bypass: bypass.unwrap_or(DEFAULT_BYPASS.into()),
        };

        let cached_current = self.cur_sysproxy.lock().clone();
        let cached_old = self.old_sysproxy.lock().clone();
        #[cfg(target_os = "windows")]
        let cached_windows_old = self.old_windows_proxy.lock().clone();
        #[cfg(target_os = "windows")]
        let cached_windows_owned = self.owned_windows_proxy.lock().clone();

        if enable {
            wait_for_local_proxy(port)
                .context("wait for local proxy before enabling system proxy")?;

            #[cfg(target_os = "windows")]
            let windows_snapshot = WindowsProxySnapshot::capture()
                .context("capture Windows proxy registry before enable")?;
            let before = match Sysproxy::get_system_proxy() {
                Ok(proxy) => Some(proxy),
                Err(err) => {
                    #[cfg(target_os = "windows")]
                    log::warn!(target: "app", "read current system proxy before enable failed; use the raw Windows snapshot for rollback: {err}");
                    #[cfg(not(target_os = "windows"))]
                    log::warn!(target: "app", "read current system proxy before enable failed; continue without a rollback snapshot: {err}");
                    None
                }
            };
            let current_was_owned = cached_current
                .as_ref()
                .filter(|expected| expected.enable)
                .zip(before.as_ref())
                .map(|(expected, actual)| is_sysproxy_match(actual, expected))
                .unwrap_or(false);
            let should_save_old = !current_was_owned;

            #[cfg(target_os = "windows")]
            {
                let recovery_original = if should_save_old {
                    &windows_snapshot
                } else {
                    cached_windows_old.as_ref().unwrap_or(&windows_snapshot)
                };
                recovery_original
                    .prepare_recovery(&target_proxy)
                    .context("persist Windows proxy recovery journal before enable")?;
            }

            let set_result = set_system_proxy_with_retry(
                &target_proxy,
                "enable system proxy",
                cfg!(target_os = "windows"),
            );

            if let Err(err) = set_result {
                #[cfg(target_os = "windows")]
                if let Err(rollback_err) = windows_snapshot.restore() {
                    log::error!(target: "app", "enable system proxy rollback failed: {rollback_err}");
                }
                #[cfg(target_os = "windows")]
                rearm_windows_proxy_recovery(
                    cached_windows_old.as_ref(),
                    &windows_snapshot,
                    cached_current.as_ref(),
                    "proxy enable rollback",
                );
                #[cfg(not(target_os = "windows"))]
                if let Some(before) = before.as_ref() {
                    if let Err(rollback_err) = set_system_proxy_with_retry(
                        before,
                        "restore system proxy after failed enable",
                        false,
                    ) {
                        log::error!(target: "app", "enable system proxy rollback failed: {rollback_err}");
                    }
                }
                return Err(err);
            }

            #[cfg(target_os = "windows")]
            let owned_snapshot = match WindowsProxySnapshot::capture() {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    if let Err(rollback_err) = windows_snapshot.restore() {
                        log::error!(target: "app", "enable system proxy rollback failed after ownership capture error: {rollback_err}");
                    }
                    rearm_windows_proxy_recovery(
                        cached_windows_old.as_ref(),
                        &windows_snapshot,
                        cached_current.as_ref(),
                        "ownership capture rollback",
                    );
                    return Err(err).context("capture Windows proxy ownership after enable");
                }
            };
            #[cfg(target_os = "windows")]
            if let Err(err) = owned_snapshot.record_owned_recovery(&target_proxy) {
                log::warn!(target: "app", "could not finalize Windows proxy recovery journal; prepared recovery remains usable: {err}");
            }

            if should_save_old {
                *self.old_sysproxy.lock() = before;
                #[cfg(target_os = "windows")]
                {
                    *self.old_windows_proxy.lock() = Some(windows_snapshot);
                }
            }
            #[cfg(target_os = "windows")]
            {
                *self.owned_windows_proxy.lock() = Some(owned_snapshot);
            }
            *self.cur_sysproxy.lock() = Some(target_proxy);
            return Ok(());
        }

        let before =
            Sysproxy::get_system_proxy().context("read current system proxy before disable")?;
        let is_owned = cached_current
            .as_ref()
            .map(|expected| is_sysproxy_match(&before, expected))
            .unwrap_or(false);
        if !is_owned {
            log::info!(target: "app", "disable system proxy skipped because current proxy is not owned by app");
            #[cfg(target_os = "windows")]
            {
                *self.old_windows_proxy.lock() = None;
                *self.owned_windows_proxy.lock() = None;
                clear_windows_proxy_recovery("proxy ownership relinquished");
            }
            *self.old_sysproxy.lock() = None;
            *self.cur_sysproxy.lock() = Some(target_proxy);
            return Ok(());
        }

        #[cfg(target_os = "windows")]
        match (cached_windows_old, cached_windows_owned) {
            (Some(original), Some(owned)) => {
                original
                    .restore_if_unchanged(&owned)
                    .context("restore original Windows proxy registry")?;
                *self.old_windows_proxy.lock() = None;
                *self.owned_windows_proxy.lock() = None;
                clear_windows_proxy_recovery("system proxy restored");
                *self.old_sysproxy.lock() = None;
                *self.cur_sysproxy.lock() = Some(target_proxy);
                return Ok(());
            }
            (Some(_), None) => {
                log::warn!(target: "app", "original Windows proxy snapshot exists without an ownership snapshot; disable app proxy without restoring stale values");
                let mut disabled = before.clone();
                disabled.enable = false;
                set_system_proxy_with_retry(&disabled, "disable system proxy", false)?;
                *self.old_windows_proxy.lock() = None;
                clear_windows_proxy_recovery("system proxy disabled without ownership snapshot");
                *self.old_sysproxy.lock() = None;
                *self.cur_sysproxy.lock() = Some(target_proxy);
                return Ok(());
            }
            (None, Some(owned)) => {
                let current = WindowsProxySnapshot::capture()
                    .context("verify Windows proxy registry ownership before disable")?;
                if current == owned {
                    let mut disabled = before.clone();
                    disabled.enable = false;
                    set_system_proxy_with_retry(&disabled, "disable system proxy", false)?;
                } else {
                    log::info!(target: "app", "disable system proxy skipped because Windows proxy registry was changed after app enable");
                }
                *self.owned_windows_proxy.lock() = None;
                clear_windows_proxy_recovery("system proxy disabled without original snapshot");
                *self.old_sysproxy.lock() = None;
                *self.cur_sysproxy.lock() = Some(target_proxy);
                return Ok(());
            }
            _ => {}
        }

        let mut restore_target = cached_old.unwrap_or_else(|| {
            let mut disabled = before.clone();
            disabled.enable = false;
            disabled
        });

        if !restore_target.enable {
            restore_target.enable = false;
        }

        set_system_proxy_with_retry(
            &restore_target,
            if restore_target.enable {
                "restore original system proxy"
            } else {
                "disable system proxy"
            },
            false,
        )?;

        #[cfg(target_os = "windows")]
        {
            *self.owned_windows_proxy.lock() = None;
            clear_windows_proxy_recovery("system proxy disabled");
        }
        *self.cur_sysproxy.lock() = Some(target_proxy);
        Ok(())
    }

    /// reset the sysproxy
    pub fn reset_sysproxy(&self) -> Result<()> {
        let _operation = self.proxy_operation.lock();
        #[cfg(target_os = "windows")]
        let old_windows_proxy = self.old_windows_proxy.lock().take();
        #[cfg(target_os = "windows")]
        let owned_windows_proxy = self.owned_windows_proxy.lock().take();
        let (cur_sysproxy, old_sysproxy) = {
            let mut cur_sysproxy = self.cur_sysproxy.lock();
            let mut old_sysproxy = self.old_sysproxy.lock();
            (cur_sysproxy.take(), old_sysproxy.take())
        };

        #[cfg(target_os = "windows")]
        if let Some(snapshot) = old_windows_proxy {
            let parsed_owned = match (
                cur_sysproxy.as_ref(),
                Sysproxy::get_system_proxy().map_err(anyhow::Error::from),
            ) {
                (Some(cur), Ok(actual)) => is_sysproxy_match(&actual, cur),
                (Some(_), Err(err)) => {
                    log::warn!(target: "app", "skip Windows proxy restore because current ownership could not be verified: {err}");
                    false
                }
                (None, _) => false,
            };

            if parsed_owned {
                if let Some(owned_snapshot) = owned_windows_proxy.as_ref() {
                    snapshot
                        .restore_if_unchanged(owned_snapshot)
                        .context("restore original Windows proxy registry on exit")?;
                } else {
                    log::warn!(target: "app", "disable app proxy on exit without restoring an unverifiable Windows snapshot");
                    if let Some(mut current) = cur_sysproxy.clone() {
                        current.enable = false;
                        current.set_system_proxy()?;
                    }
                }
            } else {
                log::info!(target: "app", "skip Windows proxy restore because current proxy is not owned by app");
            }
            clear_windows_proxy_recovery("system proxy exit handling completed");
            return Ok(());
        }

        #[cfg(target_os = "windows")]
        if let Some(owned_snapshot) = owned_windows_proxy {
            let parsed_owned = match (
                cur_sysproxy.as_ref(),
                Sysproxy::get_system_proxy().map_err(anyhow::Error::from),
            ) {
                (Some(cur), Ok(actual)) => is_sysproxy_match(&actual, cur),
                (Some(_), Err(err)) => {
                    log::warn!(target: "app", "skip Windows proxy disable because current ownership could not be verified: {err}");
                    false
                }
                (None, _) => false,
            };
            let registry_owned = match WindowsProxySnapshot::capture() {
                Ok(current_snapshot) => current_snapshot == owned_snapshot,
                Err(err) => {
                    log::warn!(target: "app", "skip Windows proxy disable because registry ownership could not be verified: {err}");
                    false
                }
            };

            if parsed_owned && registry_owned {
                if let Some(mut current) = cur_sysproxy.clone() {
                    current.enable = false;
                    current.set_system_proxy()?;
                }
            } else {
                log::info!(target: "app", "skip Windows proxy disable because current proxy is not owned by app");
            }
            clear_windows_proxy_recovery("system proxy exit handling completed");
            return Ok(());
        }

        let mut target = None;

        if let Some(mut old) = old_sysproxy {
            // 如果原代理和当前代理 端口一致，就disable关闭，否则就恢复原代理设置
            // 当前没有设置代理的时候，不确定旧设置是否和当前一致，全关了
            let port_same = cur_sysproxy
                .as_ref()
                .map_or(true, |cur| old.port == cur.port);

            if old.enable && port_same {
                old.enable = false;
                log::info!(target: "app", "reset proxy by disabling the original proxy");
            } else {
                log::info!(target: "app", "reset proxy to the original proxy");
            }
            target = Some(old);
        } else if let Some(mut cur @ Sysproxy { enable: true, .. }) = cur_sysproxy {
            // 没有原代理，就按现在的代理设置disable即可
            log::info!(target: "app", "reset proxy by disabling the current proxy");
            cur.enable = false;
            target = Some(cur);
        } else {
            log::info!(target: "app", "reset proxy with no action");
        }

        if let Some(proxy) = target {
            proxy.set_system_proxy()?;
        }

        #[cfg(target_os = "windows")]
        clear_windows_proxy_recovery("system proxy exit handling completed");

        Ok(())
    }

    /// init the auto launch
    pub fn init_launch(&self) -> Result<()> {
        let app_exe = current_exe()?;
        let app_exe = dunce::canonicalize(app_exe)?;
        let app_name = app_exe
            .file_stem()
            .and_then(|f| f.to_str())
            .ok_or(anyhow!("failed to get file stem"))?;

        let app_path = app_exe
            .as_os_str()
            .to_str()
            .ok_or(anyhow!("failed to get app_path"))?
            .to_string();

        // fix issue #26
        #[cfg(target_os = "windows")]
        let app_path = format!("\"{app_path}\"");

        // use the /Applications/Clash-Verge-Buty.app path
        #[cfg(target_os = "macos")]
        let app_path = (|| -> Option<String> {
            let path = std::path::PathBuf::from(&app_path);
            let path = path.parent()?.parent()?.parent()?;
            let extension = path.extension()?.to_str()?;
            match extension == "app" {
                true => Some(path.as_os_str().to_str()?.to_string()),
                false => None,
            }
        })()
        .unwrap_or(app_path);

        // fix #403
        #[cfg(target_os = "linux")]
        let app_path = {
            use crate::core::handle::Handle;
            use tauri::Manager;

            let handle = Handle::global();
            match handle.app_handle.lock().as_ref() {
                Some(app_handle) => {
                    let appimage = app_handle.env().appimage;
                    appimage
                        .and_then(|p| p.to_str().map(|s| s.to_string()))
                        .unwrap_or(app_path)
                }
                None => app_path,
            }
        };

        let auto = AutoLaunchBuilder::new()
            .set_app_name(app_name)
            .set_app_path(&app_path)
            .build()?;

        *self.auto_launch.lock() = Some(auto);

        Ok(())
    }

    /// update the startup
    pub fn update_launch(&self) -> Result<()> {
        let auto_launch = self.auto_launch.lock();

        if auto_launch.is_none() {
            drop(auto_launch);
            return self.init_launch();
        }
        let enable = { Config::verge().latest().enable_auto_launch };
        let enable = enable.unwrap_or(false);
        let auto_launch = auto_launch.as_ref().unwrap();

        match enable {
            true => auto_launch.enable()?,
            false => log_err!(auto_launch.disable()), // 忽略关闭的错误
        };

        Ok(())
    }

    /// launch a system proxy guard
    /// read config from file directly
    pub fn guard_proxy(&self) {
        use tokio::time::{sleep, Duration};

        let guard_state = self.guard_state.clone();

        tauri::async_runtime::spawn(async move {
            // if it is running, exit
            let mut state = guard_state.lock().await;
            if *state {
                return;
            }
            *state = true;
            drop(state);

            // default duration is 10s
            let mut wait_secs = 10u64;

            loop {
                sleep(Duration::from_secs(wait_secs)).await;

                let (enable, guard, guard_duration, bypass) = {
                    let verge = Config::verge();
                    let verge = verge.latest();
                    (
                        verge.enable_system_proxy.unwrap_or(false),
                        verge.enable_proxy_guard.unwrap_or(false),
                        verge.proxy_guard_duration.unwrap_or(10),
                        verge.system_proxy_bypass.clone(),
                    )
                };

                // stop loop
                if !enable || !guard {
                    break;
                }

                // update duration
                wait_secs = guard_duration;

                log::debug!(target: "app", "try to guard the system proxy");

                let port = {
                    Config::verge()
                        .latest()
                        .verge_mixed_port
                        .unwrap_or(Config::clash().data().get_mixed_port())
                };

                let sysproxy = Sysproxy {
                    enable: true,
                    host: "127.0.0.1".into(),
                    port,
                    bypass: bypass.unwrap_or(DEFAULT_BYPASS.into()),
                };

                if let Err(err) = wait_for_local_proxy(port)
                    .context("wait for local proxy before guarding system proxy")
                {
                    log::error!(target: "app", "{err}");
                    continue;
                }

                let _operation = Sysopt::global().proxy_operation.lock();
                let still_enabled = {
                    let verge = Config::verge();
                    let verge = verge.latest();
                    verge.enable_system_proxy.unwrap_or(false)
                        && verge.enable_proxy_guard.unwrap_or(false)
                };
                if !still_enabled {
                    continue;
                }

                let guard_result = sysproxy.set_system_proxy().context("guard system proxy");

                #[cfg(target_os = "windows")]
                if guard_result.is_ok() {
                    let mut owned = Sysopt::global().owned_windows_proxy.lock();
                    if let Some(snapshot) = owned.as_mut() {
                        if let Err(err) = snapshot.refresh_app_owned_values() {
                            log::warn!(target: "app", "could not refresh Windows proxy ownership snapshot after guard: {err}");
                        } else if let Err(err) = snapshot.record_owned_recovery(&sysproxy) {
                            log::warn!(target: "app", "could not refresh persistent Windows proxy ownership after guard: {err}");
                        }
                    }
                }
                log_err!(guard_result);
            }

            let mut state = guard_state.lock().await;
            *state = false;
            drop(state);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::is_sysproxy_match;
    use sysproxy::Sysproxy;

    fn proxy(port: u16, bypass: &str) -> Sysproxy {
        Sysproxy {
            enable: true,
            host: "127.0.0.1".into(),
            port,
            bypass: bypass.into(),
        }
    }

    #[test]
    fn proxy_ownership_requires_matching_endpoint_and_bypass() {
        let expected = proxy(7897, "localhost;<local>");
        assert!(is_sysproxy_match(&expected, &expected));
        assert!(!is_sysproxy_match(
            &proxy(7898, "localhost;<local>"),
            &expected
        ));
        assert!(!is_sysproxy_match(&proxy(7897, "<local>"), &expected));
    }
}
