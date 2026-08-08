use crate::config::IVerge;
use crate::{
    config::Config,
    core::*,
    utils::dirs,
    utils::init,
    utils::server,
};
use crate::{log_err, trace_err};
use anyhow::Result;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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

const MAIN_WINDOW_STATE_MISSING: u64 = 0;
const MAIN_WINDOW_STATE_CREATING: u64 = 1;
const MAIN_WINDOW_STATE_ALIVE: u64 = 2;
const MAIN_WINDOW_STATE_UNEXPECTED_DESTROYED: u64 = 3;
const MAIN_WINDOW_STATE_RECREATE_BACKOFF: u64 = 4;
const MAIN_WINDOW_RECREATE_BACKOFF_MS: u64 = 3_000;
const MAIN_WINDOW_DESTROY_WINDOW_MS: u64 = 60_000;
const MAIN_WINDOW_MAX_RECREATE_DESTROYS: u64 = 3;
const WINDOW_SAVE_DEBOUNCE_MS: u64 = 600;
const HEARTBEAT_WARN_AFTER_MS: u64 = 15_000;

static MAIN_WINDOW_STATE: AtomicU64 = AtomicU64::new(MAIN_WINDOW_STATE_MISSING);
static MAIN_WINDOW_UNEXPECTED_DESTROY_COUNT: AtomicU64 = AtomicU64::new(0);
static MAIN_WINDOW_UNEXPECTED_DESTROY_WINDOW_START_MS: AtomicU64 = AtomicU64::new(0);
static MAIN_WINDOW_LAST_UNEXPECTED_DESTROY_MS: AtomicU64 = AtomicU64::new(0);
static MAIN_WINDOW_LAST_CREATE_ATTEMPT_MS: AtomicU64 = AtomicU64::new(0);
static WINDOW_SAVE_SCHEDULED: AtomicBool = AtomicBool::new(false);
static LAST_WINDOW_MOVE_RESIZE_MS: AtomicU64 = AtomicU64::new(0);
static WINDOW_SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static WINDOW_HIDE_IN_PROGRESS_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
static LAST_FRONTEND_HEARTBEAT_MS: AtomicU64 = AtomicU64::new(0);

fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowBuildMode {
    NormalConfigured,
    ForcedReliableFallback,
}

impl WindowBuildMode {
    fn reason(self) -> &'static str {
        match self {
            Self::NormalConfigured => "configured",
            Self::ForcedReliableFallback => "unexpected_destroyed",
        }
    }
}

#[cfg(target_os = "windows")]
fn configured_reliable_mode() -> bool {
    !Config::verge()
        .latest()
        .enable_custom_frameless_window
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn configured_reliable_mode() -> bool {
    false
}

fn effective_reliable_mode(build_mode: WindowBuildMode) -> bool {
    match build_mode {
        WindowBuildMode::ForcedReliableFallback => true,
        WindowBuildMode::NormalConfigured => configured_reliable_mode(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowStyleConfig {
    pub platform: String,
    pub native_decorations: bool,
    pub reliable_mode: bool,
    pub custom_frameless: bool,
}

pub fn get_window_style_config() -> WindowStyleConfig {
    let now = current_time_millis();
    let build_mode = if should_force_reliable_fallback(now) {
        WindowBuildMode::ForcedReliableFallback
    } else {
        WindowBuildMode::NormalConfigured
    };
    let reliable_mode = effective_reliable_mode(build_mode);
    let custom_frameless = cfg!(target_os = "windows") && !reliable_mode;

    WindowStyleConfig {
        platform: std::env::consts::OS.to_string(),
        native_decorations: cfg!(target_os = "windows") && reliable_mode,
        reliable_mode,
        custom_frameless,
    }
}

fn main_window_state_name(state: u64) -> &'static str {
    match state {
        MAIN_WINDOW_STATE_MISSING => "Missing",
        MAIN_WINDOW_STATE_CREATING => "Creating",
        MAIN_WINDOW_STATE_ALIVE => "Alive",
        MAIN_WINDOW_STATE_UNEXPECTED_DESTROYED => "UnexpectedDestroyed",
        MAIN_WINDOW_STATE_RECREATE_BACKOFF => "RecreateBackoff",
        _ => "Unknown",
    }
}

fn unexpected_destroy_count_in_window(now: u64) -> u64 {
    let start = MAIN_WINDOW_UNEXPECTED_DESTROY_WINDOW_START_MS.load(Ordering::SeqCst);
    let count = MAIN_WINDOW_UNEXPECTED_DESTROY_COUNT.load(Ordering::SeqCst);
    if start > 0 && now.saturating_sub(start) <= MAIN_WINDOW_DESTROY_WINDOW_MS {
        count
    } else {
        0
    }
}

fn should_force_reliable_fallback(now: u64) -> bool {
    MAIN_WINDOW_STATE.load(Ordering::SeqCst) == MAIN_WINDOW_STATE_UNEXPECTED_DESTROYED
        || unexpected_destroy_count_in_window(now) >= 1
}

pub fn is_main_window_creating() -> bool {
    MAIN_WINDOW_CREATING.load(Ordering::SeqCst)
}

pub fn record_frontend_heartbeat() {
    LAST_FRONTEND_HEARTBEAT_MS.store(current_time_millis(), Ordering::SeqCst);
}

pub fn record_frontend_error(message: String, stack: Option<String>) {
    log::error!(
        target: "app",
        "frontend error reported: message={}, stack={}",
        message,
        stack.unwrap_or_default()
    );
}

pub fn check_main_window_health(app_handle: &AppHandle, context: &str) {
    let Some(window) = app_handle.get_window("main") else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let last = LAST_FRONTEND_HEARTBEAT_MS.load(Ordering::SeqCst);
    let now = current_time_millis();
    if last > 0 && now.saturating_sub(last) > HEARTBEAT_WARN_AFTER_MS {
        log::warn!(
            target: "app",
            "webview may be hung: no frontend heartbeat for {}ms, context={}",
            now.saturating_sub(last),
            context
        );
    }
}

pub fn on_main_window_destroyed(app_handle: &AppHandle) {
    let is_quitting = is_app_quitting();
    let now = current_time_millis();
    log::warn!(
        target: "app",
        "main window destroyed, is_quitting={}",
        is_quitting
    );

    if is_quitting {
        MAIN_WINDOW_STATE.store(MAIN_WINDOW_STATE_MISSING, Ordering::SeqCst);
        schedule_window_state_save(app_handle.clone(), Duration::from_millis(250));
        return;
    }

    let window_start = MAIN_WINDOW_UNEXPECTED_DESTROY_WINDOW_START_MS.load(Ordering::SeqCst);
    if window_start == 0 || now.saturating_sub(window_start) > MAIN_WINDOW_DESTROY_WINDOW_MS {
        MAIN_WINDOW_UNEXPECTED_DESTROY_WINDOW_START_MS.store(now, Ordering::SeqCst);
        MAIN_WINDOW_UNEXPECTED_DESTROY_COUNT.store(1, Ordering::SeqCst);
    } else {
        MAIN_WINDOW_UNEXPECTED_DESTROY_COUNT.fetch_add(1, Ordering::SeqCst);
    }
    let destroy_count = MAIN_WINDOW_UNEXPECTED_DESTROY_COUNT.load(Ordering::SeqCst);
    MAIN_WINDOW_LAST_UNEXPECTED_DESTROY_MS.store(now, Ordering::SeqCst);
    MAIN_WINDOW_STATE.store(MAIN_WINDOW_STATE_UNEXPECTED_DESTROYED, Ordering::SeqCst);
    MAIN_WINDOW_CREATING.store(false, Ordering::SeqCst);
    MAIN_WINDOW_SHOWING.store(false, Ordering::SeqCst);
    MAIN_WINDOW_SHOW_PENDING_ID.store(0, Ordering::SeqCst);
    FRONTEND_READY_HANDLED.store(false, Ordering::SeqCst);

    log::error!(
        target: "app",
        "main window unexpected destroyed: unexpected_destroyed_count={}, last_show_request_id={}, reliable_mode={}, state={}, action=wait_for_user_backoff",
        destroy_count,
        MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst),
        configured_reliable_mode(),
        main_window_state_name(MAIN_WINDOW_STATE.load(Ordering::SeqCst))
    );

    if destroy_count >= 2 {
        log::warn!(
            target: "app",
            "main window unexpected destroyed {} times within 60s; custom frameless mode will be disabled for fallback rebuilds",
            destroy_count
        );
    }
    if destroy_count >= MAIN_WINDOW_MAX_RECREATE_DESTROYS {
        MAIN_WINDOW_STATE.store(MAIN_WINDOW_STATE_RECREATE_BACKOFF, Ordering::SeqCst);
        log::error!(
            target: "app",
            "main window unexpected destroyed {} times within 60s; automatic rebuild stopped, please restart app or open logs directory",
            destroy_count
        );
    }
}

fn main_window_recreate_allowed(now: u64) -> bool {
    if MAIN_WINDOW_CREATING.load(Ordering::SeqCst) {
        log::warn!(target: "app", "main window show ignored because creation is already in progress; pending_show recorded");
        return false;
    }

    let state = MAIN_WINDOW_STATE.load(Ordering::SeqCst);
    if state == MAIN_WINDOW_STATE_RECREATE_BACKOFF
        || unexpected_destroy_count_in_window(now) >= MAIN_WINDOW_MAX_RECREATE_DESTROYS
    {
        log::error!(
            target: "app",
            "main window recreate blocked by backoff: state={}, unexpected_destroyed_count={}, last_show_request_id={}",
            main_window_state_name(state),
            unexpected_destroy_count_in_window(now),
            MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
        );
        return false;
    }

    let last_create = MAIN_WINDOW_LAST_CREATE_ATTEMPT_MS.load(Ordering::SeqCst);
    let last_destroy = MAIN_WINDOW_LAST_UNEXPECTED_DESTROY_MS.load(Ordering::SeqCst);
    if last_destroy > 0
        && last_create > 0
        && now.saturating_sub(last_create) < MAIN_WINDOW_RECREATE_BACKOFF_MS
    {
        log::warn!(
            target: "app",
            "main window recreate ignored by backoff, elapsed_since_create_ms={}, backoff_ms={}, last_destroy_ms={}",
            now.saturating_sub(last_create),
            MAIN_WINDOW_RECREATE_BACKOFF_MS,
            last_destroy
        );
        return false;
    }

    true
}

pub fn mark_window_hiding_for(duration: Duration) {
    let until = current_time_millis().saturating_add(duration.as_millis() as u64);
    WINDOW_HIDE_IN_PROGRESS_UNTIL_MS.store(until, Ordering::SeqCst);
}

pub fn is_window_hiding_in_progress() -> bool {
    let until = WINDOW_HIDE_IN_PROGRESS_UNTIL_MS.load(Ordering::SeqCst);
    until > 0 && current_time_millis() < until
}

pub fn schedule_window_state_save(app_handle: AppHandle, delay: Duration) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        match save_window_size_position(&app_handle, true) {
            Ok(_) => log::debug!(target: "app", "window state saved after delayed request"),
            Err(err) => log::trace!(
                target: "app",
                "window state delayed save skipped: {err}"
            ),
        }
    });
}

pub fn schedule_save_window_size_position(app_handle: AppHandle) {
    let now = current_time_millis();
    LAST_WINDOW_MOVE_RESIZE_MS.store(now, Ordering::SeqCst);
    WINDOW_SAVE_GENERATION.fetch_add(1, Ordering::SeqCst);

    if WINDOW_SAVE_SCHEDULED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    log::debug!(target: "app", "window moved/resized debounce started");
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(WINDOW_SAVE_DEBOUNCE_MS)).await;
            let now = current_time_millis();
            let last = LAST_WINDOW_MOVE_RESIZE_MS.load(Ordering::SeqCst);
            let elapsed = now.saturating_sub(last);
            if elapsed < WINDOW_SAVE_DEBOUNCE_MS {
                continue;
            }

            if is_window_hiding_in_progress() {
                log::debug!(
                    target: "app",
                    "window moved/resized save skipped, reason=hiding_or_not_visible"
                );
                WINDOW_SAVE_SCHEDULED.store(false, Ordering::SeqCst);
                return;
            }

            let Some(window) = app_handle.get_window("main") else {
                log::debug!(
                    target: "app",
                    "window moved/resized save skipped, reason=hiding_or_not_visible"
                );
                WINDOW_SAVE_SCHEDULED.store(false, Ordering::SeqCst);
                return;
            };

            if !window.is_visible().unwrap_or(false) {
                log::debug!(
                    target: "app",
                    "window moved/resized save skipped, reason=hiding_or_not_visible"
                );
                WINDOW_SAVE_SCHEDULED.store(false, Ordering::SeqCst);
                return;
            }

            match save_window_size_position(&app_handle, false) {
                Ok(_) => log::debug!(
                    target: "app",
                    "window moved/resized saved after quiet, elapsed_ms={}",
                    elapsed
                ),
                Err(err) => log::trace!(
                    target: "app",
                    "window moved/resized save after quiet skipped: {err}"
                ),
            }
            WINDOW_SAVE_SCHEDULED.store(false, Ordering::SeqCst);

            if WINDOW_SAVE_GENERATION.load(Ordering::SeqCst) != 0
                && current_time_millis()
                    .saturating_sub(LAST_WINDOW_MOVE_RESIZE_MS.load(Ordering::SeqCst))
                    < WINDOW_SAVE_DEBOUNCE_MS
            {
                schedule_save_window_size_position(app_handle.clone());
            }
            return;
        }
    });
}

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
            MAIN_WINDOW_STATE.store(MAIN_WINDOW_STATE_CREATING, Ordering::SeqCst);
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

    log_err!(sysopt::Sysopt::global().recover_pending_sysproxy());
    log_err!(init::init_resources());
    #[cfg(target_os = "windows")]
    log_err!(init::cleanup_legacy_scheme_registration());
    #[cfg(target_os = "windows")]
    log_err!(crate::core::win_firewall::reconcile_lan_firewall_on_startup());
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
    #[cfg(target_os = "windows")]
    log_err!(sysopt::Sysopt::global().update_launch());

    log_err!(handle::Handle::update_systray_part());
    log_err!(hotkey::Hotkey::global().init(app.app_handle()));
    log_err!(timer::Timer::global().init());
}

/// reset system proxy
pub fn resolve_reset() {
    log_err!(sysopt::Sysopt::global().reset_sysproxy());
    log_err!(CoreManager::global().stop_core());
    #[cfg(target_os = "windows")]
    log_err!(tauri::async_runtime::block_on(
        crate::core::win_service::stop_service_if_idle()
    ));
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
    if !focused {
        log::trace!("windows window focus is false but will not trigger repair by itself");
    }
    !visible || minimized
}

#[cfg(not(target_os = "windows"))]
fn window_needs_repair(window: &tauri::Window) -> bool {
    let (visible, minimized, _) = window_state(window);
    !visible || minimized
}

pub fn focus_main_window_if_open(app_handle: &AppHandle, context: &str) -> bool {
    let Some(window) = app_handle.get_window("main") else {
        return false;
    };

    let (visible, minimized, focused) = window_state(&window);
    if !visible || minimized {
        return false;
    }

    log::trace!(
        "main window lightweight focus requested, context={}, visible={}, minimized={}, focused={}",
        context,
        visible,
        minimized,
        focused
    );
    trace_err!(window.set_focus(), "set win focus lightweight");

    #[cfg(target_os = "windows")]
    force_activate_window(&window, context);

    true
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

    #[cfg(target_os = "windows")]
    force_activate_window(window, "fast show explicit");
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

pub fn show_main_window_after_hide_transition(app_handle: AppHandle) {
    if is_window_hiding_in_progress() {
        log::warn!(
            target: "app",
            "show_main_window delayed because window hide is in progress"
        );
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            show_main_window(&app_handle);
        });
        return;
    }

    show_main_window(&app_handle);
}

/// show and focus the main window
pub fn show_main_window(app_handle: &AppHandle) {
    check_main_window_health(app_handle, "show_main_window");
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
        record_pending_show_request(show_request_id);
        let now = current_time_millis();
        if !main_window_recreate_allowed(now) {
            return;
        }
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

#[cfg(target_os = "windows")]
fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn valid_window_size_position(size_pos: &[f64]) -> bool {
    size_pos.len() == 4
        && size_pos.iter().all(|value| value.is_finite())
        && size_pos[0] >= 600.0
        && size_pos[1] >= 520.0
        && size_pos[0] <= 20_000.0
        && size_pos[1] <= 20_000.0
        && size_pos[2].abs() <= 100_000.0
        && size_pos[3].abs() <= 100_000.0
}

#[cfg(target_os = "windows")]
fn windows_webview2_args(reliable_mode: bool, fallback_mode: bool) -> Option<String> {
    let mut args = Vec::new();

    if !reliable_mode {
        args.push("--enable-features=msWebView2EnableDraggableRegions".to_string());
    }

    if fallback_mode || env_flag_enabled("CLASH_VERGE_WEBVIEW2_DISABLE_GPU") {
        args.push("--disable-gpu".to_string());
    }

    if fallback_mode || env_flag_enabled("CLASH_VERGE_WEBVIEW2_DISABLE_HW_ACCELERATION") {
        args.push("--disable-software-rasterizer".to_string());
        args.push("--disable-accelerated-2d-canvas".to_string());
        args.push("--disable-accelerated-video-decode".to_string());
    }

    if let Ok(extra_args) = std::env::var("CLASH_VERGE_WEBVIEW2_ARGS") {
        args.extend(
            extra_args
                .split_whitespace()
                .filter(|arg| !arg.trim().is_empty())
                .map(|arg| arg.to_string()),
        );
    }

    if args.is_empty() {
        None
    } else {
        Some(args.join(" "))
    }
}

/// create main window
pub fn create_window(app_handle: &AppHandle, show_when_ready: bool) {
    if show_when_ready {
        FRONTEND_SHOW_WHEN_READY.store(true, Ordering::SeqCst);
    }

    let now = current_time_millis();
    if !main_window_recreate_allowed(now) {
        return;
    }
    let build_mode = if should_force_reliable_fallback(now) {
        WindowBuildMode::ForcedReliableFallback
    } else {
        WindowBuildMode::NormalConfigured
    };
    let reliable_mode = effective_reliable_mode(build_mode);
    MAIN_WINDOW_LAST_CREATE_ATTEMPT_MS.store(now, Ordering::SeqCst);

    log::trace!(
        "create_window called, show_when_ready={}, existing={}, show_request_id={}",
        show_when_ready,
        app_handle.get_window("main").is_some(),
        MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
    );
    if reliable_mode {
        log::info!(
            target: "app",
            "create_window using reliable mode, mode={:?}, reason={}, transparent=false, decorations=true, set_shadow skipped, draggable regions disabled",
            build_mode,
            build_mode.reason()
        );
    } else {
        log::warn!(
            target: "app",
            "create_window using experimental custom frameless mode, mode={:?}, reason={}, transparent=true, decorations=false, set_shadow enabled, draggable regions enabled",
            build_mode,
            build_mode.reason()
        );
    }

    if let Some(window) = app_handle.get_window("main") {
        MAIN_WINDOW_STATE.store(MAIN_WINDOW_STATE_ALIVE, Ordering::SeqCst);
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

    let webview_data_dir = match dirs::webview_data_dir() {
        Ok(path) => path,
        Err(err) => {
            log::error!(target: "app", "failed to resolve portable WebView data directory: {err}");
            return;
        }
    };
    if let Err(err) = std::fs::create_dir_all(&webview_data_dir) {
        log::error!(target: "app", "failed to create portable WebView data directory {}: {err}", webview_data_dir.display());
        return;
    }

    let window_size_locked = Config::verge().latest().window_size_locked.unwrap_or(false);
    let configured_size_pos = Config::verge().latest().window_size_position.clone();
    let size_pos_is_valid = configured_size_pos
        .as_ref()
        .map(|size_pos| valid_window_size_position(size_pos))
        .unwrap_or(false);
    if let Some(size_pos) = configured_size_pos.as_ref() {
        if !size_pos_is_valid {
            log::warn!(
                target: "app",
                "create_window ignoring abnormal window_size_position={:?}; safe centered defaults will be used and saved value will be cleared",
                size_pos
            );
            Config::verge().data().patch_config(IVerge {
                window_size_position: None,
                ..IVerge::default()
            });
            match Config::verge().data().save_file() {
                Ok(_) => log::info!(
                    target: "app",
                    "create_window cleared abnormal saved window_size_position"
                ),
                Err(err) => log::warn!(
                    target: "app",
                    "create_window failed to clear abnormal saved window_size_position: {err}"
                ),
            }
        } else {
            log::info!(
                target: "app",
                "create_window using saved window_size_position={:?}",
                size_pos
            );
        }
    } else {
        log::info!(target: "app", "create_window no saved window_size_position");
    }

    let build_window = |fallback_mode: bool, use_saved_size_pos: bool| {
        let mut builder = tauri::window::WindowBuilder::new(
            app_handle,
            "main".to_string(),
            tauri::WindowUrl::App("index.html".into()),
        )
        .title("Clash-Verge-Buty")
        .data_directory(webview_data_dir.clone())
        .visible(false)
        .fullscreen(false)
        .min_inner_size(600.0, 520.0)
        .resizable(!window_size_locked)
        .maximizable(!window_size_locked);

        if use_saved_size_pos {
            if let Some(size_pos) = configured_size_pos.as_ref() {
                let w = size_pos[0].clamp(600.0, 20_000.0);
                let h = size_pos[1].clamp(520.0, 20_000.0);
                builder = builder.inner_size(w, h).position(size_pos[2], size_pos[3]);
            }
        } else {
            #[cfg(target_os = "windows")]
            {
                builder = builder.inner_size(800.0, 636.0).center();
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

        #[cfg(target_os = "windows")]
        if let Some(args) = windows_webview2_args(reliable_mode, fallback_mode) {
            log::info!(
                target: "app",
                "create_window applying WebView2 additional_browser_args='{}', fallback_mode={}",
                args,
                fallback_mode
            );
            builder = builder.additional_browser_args(&args);
        }

        #[cfg(target_os = "windows")]
        let window = if reliable_mode || fallback_mode {
            builder
                .decorations(true)
                .transparent(false)
                .visible(false)
                .build()
        } else {
            builder
                .decorations(false)
                .transparent(true)
                .visible(false)
                .build()
        };
        #[cfg(target_os = "macos")]
        let window = builder
            .decorations(true)
            .hidden_title(true)
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .build();
        #[cfg(target_os = "linux")]
        let window = builder.decorations(true).transparent(false).build();

        window
    };

    let use_saved_size_pos = size_pos_is_valid;
    let build_start = Instant::now();
    log::info!(
        target: "app",
        "create_window primary build start before WindowBuilder::build, show_when_ready={}, show_request_id={}, reliable_mode={}, saved_size_pos={}, fallback_mode=false, window_size_locked={}",
        show_when_ready,
        MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst),
        reliable_mode,
        use_saved_size_pos,
        window_size_locked
    );
    let mut window = build_window(false, use_saved_size_pos);
    let mut used_window_fallback = false;

    log::info!(
        target: "app",
        "create_window primary build finished after WindowBuilder::build, result={}, elapsed_ms={}, show_when_ready={}, show_request_id={}, fallback_mode=false",
        if window.is_ok() { "ok" } else { "err" },
        build_start.elapsed().as_millis(),
        show_when_ready,
        MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
    );

    #[cfg(target_os = "windows")]
    if let Err(err) = &window {
        let retry_start = Instant::now();
        log::error!(
            target: "app",
            "create_window primary build Err: {err}; retrying with reliable decorated WebView2 fallback, disabled GPU/hardware acceleration, and centered safe size"
        );
        if configured_size_pos.is_some() && size_pos_is_valid {
            Config::verge().data().patch_config(IVerge {
                window_size_position: None,
                ..IVerge::default()
            });
            match Config::verge().data().save_file() {
                Ok(_) => log::info!(
                    target: "app",
                    "create_window cleared saved window_size_position before fallback retry"
                ),
                Err(err) => log::warn!(
                    target: "app",
                    "create_window failed to clear saved window_size_position before fallback retry: {err}"
                ),
            }
        }
        log::info!(
            target: "app",
            "create_window fallback build start before WindowBuilder::build, show_when_ready={}, show_request_id={}, reliable_mode=true, saved_size_pos=false, fallback_mode=true",
            show_when_ready,
            MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
        );
        window = build_window(true, false);
        used_window_fallback = window.is_ok();
        log::info!(
            target: "app",
            "create_window fallback build finished after WindowBuilder::build, result={}, elapsed_ms={}, show_when_ready={}, show_request_id={}, fallback_mode=true",
            if window.is_ok() { "ok" } else { "err" },
            retry_start.elapsed().as_millis(),
            show_when_ready,
            MAIN_WINDOW_SHOW_REQUEST_ID.load(Ordering::SeqCst)
        );
    }

    match window {
        Ok(win) => {
            MAIN_WINDOW_STATE.store(MAIN_WINDOW_STATE_ALIVE, Ordering::SeqCst);
            trace_err!(win.set_resizable(!window_size_locked), "set win resizable");
            trace_err!(
                win.set_maximizable(!window_size_locked),
                "set win maximizable"
            );
            let is_maximized = !window_size_locked
                && Config::verge()
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

            if reliable_mode || used_window_fallback {
                log::info!(
                    target: "app",
                    "create_window set_shadow skipped, reliable_mode={}, fallback_mode={}",
                    reliable_mode,
                    used_window_fallback
                );
            } else {
                log::debug!(target: "app", "create_window before set_shadow");
                trace_err!(set_shadow(&win, true), "set win shadow");
                log::debug!(target: "app", "create_window after set_shadow");
            }
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
            MAIN_WINDOW_STATE.store(MAIN_WINDOW_STATE_MISSING, Ordering::SeqCst);
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
    if save_to_file {
        verge.save_file()?;
    }
    Ok(())
}
