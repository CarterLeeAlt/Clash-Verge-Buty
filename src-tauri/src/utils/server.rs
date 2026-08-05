extern crate warp;

use super::resolve;
use crate::config::IVerge;
use anyhow::{bail, Result};
use port_scanner::local_port_available;
use tauri::AppHandle;
use warp::Filter;

/// check whether there is already exists
pub fn check_singleton() -> Result<()> {
    let port = IVerge::get_singleton_port();

    if !local_port_available(port) {
        tauri::async_runtime::block_on(async {
            let resp = reqwest::get(format!("http://127.0.0.1:{port}/commands/ping"))
                .await?
                .text()
                .await?;

            if &resp == "ok" {
                reqwest::get(format!("http://127.0.0.1:{port}/commands/visible"))
                    .await?
                    .text()
                    .await?;
                bail!("app exists");
            }

            log::error!("failed to setup singleton listen server");
            Ok(())
        })
    } else {
        Ok(())
    }
}

/// The embed server only be used to implement singleton process
/// maybe it can be used as pac server later
pub fn embed_server(app_handle: AppHandle) {
    let port = IVerge::get_singleton_port();

    tauri::async_runtime::spawn(async move {
        let ping = warp::path!("commands" / "ping").map(move || "ok");

        let visible = warp::path!("commands" / "visible").map(move || {
            log::trace!("singleton visible command received -> resolve::show_main_window");
            resolve::show_main_window(&app_handle);
            "ok"
        });

        let commands = ping.or(visible);
        warp::serve(commands).run(([127, 0, 0, 1], port)).await;
    });
}
