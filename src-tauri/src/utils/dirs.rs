use crate::core::handle;
use anyhow::Result;
use std::path::PathBuf;
use tauri::{api::path::resource_dir, Env};

#[cfg(not(feature = "verge-dev"))]
pub static APP_ID: &str = "io.github.clash-verge-buty.data";
#[cfg(feature = "verge-dev")]
pub static APP_ID: &str = "io.github.clash-verge-buty.data.dev";

static CLASH_CONFIG: &str = "config.yaml";
static VERGE_CONFIG: &str = "verge.yaml";
static PROFILE_YAML: &str = "profiles.yaml";

/// Directory that contains the running executable. All application-created
/// files must stay in this directory or one of its descendants.
pub fn executable_dir() -> Result<PathBuf> {
    use tauri::utils::platform::current_exe;

    let app_exe = dunce::canonicalize(current_exe()?)?;
    app_exe
        .parent()
        .map(PathBuf::from)
        .ok_or(anyhow::anyhow!("failed to get the executable directory"))
}

/// get the verge app home dir
pub fn app_home_dir() -> Result<PathBuf> {
    Ok(executable_dir()?.join(".config").join(APP_ID))
}

/// WebView cache, cookies, LocalStorage and IndexedDB.
pub fn webview_data_dir() -> Result<PathBuf> {
    Ok(app_home_dir()?.join("webview"))
}

/// Temporary files owned by this application.
pub fn app_temp_dir() -> Result<PathBuf> {
    Ok(app_home_dir()?.join("temp"))
}

/// get the resources dir
pub fn app_resources_dir() -> Result<PathBuf> {
    let handle = handle::Handle::global();
    let app_handle = handle.app_handle.lock();
    if let Some(app_handle) = app_handle.as_ref() {
        let res_dir = resource_dir(app_handle.package_info(), &Env::default())
            .ok_or(anyhow::anyhow!("failed to get the resource dir"))?
            .join("resources");
        return Ok(res_dir);
    };
    Err(anyhow::anyhow!("failed to get the resource dir"))
}

/// profiles dir
pub fn app_profiles_dir() -> Result<PathBuf> {
    Ok(app_home_dir()?.join("profiles"))
}

/// logs dir
pub fn logs_dir() -> Result<PathBuf> {
    Ok(app_home_dir()?.join("logs"))
}

/// app logs dir
pub fn app_logs_dir() -> Result<PathBuf> {
    let dir = logs_dir()?.join("app");
    let _ = std::fs::create_dir_all(&dir);
    Ok(dir)
}

/// service logs dir
pub fn service_logs_dir() -> Result<PathBuf> {
    let dir = logs_dir()?.join("service");
    let _ = std::fs::create_dir_all(&dir);
    Ok(dir)
}

pub fn clash_path() -> Result<PathBuf> {
    Ok(app_home_dir()?.join(CLASH_CONFIG))
}

pub fn verge_path() -> Result<PathBuf> {
    Ok(app_home_dir()?.join(VERGE_CONFIG))
}

pub fn profiles_path() -> Result<PathBuf> {
    Ok(app_home_dir()?.join(PROFILE_YAML))
}

pub fn clash_pid_path() -> Result<PathBuf> {
    Ok(app_home_dir()?.join("clash.pid"))
}

#[cfg(windows)]
pub fn service_path() -> Result<PathBuf> {
    // Windows service binary keeps the historical filename clash-verge-service.exe
    // because local-binaries and CI provide it under this name.
    Ok(app_resources_dir()?.join("clash-verge-service.exe"))
}

#[cfg(windows)]
pub fn firewall_helper_path() -> Result<PathBuf> {
    Ok(app_resources_dir()?.join("firewall-helper.exe"))
}

#[cfg(windows)]
pub fn service_api_token_path() -> Result<PathBuf> {
    Ok(app_resources_dir()?
        .join("service-data")
        .join("service-api-token"))
}

#[cfg(windows)]
pub fn service_log_file() -> Result<PathBuf> {
    use chrono::Local;

    let log_dir = service_logs_dir()?;

    let local_time = Local::now().format("%Y-%m-%d-%H%M").to_string();
    let log_file = format!("{}.log", local_time);
    let log_file = log_dir.join(log_file);

    Ok(log_file)
}

pub fn path_to_str(path: &PathBuf) -> Result<&str> {
    let path_str = path
        .as_os_str()
        .to_str()
        .ok_or(anyhow::anyhow!("failed to get path from {:?}", path))?;
    Ok(path_str)
}
