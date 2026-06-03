use crate::config::{IVerge, PrfOption};
use crate::{
    config::{Config, PrfItem},
    core::*,
    utils::init,
    utils::server,
};
use crate::{log_err, trace_err};
use anyhow::Result;
use once_cell::sync::OnceCell;
use serde_yaml::Mapping;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::api::notification;
use tauri::{App, AppHandle, Manager};
use window_shadows::set_shadow;

pub static VERSION: OnceCell<String> = OnceCell::new();

static MAIN_WINDOW_CREATING: AtomicBool = AtomicBool::new(false);
static FRONTEND_READY_LISTENING: AtomicBool = AtomicBool::new(false);
static FRONTEND_READY_HANDLED: AtomicBool = AtomicBool::new(false);
static FRONTEND_SHOW_WHEN_READY: AtomicBool = AtomicBool::new(true);

struct MainWindowCreatingGuard;

impl MainWindowCreatingGuard {
    fn try_lock() -> Option<Self> {
        if MAIN_WINDOW_CREATING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log::trace!("create_window skipped because main window is already creating");
            None
        } else {
            Some(Self)
        }
    }
}

impl Drop for MainWindowCreatingGuard {
    fn drop(&mut self) {
        MAIN_WINDOW_CREATING.store(false, Ordering::SeqCst);
        log::trace!("create_window creating lock released");
    }
}

pub fn find_unused_port() -> Result<u16> {
    match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => {
            let port = listener.local_addr()?.port();
            Ok(port)
        }
        Err(_) => {
            let port = Config::verge()
                .latest()
                .verge_mixed_port
                .unwrap_or(Config::clash().data().get_mixed_port());
            log::warn!(target: "app", "use default port: {}", port);
            Ok(port)
        }
    }
}

/// handle something when start app
pub fn resolve_setup(app: &mut App) {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    let version = app.package_info().version.to_string();
    handle::Handle::global().init(app.app_handle());
    VERSION.get_or_init(|| version.clone());

    log_err!(init::init_resources());
    log_err!(init::init_scheme());
    log_err!(init::startup_script());
    // 处理随机端口
    let enable_random_port = Config::verge().latest().enable_random_port.unwrap_or(false);

    let mut port = Config::verge()
        .latest()
        .verge_mixed_port
        .unwrap_or(Config::clash().data().get_mixed_port());

    if enable_random_port {
        port = find_unused_port().unwrap_or(
            Config::verge()
                .latest()
                .verge_mixed_port
                .unwrap_or(Config::clash().data().get_mixed_port()),
        );
    }

    Config::verge().data().patch_config(IVerge {
        verge_mixed_port: Some(port),
        ..IVerge::default()
    });
    let _ = Config::verge().data().save_file();
    let mut mapping = Mapping::new();
    mapping.insert("mixed-port".into(), port.into());
    Config::clash().data().patch_config(mapping);
    let _ = Config::clash().data().save_config();

    // 启动核心
    log::trace!("init config");
    log_err!(Config::init_config());

    log::trace!("launch core");
    log_err!(CoreManager::global().init());

    // setup a simple http server for singleton
    log::trace!("launch embed server");
    server::embed_server(app.app_handle());

    log::trace!("init system tray");
    log_err!(tray::Tray::update_systray(&app.app_handle()));

    let silent_start = { Config::verge().data().enable_silent_start.unwrap_or(false) };
    let show_when_ready = !silent_start;
    log::trace!(
        "resolve_setup pre-create main window, silent_start={}, show_when_ready={}",
        silent_start,
        show_when_ready
    );
    register_frontend_ready_listener(&app.app_handle(), show_when_ready);
    create_window(&app.app_handle(), show_when_ready);

    log_err!(sysopt::Sysopt::global().init_launch());
    log_err!(sysopt::Sysopt::global().init_sysproxy());

    log_err!(handle::Handle::update_systray_part());
    log_err!(hotkey::Hotkey::global().init(app.app_handle()));
    log_err!(timer::Timer::global().init());

    let argvs: Vec<String> = std::env::args().collect();
    if argvs.len() > 1 {
        tauri::async_runtime::block_on(async {
            resolve_scheme(argvs[1].to_owned()).await;
        });
    }
}

/// reset system proxy
pub fn resolve_reset() {
    log_err!(sysopt::Sysopt::global().reset_sysproxy());
    log_err!(CoreManager::global().stop_core());
}

fn register_frontend_ready_listener(app_handle: &AppHandle, show_when_ready: bool) {
    FRONTEND_SHOW_WHEN_READY.store(show_when_ready, Ordering::SeqCst);

    if FRONTEND_READY_LISTENING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        log::trace!(
            "frontend ready listener already registered, show_when_ready={}",
            show_when_ready
        );
        return;
    }

    let listener_app_handle = app_handle.clone();
    let ready_app_handle = app_handle.clone();
    log::trace!(
        "register frontend ready listener, show_when_ready={}",
        show_when_ready
    );
    listener_app_handle.listen_global("frontend://ready", move |_| {
        if FRONTEND_READY_HANDLED
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log::trace!("frontend ready already handled, skip");
            return;
        }

        let show_when_ready = FRONTEND_SHOW_WHEN_READY.load(Ordering::SeqCst);
        log::trace!(
            "frontend ready received, show_when_ready={}",
            show_when_ready
        );
        if show_when_ready {
            log::trace!("frontend ready -> show main window");
            show_main_window(&ready_app_handle);
        } else if let Some(window) = ready_app_handle.get_window("main") {
            log::trace!("frontend ready -> keep main window hidden");
            trace_err!(window.hide(), "set win hidden after frontend ready");
        } else {
            log::trace!("frontend ready -> main window missing while silent");
        }
    });
}

fn focus_window_with_retry(window: &tauri::Window) {
    trace_err!(window.set_focus(), "set win focus");

    #[cfg(target_os = "windows")]
    {
        let win = window.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            crate::trace_err!(win.set_focus(), "set win focus delayed");
        });
    }
}

fn show_existing_window(window: &tauri::Window) {
    trace_err!(window.unminimize(), "set win unminimize");
    trace_err!(window.show(), "set win visible");
    focus_window_with_retry(window);
}

/// show and focus the main window
pub fn show_main_window(app_handle: &AppHandle) {
    FRONTEND_SHOW_WHEN_READY.store(true, Ordering::SeqCst);
    if let Some(window) = app_handle.get_window("main") {
        log::trace!("show_main_window found existing main window");
        show_existing_window(&window);
    } else {
        log::trace!("show_main_window did not find main window, create fallback");
        create_window(app_handle, true);
    }
}

/// create main window
pub fn create_window(app_handle: &AppHandle, show_when_ready: bool) {
    if show_when_ready {
        FRONTEND_SHOW_WHEN_READY.store(true, Ordering::SeqCst);
    }

    log::trace!(
        "create_window called, show_when_ready={}, existing={}",
        show_when_ready,
        app_handle.get_window("main").is_some()
    );

    if let Some(window) = app_handle.get_window("main") {
        if show_when_ready {
            log::trace!("create_window found existing main window, show/focus it");
            show_existing_window(&window);
        } else {
            log::trace!("create_window found existing main window, leave visibility unchanged");
        }
        return;
    }

    let Some(_creating_guard) = MainWindowCreatingGuard::try_lock() else {
        return;
    };
    FRONTEND_READY_HANDLED.store(false, Ordering::SeqCst);
    log::trace!("create_window reset frontend ready handled for new main window");

    let mut builder = tauri::window::WindowBuilder::new(
        app_handle,
        "main".to_string(),
        tauri::WindowUrl::App("index.html".into()),
    )
    .title("Clash-Verge-Buty")
    .visible(false)
    .fullscreen(false)
    .min_inner_size(600.0, 520.0);

    match Config::verge().latest().window_size_position.clone() {
        Some(size_pos) if size_pos.len() == 4 => {
            let size = (size_pos[0], size_pos[1]);
            let pos = (size_pos[2], size_pos[3]);
            let w = size.0.clamp(600.0, f64::INFINITY);
            let h = size.1.clamp(520.0, f64::INFINITY);
            builder = builder.inner_size(w, h).position(pos.0, pos.1);
        }
        _ => {
            #[cfg(target_os = "windows")]
            {
                builder = builder
                    .additional_browser_args("--enable-features=msWebView2EnableDraggableRegions")
                    .inner_size(800.0, 636.0)
                    .center();
            }

            #[cfg(target_os = "macos")]
            {
                builder = builder.inner_size(800.0, 642.0).center();
            }

            #[cfg(target_os = "linux")]
            {
                builder = builder.inner_size(800.0, 642.0).center();
            }
        }
    };
    #[cfg(target_os = "windows")]
    let window = builder
        .decorations(false)
        .transparent(true)
        .visible(false)
        .build();
    #[cfg(target_os = "macos")]
    let window = builder
        .decorations(true)
        .hidden_title(true)
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .build();
    #[cfg(target_os = "linux")]
    let window = builder.decorations(true).transparent(false).build();

    match window {
        Ok(win) => {
            let is_maximized = Config::verge()
                .latest()
                .window_is_maximized
                .unwrap_or(false);
            log::trace!("try to calculate the monitor size");
            let center = (|| -> Result<bool> {
                let mut center = false;
                let monitor = win.current_monitor()?.ok_or(anyhow::anyhow!(""))?;
                let size = monitor.size();
                let pos = win.outer_position()?;

                if pos.x < -400
                    || pos.x > (size.width - 200) as i32
                    || pos.y < -200
                    || pos.y > (size.height - 200) as i32
                {
                    center = true;
                }
                Ok(center)
            })();

            if center.unwrap_or(true) {
                trace_err!(win.center(), "set win center");
            }

            trace_err!(set_shadow(&win, true), "set win shadow");
            if is_maximized {
                trace_err!(win.maximize(), "set win maximize");
            }
        }
        Err(err) => {
            log::error!("failed to create window: {err}");
            return;
        }
    }

    log::trace!(
        "create_window finished, show_when_ready={}",
        show_when_ready
    );
}

/// save window size and position
pub fn save_window_size_position(app_handle: &AppHandle, save_to_file: bool) -> Result<()> {
    let verge = Config::verge();
    let mut verge = verge.latest();

    if save_to_file {
        verge.save_file()?;
    }

    let win = app_handle
        .get_window("main")
        .ok_or(anyhow::anyhow!("failed to get window"))?;

    let scale = win.scale_factor()?;
    let size = win.inner_size()?;
    let size = size.to_logical::<f64>(scale);
    let pos = win.outer_position()?;
    let pos = pos.to_logical::<f64>(scale);
    let is_maximized = win.is_maximized()?;
    verge.window_is_maximized = Some(is_maximized);
    if !is_maximized && size.width >= 600.0 && size.height >= 520.0 {
        verge.window_size_position = Some(vec![size.width, size.height, pos.x, pos.y]);
    }
    Ok(())
}

pub async fn resolve_scheme(param: String) {
    let url = param
        .trim_start_matches("clash://install-config/?url=")
        .trim_start_matches("clash://install-config?url=");
    let option = PrfOption {
        user_agent: None,
        with_proxy: Some(true),
        self_proxy: None,
        danger_accept_invalid_certs: None,
        update_interval: None,
    };
    if let Ok(item) = PrfItem::from_url(url, None, None, Some(option)).await {
        if Config::profiles().data().append_item(item).is_ok() {
            notification::Notification::new(crate::utils::dirs::APP_ID)
                .title("Clash-Verge-Buty")
                .body("Import profile success")
                .show()
                .map_err(|err| log::warn!("failed to show import success notification: {err}"))
                .ok();
        };
    } else {
        notification::Notification::new(crate::utils::dirs::APP_ID)
            .title("Clash-Verge-Buty")
            .body("Import profile failed")
            .show()
            .map_err(|err| log::warn!("failed to show import failed notification: {err}"))
            .ok();
        log::error!("failed to parse url: {}", url);
    }
}
