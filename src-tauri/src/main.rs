#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod cmds;
mod config;
mod core;
mod enhance;
mod feat;
mod utils;

use crate::utils::{init, resolve, server};
use std::time::Duration;
use tauri::{Manager, SystemTray};

#[cfg(target_os = "windows")]
const WATCHDOG_CHILD_ARG: &str = "--clash-verge-watchdog-child";

/// Keep a small supervisor process outside the Tauri/WebView process.  A panic
/// hook cannot catch Windows fail-fast exceptions such as 0xc0000409, so the
/// supervisor is the last line of defence for startup crashes.
#[cfg(target_os = "windows")]
fn run_watchdog() -> Option<std::io::Result<()>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args
        .iter()
        .any(|arg| arg == std::ffi::OsStr::new(WATCHDOG_CHILD_ARG))
    {
        return None;
    }

    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => return Some(Err(err)),
    };
    let child_args = args.into_iter().skip(1).chain(
        std::iter::once(std::ffi::OsString::from(WATCHDOG_CHILD_ARG)),
    );

    // A clean exit (status 0) means the user closed the app and must not be
    // restarted.  Non-zero exits are retried a few times to handle transient
    // WebView/Tauri startup failures without creating an endless crash loop.
    for attempt in 0..5 {
        let status = match std::process::Command::new(&exe)
            .args(child_args.clone())
            .spawn()
            .and_then(|mut child| child.wait())
        {
            Ok(status) => status,
            Err(err) => {
                if attempt == 4 {
                    return Some(Err(err));
                }
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        if status.success() || attempt == 4 {
            return Some(Ok(()));
        }

        log::error!(
            target: "app",
            "Clash-Verge child exited unexpectedly ({status}); retrying in 2s ({}/{})",
            attempt + 1,
            5
        );
        std::thread::sleep(Duration::from_secs(2));
    }

    Some(Ok(()))
}

fn main() -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    if let Some(result) = run_watchdog() {
        return result;
    }

    // 单例检测
    if server::check_singleton().is_err() {
        println!("app exists");
        return Ok(());
    }

    init::init_config().map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;

    let builder = tauri::Builder::default()
        .system_tray(SystemTray::new())
        .setup(|app| {
            resolve::resolve_setup(app);
            Ok(())
        })
        .on_system_tray_event(core::tray::Tray::on_system_tray_event)
        .invoke_handler(tauri::generate_handler![
            // common
            cmds::get_sys_proxy,
            cmds::open_app_dir,
            cmds::open_logs_dir,
            cmds::open_web_url,
            cmds::open_core_dir,
            cmds::restart_sidecar,
            cmds::upgrade_core,
            cmds::grant_permission,
            // clash
            cmds::get_clash_info,
            cmds::get_clash_logs,
            cmds::patch_clash_config,
            cmds::change_clash_core,
            cmds::get_runtime_config,
            cmds::get_runtime_yaml,
            cmds::get_runtime_exists,
            cmds::get_runtime_logs,
            cmds::uwp::invoke_uwp_tool,
            // verge
            cmds::get_verge_config,
            cmds::patch_verge_config,
            cmds::test_delay,
            cmds::get_app_dir,
            cmds::copy_icon_file,
            cmds::exit_app,
            cmds::frontend_heartbeat,
            cmds::report_frontend_error,
            cmds::get_window_style_config,
            cmds::set_window_size_locked,
            // profile
            cmds::get_profiles,
            cmds::enhance_profiles,
            cmds::patch_profiles_config,
            cmds::view_profile,
            cmds::patch_profile,
            cmds::create_profile,
            cmds::import_profile,
            cmds::reorder_profile,
            cmds::update_profile,
            cmds::delete_profile,
            cmds::read_profile_file,
            cmds::save_profile_file,
            // service mode
            cmds::service::check_service,
            cmds::service::is_elevated,
            cmds::service::install_service,
            cmds::service::uninstall_service,
            // clash api
            cmds::clash_api_get_proxy_delay
        ]);

    #[cfg(target_os = "macos")]
    let builder = {
        use tauri::{Menu, MenuItem, Submenu};

        builder.menu(
            Menu::new().add_submenu(Submenu::new(
                "Edit",
                Menu::new()
                    .add_native_item(MenuItem::Undo)
                    .add_native_item(MenuItem::Redo)
                    .add_native_item(MenuItem::Copy)
                    .add_native_item(MenuItem::Paste)
                    .add_native_item(MenuItem::Cut)
                    .add_native_item(MenuItem::SelectAll)
                    .add_native_item(MenuItem::CloseWindow)
                    .add_native_item(MenuItem::Quit),
            )),
        )
    };

    let app = builder
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, e| match e {
        tauri::RunEvent::ExitRequested { api, .. } => {
            if !resolve::is_app_quitting() {
                log::info!(target: "app", "app exit requested -> prevent_exit because app is not quitting");
                api.prevent_exit();
            } else {
                log::info!(target: "app", "app exit requested -> allow because app is quitting");
            }
        }
        tauri::RunEvent::WindowEvent { label, event, .. } => {
            if label == "main" {
                match event {
                    tauri::WindowEvent::Destroyed => {
                        resolve::on_main_window_destroyed(app_handle);
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        let is_quitting = resolve::is_app_quitting();
                        log::info!(
                            target: "app",
                            "main window close requested, is_quitting={}",
                            is_quitting
                        );

                        if !is_quitting {
                            api.prevent_close();
                            resolve::mark_window_hiding_for(Duration::from_millis(1500));

                            if let Some(window) = app_handle.get_window("main") {
                                match window.hide() {
                                    Ok(_) => log::info!(
                                        target: "app",
                                        "main window close requested -> prevent_close and hide immediately"
                                    ),
                                    Err(err) => log::error!(
                                        target: "app",
                                        "main window hide failed in CloseRequested: {err}"
                                    ),
                                }
                            } else {
                                log::warn!(
                                    target: "app",
                                    "main window close requested but main window not found"
                                );
                            }

                            resolve::schedule_window_state_save(
                                app_handle.clone(),
                                Duration::from_millis(1000),
                            );
                            return;
                        }

                        log::info!(
                            target: "app",
                            "main window close allowed because app is quitting"
                        );
                    }
                    tauri::WindowEvent::Focused(focused) => {
                        log::debug!(target: "app", "main window focused={focused}");
                    }
                    tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                        resolve::schedule_save_window_size_position(app_handle.clone());
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    });

    Ok(())
}
