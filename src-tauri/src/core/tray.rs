use crate::{
    cmds,
    config::Config,
    feat,
    utils::{dirs, resolve},
};
use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{
    api, AppHandle, CustomMenuItem, Manager, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem,
    SystemTraySubmenu,
};

static LAST_TRAY_MAIN_WINDOW_CLICK_MS: AtomicU64 = AtomicU64::new(0);
const TRAY_MAIN_WINDOW_DEBOUNCE_MS: u64 = 600;

pub struct Tray {}

impl Tray {
    pub fn tray_menu(app_handle: &AppHandle) -> SystemTrayMenu {
        let zh = { Config::verge().latest().language == Some("zh".into()) };

        let version = app_handle.package_info().version.to_string();

        macro_rules! t {
            ($en: expr, $zh: expr) => {
                if zh {
                    $zh
                } else {
                    $en
                }
            };
        }

        SystemTrayMenu::new()
            .add_item(CustomMenuItem::new(
                "open_window",
                t!("Dashboard", "打开面板"),
            ))
            .add_native_item(SystemTrayMenuItem::Separator)
            .add_item(CustomMenuItem::new(
                "rule_mode",
                t!("Rule Mode", "规则模式"),
            ))
            .add_item(CustomMenuItem::new(
                "global_mode",
                t!("Global Mode", "全局模式"),
            ))
            .add_item(CustomMenuItem::new(
                "direct_mode",
                t!("Direct Mode", "直连模式"),
            ))
            .add_native_item(SystemTrayMenuItem::Separator)
            .add_item(CustomMenuItem::new(
                "system_proxy",
                t!("System Proxy", "系统代理"),
            ))
            .add_item(CustomMenuItem::new("tun_mode", t!("TUN Mode", "网卡模式")))
            .add_item(CustomMenuItem::new(
                "copy_env",
                t!("Copy Env", "复制环境变量"),
            ))
            .add_submenu(SystemTraySubmenu::new(
                t!("Open Dir", "打开目录"),
                SystemTrayMenu::new()
                    .add_item(CustomMenuItem::new(
                        "open_app_dir",
                        t!("App Dir", "应用目录"),
                    ))
                    .add_item(CustomMenuItem::new(
                        "open_core_dir",
                        t!("Core Dir", "内核目录"),
                    ))
                    .add_item(CustomMenuItem::new(
                        "open_logs_dir",
                        t!("Logs Dir", "日志目录"),
                    )),
            ))
            .add_submenu(SystemTraySubmenu::new(
                t!("More", "更多"),
                SystemTrayMenu::new()
                    .add_item(CustomMenuItem::new(
                        "restart_clash",
                        t!("Restart Clash", "重启 Clash"),
                    ))
                    .add_item(CustomMenuItem::new(
                        "restart_app",
                        t!("Restart App", "重启应用"),
                    ))
                    .add_item(
                        CustomMenuItem::new("app_version", format!("Version {version}")).disabled(),
                    ),
            ))
            .add_native_item(SystemTrayMenuItem::Separator)
            .add_item(CustomMenuItem::new("quit", t!("Quit", "退出")).accelerator("CmdOrControl+Q"))
    }

    pub fn update_systray(app_handle: &AppHandle) -> Result<()> {
        app_handle
            .tray_handle()
            .set_menu(Tray::tray_menu(app_handle))?;
        Tray::update_part(app_handle)?;
        Ok(())
    }

    pub fn update_part(app_handle: &AppHandle) -> Result<()> {
        let zh = { Config::verge().latest().language == Some("zh".into()) };

        let version = app_handle.package_info().version.to_string();

        macro_rules! t {
            ($en: expr, $zh: expr) => {
                if zh {
                    $zh
                } else {
                    $en
                }
            };
        }

        let mode = {
            Config::clash()
                .latest()
                .0
                .get("mode")
                .map(|val| val.as_str().unwrap_or("rule"))
                .unwrap_or("rule")
                .to_owned()
        };

        let tray = app_handle.tray_handle();

        let _ = tray.get_item("rule_mode").set_selected(mode == "rule");
        let _ = tray.get_item("global_mode").set_selected(mode == "global");
        let _ = tray.get_item("direct_mode").set_selected(mode == "direct");

        let verge = Config::verge();
        let verge = verge.latest();
        let system_proxy = verge.enable_system_proxy.as_ref().unwrap_or(&false);
        let tun_mode = verge.enable_tun_mode.as_ref().unwrap_or(&false);
        let common_tray_icon = verge.common_tray_icon.as_ref().unwrap_or(&false);
        let sysproxy_tray_icon = verge.sysproxy_tray_icon.as_ref().unwrap_or(&false);
        let tun_tray_icon = verge.tun_tray_icon.as_ref().unwrap_or(&false);

        let mut indication_icon = if *system_proxy {
            #[cfg(not(target_os = "macos"))]
            let mut icon = include_bytes!("../../icons/tray-icon-sys.png").to_vec();
            #[cfg(target_os = "macos")]
            let mut icon = include_bytes!("../../icons/mac-tray-icon-sys.png").to_vec();
            if *sysproxy_tray_icon {
                let path = dirs::app_home_dir()?.join("icons").join("sysproxy.png");
                if path.exists() {
                    if let Ok(custom_icon) = std::fs::read(path) {
                        icon = custom_icon;
                    }
                }
            }
            icon
        } else {
            #[cfg(not(target_os = "macos"))]
            let mut icon = include_bytes!("../../icons/tray-icon.png").to_vec();
            #[cfg(target_os = "macos")]
            let mut icon = include_bytes!("../../icons/mac-tray-icon.png").to_vec();
            if *common_tray_icon {
                let path = dirs::app_home_dir()?.join("icons").join("common.png");
                if path.exists() {
                    if let Ok(custom_icon) = std::fs::read(path) {
                        icon = custom_icon;
                    }
                }
            }
            icon
        };

        if *tun_mode {
            #[cfg(not(target_os = "macos"))]
            let mut icon = include_bytes!("../../icons/tray-icon-tun.png").to_vec();
            #[cfg(target_os = "macos")]
            let mut icon = include_bytes!("../../icons/mac-tray-icon-tun.png").to_vec();
            if *tun_tray_icon {
                let path = dirs::app_home_dir()?.join("icons").join("tun.png");
                if path.exists() {
                    if let Ok(custom_icon) = std::fs::read(path) {
                        icon = custom_icon;
                    }
                }
            }
            indication_icon = icon
        }

        let _ = tray.set_icon(tauri::Icon::Raw(indication_icon));

        let _ = tray.get_item("system_proxy").set_selected(*system_proxy);
        let _ = tray.get_item("tun_mode").set_selected(*tun_mode);

        let switch_map = {
            let mut map = std::collections::HashMap::new();
            map.insert(true, "on");
            map.insert(false, "off");
            map
        };

        let _ = tray.set_tooltip(&format!(
            "Clash-Verge-Buty {version}\n{}: {}\n{}: {}",
            t!("System Proxy", "系统代理"),
            switch_map[system_proxy],
            t!("TUN Mode", "网卡模式"),
            switch_map[tun_mode]
        ));

        Ok(())
    }

    fn current_time_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default()
    }

    fn should_ignore_main_window_click() -> bool {
        let now = Tray::current_time_millis();
        let last = LAST_TRAY_MAIN_WINDOW_CLICK_MS.load(Ordering::SeqCst);

        if last > 0 && now.saturating_sub(last) < TRAY_MAIN_WINDOW_DEBOUNCE_MS {
            log::debug!(
                target: "app",
                "tray main_window click ignored by debounce, elapsed_ms={}, debounce_ms={}",
                now.saturating_sub(last),
                TRAY_MAIN_WINDOW_DEBOUNCE_MS
            );
            return true;
        }

        LAST_TRAY_MAIN_WINDOW_CLICK_MS.store(now, Ordering::SeqCst);
        false
    }

    fn open_or_focus_main_window(app_handle: &AppHandle) {
        if resolve::focus_main_window_if_open(app_handle, "tray main_window already open") {
            log::trace!(
                "tray main_window click used lightweight focus because main window is already open"
            );
            return;
        }

        if Tray::should_ignore_main_window_click() {
            return;
        }

        if resolve::is_main_window_creating() {
            log::warn!(target: "app", "tray main_window click recorded as pending because main window is creating");
        }

        log::trace!("tray main_window -> resolve::show_main_window");
        resolve::show_main_window_after_hide_transition(app_handle.clone());
    }

    pub fn on_left_click(app_handle: &AppHandle) {
        log::trace!("tray left click triggered, action=main_window");
        Tray::open_or_focus_main_window(app_handle);
    }

    pub fn on_system_tray_event(app_handle: &AppHandle, event: SystemTrayEvent) {
        match event {
            SystemTrayEvent::LeftClick { .. } => {
                log::trace!("tray event received: left click");
                Tray::on_left_click(app_handle);
            }
            SystemTrayEvent::MenuItemClick { id, .. } => {
                log::trace!("tray event received: menu item click, id={}", id.as_str());
                match id.as_str() {
                    mode @ ("rule_mode" | "global_mode" | "direct_mode") => {
                        let mode = &mode[0..mode.len() - 5];
                        feat::change_clash_mode(mode.into());
                    }
                    "open_window" => {
                        if resolve::is_main_window_creating() {
                            log::warn!(target: "app", "tray open_window menu recorded as pending because main window is creating");
                        }
                        log::trace!("tray open_window menu -> resolve::show_main_window");
                        resolve::show_main_window_after_hide_transition(app_handle.clone());
                    }
                    "system_proxy" => feat::toggle_system_proxy(),
                    "tun_mode" => feat::toggle_tun_mode(),
                    "copy_env" => feat::copy_clash_env(app_handle),
                    "open_app_dir" => crate::log_err!(cmds::open_app_dir()),
                    "open_core_dir" => crate::log_err!(cmds::open_core_dir()),
                    "open_logs_dir" => crate::log_err!(cmds::open_logs_dir()),
                    "restart_clash" => feat::restart_clash_core(),
                    "restart_app" => api::process::restart(&app_handle.env()),
                    "quit" => cmds::exit_app(app_handle.clone()),

                    _ => {}
                }
            }
            SystemTrayEvent::RightClick { .. } => {
                log::trace!("tray event received: right click");
            }
            SystemTrayEvent::DoubleClick { .. } => {
                log::trace!("tray event received: double click");
                Tray::open_or_focus_main_window(app_handle);
            }
            _ => {}
        }
    }
}
