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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
use tauri::api::notification;
use tauri::{App, AppHandle, Manager};
use window_shadows::set_shadow;

pub static VERSION: OnceCell<String> = OnceCell::new();

static MAIN_WINDOW_CREATING: AtomicBool = AtomicBool::new(false);
static FRONTEND_READY_LISTENING: AtomicBool = AtomicBool::new(false);
static FRONTEND_READY_HANDLED: AtomicBool = AtomicBool::new(false);
static FRONTEND_SHOW_WHEN_READY: AtomicBool = AtomicBool::new(true);
static MAIN_WINDOW_SHOW_REQUEST_ID: AtomicU64 = AtomicU64::new(0);
static MAIN_WINDOW_SHOWING: AtomicBool = AtomicBool::new(false);
static MAIN_WINDOW_SHOW_PENDING_ID: AtomicU64 = AtomicU64::new(0);
static IS_APP_QUITTING: AtomicBool = AtomicBool::new(false);

pub fn set_app_quitting(value: bool) {
    IS_APP_QUITTING.store(value, Ordering::SeqCst);
}

pub fn is_app_quitting() -> bool {
    IS_APP_QUITTING.load(Ordering::SeqCst)
}

#[derive(Clone, Copy, Debug)]
enum ShowReason {
    Explicit,
    FrontendReady,
    FrontendReadyRestore,
    CreateExisting,
    CreateFallback,
}

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

struct MainWindowShowingGuard;

impl MainWindowShowingGuard {
    fn try_lock(request_id: u64, reason: ShowReason) -> Option<Self> {
        if MAIN_WINDOW_SHOWING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log::trace!(
                "show skipped/merged because show already in progress, request_id={}, reason={:?}",
                request_id,
                reason
            );
            None
        } else {
            Some(Self)
        }
    }
}

impl Drop for MainWindowShowingGuard {
    fn drop(&mut self) {
        MAIN_WINDOW_SHOWING.store(false, Ordering::SeqCst);
        log::trace!("main window show lock released");
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
        let show_request_id = MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst);
        log::trace!(
            "frontend ready received, show_when_ready={}, show_request_id={}",
            show_when_ready,
            show_request_id
        );

        if show_when_ready || show_request_id > 0 {
            log::trace!(
                "frontend ready wants show, show_when_ready={}, show_request_id={}",
                show_when_ready,
                show_request_id
            );
            if let Some(window) = ready_app_handle.get_window("main") {
                show_existing_window(&window, show_request_id, ShowReason::FrontendReady);
            } else {
                log::trace!("frontend ready wants show but main window is missing");
                create_window(&ready_app_handle, true);
            }
            return;
        }

        if let Some(window) = ready_app_handle.get_window("main") {
            log::trace!(
                "frontend ready wants hide, show_request_id={}",
                show_request_id
            );
            let latest_show_request_id = MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst);
            if latest_show_request_id > 0 {
                log::trace!(
                    "frontend ready hide skipped because show requested, show_request_id={}",
                    latest_show_request_id
                );
                show_existing_window(
                    &window,
                    latest_show_request_id,
                    ShowReason::FrontendReadyRestore,
                );
                return;
            }

            trace_err!(window.hide(), "set win hidden after frontend ready");
            let latest_show_request_id = MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst);
            if latest_show_request_id > 0 {
                log::trace!(
                    "frontend ready post-hide show requested, restore main window, show_request_id={}",
                    latest_show_request_id
                );
                show_existing_window(
                    &window,
                    latest_show_request_id,
                    ShowReason::FrontendReadyRestore,
                );
            } else {
                log::trace!("frontend ready hide completed without show request");
            }
        } else {
            log::trace!("frontend ready wants hide but main window is missing while silent");
        }
    });
}

fn log_window_state(window: &tauri::Window, context: &str) {
    let visible = window
        .is_visible()
        .map(|value| value.to_string())
        .unwrap_or_else(|err| format!("err:{err}"));
    let minimized = window
        .is_minimized()
        .map(|value| value.to_string())
        .unwrap_or_else(|err| format!("err:{err}"));
    let focused = window
        .is_focused()
        .map(|value| value.to_string())
        .unwrap_or_else(|err| format!("err:{err}"));
    log::trace!(
        "{} window state: visible={}, minimized={}, focused={}",
        context,
        visible,
        minimized,
        focused
    );
}

fn ensure_main_window_onscreen(window: &tauri::Window, context: &str) {
    let position = match window.outer_position() {
        Ok(position) => position,
        Err(err) => {
            log::trace!(
                "main window offscreen check skipped, context={}, position error={}",
                context,
                err
            );
            return;
        }
    };
    let size = match window.outer_size() {
        Ok(size) => size,
        Err(err) => {
            log::trace!(
                "main window offscreen check skipped, context={}, size error={}",
                context,
                err
            );
            return;
        }
    };
    let monitors = match window.available_monitors() {
        Ok(monitors) => monitors,
        Err(err) => {
            log::trace!(
                "main window offscreen check skipped, context={}, monitors error={}",
                context,
                err
            );
            return;
        }
    };

    if monitors.is_empty() {
        log::trace!(
            "main window offscreen check skipped, context={}, no monitors available",
            context
        );
        return;
    }

    let window_left = position.x as i64;
    let window_top = position.y as i64;
    let window_right = window_left + size.width as i64;
    let window_bottom = window_top + size.height as i64;

    let intersects_monitor = monitors.iter().any(|monitor| {
        let monitor_position = monitor.position();
        let monitor_size = monitor.size();
        let monitor_left = monitor_position.x as i64;
        let monitor_top = monitor_position.y as i64;
        let monitor_right = monitor_left + monitor_size.width as i64;
        let monitor_bottom = monitor_top + monitor_size.height as i64;

        window_left < monitor_right
            && window_right > monitor_left
            && window_top < monitor_bottom
            && window_bottom > monitor_top
    });

    if !intersects_monitor {
        log::trace!(
            "main window appears offscreen, recentering, context={}, window=({}, {})-({}, {}), monitors={}",
            context,
            window_left,
            window_top,
            window_right,
            window_bottom,
            monitors.len()
        );
        trace_err!(window.center(), "recenter offscreen main window");
    }
}

#[cfg(target_os = "windows")]
fn force_activate_window(window: &tauri::Window, context: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, SetForegroundWindow, ShowWindowAsync, SW_RESTORE,
    };

    match window.hwnd() {
        Ok(hwnd) => {
            let hwnd = hwnd.0 as isize;
            log::trace!("windows force activate executing, context={}", context);
            unsafe {
                let show_ok = ShowWindowAsync(hwnd, SW_RESTORE) != 0;
                let bring_ok = BringWindowToTop(hwnd) != 0;
                let foreground_ok = SetForegroundWindow(hwnd) != 0;
                log::trace!(
                    "windows force activate finished, context={}, show_ok={}, bring_ok={}, foreground_ok={}",
                    context,
                    show_ok,
                    bring_ok,
                    foreground_ok
                );
                if !foreground_ok {
                    log::trace!(
                        "windows force activate SetForegroundWindow returned false, context={} (Windows foreground restrictions may apply)",
                        context
                    );
                }
            }
        }
        Err(err) => {
            log::trace!(
                "windows force activate skipped because hwnd unavailable, context={}, error={}",
                context,
                err
            );
        }
    }
}

fn window_state(window: &tauri::Window) -> (bool, bool, bool) {
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    (visible, minimized, focused)
}

#[cfg(target_os = "windows")]
fn window_needs_repair(window: &tauri::Window) -> bool {
    let (visible, minimized, focused) = window_state(window);
    !visible || minimized || !focused
}

#[cfg(not(target_os = "windows"))]
fn window_needs_repair(window: &tauri::Window) -> bool {
    let (visible, minimized, _) = window_state(window);
    !visible || minimized
}

fn record_pending_show_request(request_id: u64) {
    let mut pending = MAIN_WINDOW_SHOW_PENDING_ID.load(Ordering::SeqCst);
    while request_id > pending {
        match MAIN_WINDOW_SHOW_PENDING_ID.compare_exchange(
            pending,
            request_id,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return,
            Err(next_pending) => pending = next_pending,
        }
    }
}

fn window_show_once_fast(window: &tauri::Window, request_id: u64, reason: ShowReason) {
    log::trace!(
        "main window fast show attempt, request_id={}, reason={:?}",
        request_id,
        reason
    );
    trace_err!(window.unminimize(), "set win unminimize");
    trace_err!(window.show(), "set win visible");
    trace_err!(window.set_focus(), "set win focus");
    ensure_main_window_onscreen(window, "fast show after show");
}

fn window_show_once_repair(window: &tauri::Window, request_id: u64, context: &str) {
    log::trace!(
        "main window repair show attempt, request_id={}, context={}",
        request_id,
        context
    );
    ensure_main_window_onscreen(window, &format!("{} before show", context));
    trace_err!(window.unminimize(), "set win unminimize");
    trace_err!(window.show(), "set win visible");
    trace_err!(window.set_focus(), "set win focus");
    ensure_main_window_onscreen(window, &format!("{} after show", context));

    #[cfg(target_os = "windows")]
    force_activate_window(window, context);
}

fn finish_show_flow(
    window: &tauri::Window,
    current_request_id: u64,
    showing_guard: MainWindowShowingGuard,
) {
    drop(showing_guard);
    log_window_state(window, "show_existing_window released final");

    let pending_request_id = MAIN_WINDOW_SHOW_PENDING_ID.load(Ordering::SeqCst);
    if pending_request_id <= current_request_id {
        return;
    }

    log::trace!(
        "show flow processing merged pending request, current_request_id={}, pending_request_id={}",
        current_request_id,
        pending_request_id
    );
    request_show_existing_window(window, pending_request_id, ShowReason::Explicit);
}

#[cfg(target_os = "windows")]
fn schedule_windows_show_retries(
    window: tauri::Window,
    request_id: u64,
    reason: ShowReason,
    showing_guard: MainWindowShowingGuard,
) {
    log::trace!(
        "windows delayed retry scheduled, request_id={}, reason={:?}",
        request_id,
        reason
    );
    tauri::async_runtime::spawn(async move {
        let retries = [(1usize, 200u64), (2, 600)];
        for (attempt, delay_ms) in retries {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            let latest_request_id = MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst);
            if latest_request_id != request_id {
                log::trace!(
                    "windows delayed retry skipped because stale, request_id={}, latest_request_id={}, attempt={}, delay_ms={}",
                    request_id,
                    latest_request_id,
                    attempt,
                    delay_ms
                );
                break;
            }

            let (visible, minimized, focused) = window_state(&window);
            if !window_needs_repair(&window) {
                log::trace!(
                    "windows delayed retry skipped because window already visible, request_id={}, attempt={}, visible={}, minimized={}, focused={}",
                    request_id,
                    attempt,
                    visible,
                    minimized,
                    focused
                );
                log_window_state(&window, "windows delayed retry final healthy");
                break;
            }

            window_show_once_repair(
                &window,
                request_id,
                &format!("windows delayed {}ms", delay_ms),
            );

            if attempt == 2 {
                log_window_state(&window, "windows delayed retry final");
            }
        }

        finish_show_flow(&window, request_id, showing_guard);
    });
}

fn request_show_existing_window(window: &tauri::Window, request_id: u64, reason: ShowReason) {
    record_pending_show_request(request_id);

    let Some(showing_guard) = MainWindowShowingGuard::try_lock(request_id, reason) else {
        return;
    };

    let current_request_id = MAIN_WINDOW_SHOW_PENDING_ID.swap(0, Ordering::SeqCst);
    window_show_once_fast(window, current_request_id, reason);

    #[cfg(target_os = "windows")]
    schedule_windows_show_retries(window.clone(), current_request_id, reason, showing_guard);

    #[cfg(not(target_os = "windows"))]
    {
        log_window_state(window, "show_existing_window final");
        finish_show_flow(window, current_request_id, showing_guard);
    }
}

fn show_existing_window(window: &tauri::Window, request_id: u64, reason: ShowReason) {
    request_show_existing_window(window, request_id, reason);
}

/// show and focus the main window
pub fn show_main_window(app_handle: &AppHandle) {
    let show_request_id = MAIN_WINDOW_SHOW_REQUEST_ID.fetch_add(1, Ordering::SeqCst) + 1;
    FRONTEND_SHOW_WHEN_READY.store(true, Ordering::SeqCst);
    let existing = app_handle.get_window("main");
    log::trace!(
        "show_main_window called, existing window {}, show_request_id={}",
        if existing.is_some() { "yes" } else { "no" },
        show_request_id
    );

    if let Some(window) = existing {
        show_existing_window(&window, show_request_id, ShowReason::Explicit);
    } else {
        log::trace!("show_main_window did not find main window, create fallback");
        create_window(app_handle, true);
    }
}

fn schedule_create_window_show_fallback(
    app_handle: AppHandle,
    show_when_ready: bool,
    scheduled_show_request_id: u64,
) {
    log::trace!(
        "create_window show fallback scheduled, show_when_ready={}, show_request_id={}",
        show_when_ready,
        scheduled_show_request_id
    );
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let ready_handled = FRONTEND_READY_HANDLED.load(Ordering::SeqCst);
        let current_show_request_id = MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst);
        let should_show = show_when_ready || current_show_request_id > 0;

        if !should_show {
            log::trace!(
                "create_window show fallback skipped, silent without show request, ready_handled={}, show_request_id={}",
                ready_handled,
                current_show_request_id
            );
            return;
        }

        let Some(window) = app_handle.get_window("main") else {
            log::trace!(
                "create_window show fallback skipped, main window missing, ready_handled={}, show_request_id={}",
                ready_handled,
                current_show_request_id
            );
            return;
        };

        let visible = window.is_visible().unwrap_or(false);
        let minimized = window.is_minimized().unwrap_or(false);
        if !ready_handled
            || current_show_request_id > scheduled_show_request_id
            || !visible
            || minimized
        {
            log::trace!(
                "create_window show fallback triggered, ready_handled={}, visible={}, minimized={}, show_when_ready={}, scheduled_show_request_id={}, current_show_request_id={}",
                ready_handled,
                visible,
                minimized,
                show_when_ready,
                scheduled_show_request_id,
                current_show_request_id
            );
            show_existing_window(&window, current_show_request_id, ShowReason::CreateFallback);
        } else {
            log::trace!(
                "create_window show fallback skipped, ready already showed window, ready_handled={}, visible={}, minimized={}, show_request_id={}",
                ready_handled,
                visible,
                minimized,
                current_show_request_id
            );
        }
    });
}

/// create main window
pub fn create_window(app_handle: &AppHandle, show_when_ready: bool) {
    if show_when_ready {
        FRONTEND_SHOW_WHEN_READY.store(true, Ordering::SeqCst);
    }

    log::trace!(
        "create_window called, show_when_ready={}, existing={}, show_request_id={}",
        show_when_ready,
        app_handle.get_window("main").is_some(),
        MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
    );

    if let Some(window) = app_handle.get_window("main") {
        let show_request_id = MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst);
        if show_when_ready || show_request_id > 0 {
            log::trace!(
                "create_window found existing main window, show/focus it, show_request_id={}",
                show_request_id
            );
            show_existing_window(&window, show_request_id, ShowReason::CreateExisting);
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
    let build_start = Instant::now();
    log::info!(
        target: "app",
        "create_window before WindowBuilder::build, show_when_ready={}, show_request_id={}",
        show_when_ready,
        MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
    );

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

    log::info!(
        target: "app",
        "create_window after WindowBuilder::build, result={}, elapsed_ms={}, show_when_ready={}, show_request_id={}",
        if window.is_ok() { "ok" } else { "err" },
        build_start.elapsed().as_millis(),
        show_when_ready,
        MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
    );

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

            log::debug!(target: "app", "create_window before set_shadow");
            trace_err!(set_shadow(&win, true), "set win shadow");
            log::debug!(target: "app", "create_window after set_shadow");
            if is_maximized {
                trace_err!(win.maximize(), "set win maximize");
            }

            let show_request_id = MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst);
            if show_when_ready || show_request_id > 0 {
                schedule_create_window_show_fallback(
                    app_handle.clone(),
                    show_when_ready,
                    show_request_id,
                );
            } else {
                log::trace!(
                    "create_window show fallback skipped, show_when_ready=false and no show request"
                );
            }
        }
        Err(err) => {
            log::error!(
                target: "app",
                "failed to create window: {err}, show_when_ready={}, show_request_id={}",
                show_when_ready,
                MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
            );
            return;
        }
    }

    log::trace!(
        "create_window finished, show_when_ready={}, show_request_id={}",
        show_when_ready,
        MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
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
