#![cfg(target_os = "windows")]

use crate::config::Config;
use crate::utils::dirs;
use anyhow::{bail, Context, Result};
use deelevate::{PrivilegeLevel, Token};
use runas::Command as RunasCommand;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;
use std::{env::current_exe, process::Command as StdCommand};
use tokio::time::sleep;

const SERVICE_URL: &str = "http://127.0.0.1:33211";
const SERVICE_NAME: &str = "clash-verge-service";
const LEGACY_SERVICE_NAME: &str = "clash_verge_service";
const SERVICE_BINARY: &str = "clash-verge-service.exe";
const INSTALL_HELPER: &str = "install-service.exe";
const UNINSTALL_HELPER: &str = "uninstall-service.exe";

#[derive(Debug)]
struct ScResult { code: i32, stdout: String, stderr: String }

fn sc(args: &[&str]) -> Result<ScResult> {
    let output = StdCommand::new("sc.exe").args(args).creation_flags(0x08000000).output()?;
    Ok(ScResult { code: output.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&output.stdout).into_owned(), stderr: String::from_utf8_lossy(&output.stderr).into_owned() })
}

fn service_exists(name: &str) -> bool { sc(&["query", name]).map(|r| r.code == 0).unwrap_or(false) }

async fn migrate_legacy_service_if_needed() -> Result<()> {
    if service_exists(SERVICE_NAME) || !service_exists(LEGACY_SERVICE_NAME) { return Ok(()); }
    let stop = sc(&["stop", LEGACY_SERVICE_NAME])?; log::info!(target:"app", "legacy stop {} => {} | {}", LEGACY_SERVICE_NAME, stop.code, stop.stdout);
    let del = sc(&["delete", LEGACY_SERVICE_NAME])?; if del.code != 0 { bail!("legacy migration failed while deleting service. expected service name: {SERVICE_NAME}; legacy service name checked: {LEGACY_SERVICE_NAME}; selected service name: {LEGACY_SERVICE_NAME}; service binary: {SERVICE_BINARY}; install helper: {INSTALL_HELPER}; uninstall helper: {UNINSTALL_HELPER}; sc.exe delete exit code: {}; stdout: {}; stderr: {}", del.code, del.stdout, del.stderr); }
    install_service().await?;
    Ok(())
}

fn start_service_process() -> Result<()> {
    let selected = if service_exists(SERVICE_NAME) { SERVICE_NAME } else if service_exists(LEGACY_SERVICE_NAME) { LEGACY_SERVICE_NAME } else { SERVICE_NAME };
    let start = sc(&["start", selected])?;
    if start.code == 0 { return Ok(()); }
    bail!("failed to start service process. expected service name: {SERVICE_NAME}; legacy service name checked: {LEGACY_SERVICE_NAME}; selected service name: {selected}; service binary: {SERVICE_BINARY}; install helper: {INSTALL_HELPER}; uninstall helper: {UNINSTALL_HELPER}; sc.exe start exit code: {}; stdout: {}; stderr: {}", start.code, start.stdout, start.stderr)
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceStateHint { Running, StartPending, Other }

fn query_service_state(name: &str) -> Result<ServiceStateHint> {
    let r = sc(&["query", name])?;
    if r.code != 0 { return Ok(ServiceStateHint::Other); }
    let out = r.stdout.to_ascii_uppercase();
    if out.contains("RUNNING") { Ok(ServiceStateHint::Running) }
    else if out.contains("START_PENDING") { Ok(ServiceStateHint::StartPending) }
    else { Ok(ServiceStateHint::Other) }
}

fn selected_service_name() -> &'static str {
    if service_exists(SERVICE_NAME) { SERVICE_NAME }
    else if service_exists(LEGACY_SERVICE_NAME) { LEGACY_SERVICE_NAME }
    else { SERVICE_NAME }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ResponseBody { pub core_type: Option<String>, pub bin_path: String, pub config_dir: String, pub log_file: String }

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct JsonResponse { pub code: u64, pub msg: String, pub data: Option<ResponseBody> }

pub async fn install_service() -> Result<()> {
    let binary_path = dirs::service_path()?;
    let install_path = binary_path.with_file_name(INSTALL_HELPER);
    if !install_path.exists() { bail!("installer exe not found: {}", INSTALL_HELPER); }
    let token = Token::with_current_process()?; let level = token.privilege_level()?;
    let status = match level { PrivilegeLevel::NotPrivileged => RunasCommand::new(install_path).show(false).status()?, _ => StdCommand::new(install_path).creation_flags(0x08000000).status()?, };
    if !status.success() { bail!("failed to install service with status {}", status.code().unwrap_or(-1)); }
    Ok(())
}

pub async fn uninstall_service() -> Result<()> {
    let binary_path = dirs::service_path()?;
    let uninstall_path = binary_path.with_file_name(UNINSTALL_HELPER);
    if !uninstall_path.exists() { bail!("uninstaller exe not found: {}", UNINSTALL_HELPER); }
    let token = Token::with_current_process()?; let level = token.privilege_level()?;
    let status = match level { PrivilegeLevel::NotPrivileged => RunasCommand::new(uninstall_path).show(false).status()?, _ => StdCommand::new(uninstall_path).creation_flags(0x08000000).status()?, };
    if !status.success() { bail!("failed to uninstall service with status {}", status.code().unwrap_or(-1)); }
    Ok(())
}

pub async fn check_service() -> Result<JsonResponse> {
    let response = reqwest::ClientBuilder::new().no_proxy().build()?.get(format!("{SERVICE_URL}/get_clash")).send().await;
    match response {
        Ok(resp) => Ok(resp.json::<JsonResponse>().await.context("failed to parse the clash-verge-service response")?),
        Err(err) => {
            if service_exists(SERVICE_NAME) || service_exists(LEGACY_SERVICE_NAME) { Ok(JsonResponse { code: 400, msg: "service installed but not active".into(), data: None }) }
            else { bail!("failed to connect to service. expected service name: {SERVICE_NAME}; legacy service name checked: {LEGACY_SERVICE_NAME}; selected service name: {SERVICE_NAME}; service binary: {SERVICE_BINARY}; install helper: {INSTALL_HELPER}; uninstall helper: {UNINSTALL_HELPER}; api ready result: false; error: {err}") }
        }
    }
}

pub async fn ensure_service_ready() -> Result<()> {
    migrate_legacy_service_if_needed().await?;
    if let Ok(status) = check_service().await {
        if status.code == 0 {
            return Ok(());
        }
    }

    start_service_process()?;
    let selected = selected_service_name();
    let timeout = Duration::from_secs(15);
    let started = std::time::Instant::now();

    loop {
        if let Ok(status) = check_service().await {
            if status.code == 0 {
                return Ok(());
            }
        }

        let state = query_service_state(selected).unwrap_or(ServiceStateHint::Other);
        if started.elapsed() >= timeout {
            if state == ServiceStateHint::StartPending {
                bail!("Windows service is stuck in StartPending and API 127.0.0.1:33211 is not ready.");
            }
            bail!("service API 127.0.0.1:33211 is not ready after start timeout; current service state: {:?}", state);
        }

        sleep(Duration::from_millis(500)).await;
    }
}

pub(super) async fn run_core_by_service(config_file: &PathBuf) -> Result<()> { ensure_service_ready().await?; let status = check_service().await?; if status.code == 0 { stop_core_by_service().await?; sleep(Duration::from_secs(1)).await; }
let clash_core = Config::verge().latest().clash_core.clone().unwrap_or("clash".into()); let clash_bin = format!("{clash_core}.exe"); let bin_path_buf = current_exe()?.with_file_name(clash_bin); let config_dir_buf = dirs::app_home_dir()?; let log_path_buf = dirs::service_log_file()?; let bin_path = dirs::path_to_str(&bin_path_buf)?; let config_dir = dirs::path_to_str(&config_dir_buf)?; let log_path = dirs::path_to_str(&log_path_buf)?; let config_file = dirs::path_to_str(config_file)?; let mut map = HashMap::new(); map.insert("core_type", clash_core.as_str()); map.insert("bin_path", bin_path); map.insert("config_dir", config_dir); map.insert("config_file", config_file); map.insert("log_file", log_path);
let res = reqwest::ClientBuilder::new().no_proxy().build()?.post(format!("{SERVICE_URL}/start_clash")).json(&map).send().await?.json::<JsonResponse>().await.context("failed to connect to the clash-verge-service")?; if res.code != 0 { bail!(res.msg); } Ok(()) }

pub(super) async fn stop_core_by_service() -> Result<()> { let res = reqwest::ClientBuilder::new().no_proxy().build()?.post(format!("{SERVICE_URL}/stop_clash")).send().await?.json::<JsonResponse>().await.context("failed to connect to the clash-verge-service")?; if res.code != 0 { bail!(res.msg); } Ok(()) }
