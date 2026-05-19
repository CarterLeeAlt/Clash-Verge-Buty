use crate::{config::Config, log_err};
use anyhow::{anyhow, Context, Result};
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::{sync::Arc, thread, time::Duration};
use sysproxy::Sysproxy;
use tauri::{async_runtime::Mutex as TokioMutex, utils::platform::current_exe};

pub struct Sysopt {
    /// current system proxy setting
    cur_sysproxy: Arc<Mutex<Option<Sysproxy>>>,

    /// record the original system proxy
    /// recover it when exit
    old_sysproxy: Arc<Mutex<Option<Sysproxy>>>,

    /// helps to auto launch the app
    auto_launch: Arc<Mutex<Option<AutoLaunch>>>,

    /// record whether the guard async is running or not
    guard_state: Arc<TokioMutex<bool>>,
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

fn is_own_proxy(proxy: &Sysproxy, port: u16) -> bool {
    proxy.enable
        && proxy.port == port
        && (proxy.host == "127.0.0.1" || proxy.host.eq_ignore_ascii_case("localhost"))
}

fn set_system_proxy_with_retry(target: &Sysproxy, action: &str) -> Result<()> {
    const BACKOFF_MS: [u64; 3] = [150, 400, 800];
    let mut last_err = None;

    for (idx, backoff) in BACKOFF_MS.iter().enumerate() {
        match target.set_system_proxy() {
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
                last_err = Some(err.into());
            }
        }

        if idx + 1 < BACKOFF_MS.len() {
            thread::sleep(Duration::from_millis(*backoff));
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("{action} failed for unknown reason")))
        .with_context(|| format!("failed to {action} after {} attempts", BACKOFF_MS.len()))
}

impl Sysopt {
    pub fn global() -> &'static Sysopt {
        static SYSOPT: OnceCell<Sysopt> = OnceCell::new();

        SYSOPT.get_or_init(|| Sysopt {
            cur_sysproxy: Arc::new(Mutex::new(None)),
            old_sysproxy: Arc::new(Mutex::new(None)),
            auto_launch: Arc::new(Mutex::new(None)),
            guard_state: Arc::new(TokioMutex::new(false)),
        })
    }

    /// init the sysproxy
    pub fn init_sysproxy(&self) -> Result<()> {
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
            let old = Sysproxy::get_system_proxy().ok();
            let should_save_old = old
                .as_ref()
                .map(|proxy| !is_own_proxy(proxy, port))
                .unwrap_or(false);
            set_system_proxy_with_retry(&current, "enable system proxy")
                .context("init system proxy")?;

            if should_save_old {
                *self.old_sysproxy.lock() = old;
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

        let cached_old = self.old_sysproxy.lock().clone();

        if enable {
            let before =
                Sysproxy::get_system_proxy().context("read current system proxy before enable")?;
            let should_save_old = !is_own_proxy(&before, port);

            let set_result = set_system_proxy_with_retry(&target_proxy, "enable system proxy");

            if let Err(err) = set_result {
                if let Err(rollback_err) =
                    set_system_proxy_with_retry(&before, "restore system proxy after failed enable")
                {
                    log::error!(target: "app", "enable system proxy rollback failed: {rollback_err}");
                }
                return Err(err);
            }

            if should_save_old {
                *self.old_sysproxy.lock() = Some(before);
            }
            *self.cur_sysproxy.lock() = Some(target_proxy);
            return Ok(());
        }

        let before =
            Sysproxy::get_system_proxy().context("read current system proxy before disable")?;
        if !is_own_proxy(&before, port) {
            log::info!(target: "app", "disable system proxy skipped because current proxy is not owned by app");
            *self.cur_sysproxy.lock() = Some(target_proxy);
            return Ok(());
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
        )?;

        *self.cur_sysproxy.lock() = Some(target_proxy);
        Ok(())
    }

    /// reset the sysproxy
    pub fn reset_sysproxy(&self) -> Result<()> {
        let (cur_sysproxy, old_sysproxy) = {
            let mut cur_sysproxy = self.cur_sysproxy.lock();
            let mut old_sysproxy = self.old_sysproxy.lock();
            (cur_sysproxy.take(), old_sysproxy.take())
        };

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

                log_err!(sysproxy.set_system_proxy());
            }

            let mut state = guard_state.lock().await;
            *state = false;
            drop(state);
        });
    }
}
