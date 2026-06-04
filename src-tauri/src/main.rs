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

fn main() -> std::io::Result<()> {
    // 单例检测
    if server::check_singleton().is_err() {
        println!("app exists");
        return Ok(());
    }

    crate::log_err!(init::init_config());

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
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
            cmds::get_portable_flag,
            // cmds::kill_sidecar,
            cmds::restart_sidecar,
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
            // cmds::update_hotkeys,
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
            cmds::service::install_service,
            cmds::service::uninstall_service,
            // clash api
            cmds::clash_api_get_proxy_delay
        ]);

    #[cfg(target_os = "macos")]
    {
        use tauri::{Menu, MenuItem, Submenu};

        builder = builder.menu(
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
        );
    }

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
