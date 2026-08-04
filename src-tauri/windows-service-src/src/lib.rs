// Keep these constants aligned with src-tauri/src/core/win_service.rs.
pub const SERVICE_NAME: &str = "clash-verge-service";
pub const SERVICE_DISPLAY_NAME: &str = "clash-verge-service";
pub const SERVICE_BINARY: &str = "clash-verge-service.exe";
pub const INSTALL_HELPER: &str = "install-service.exe";
pub const UNINSTALL_HELPER: &str = "uninstall-service.exe";

pub const API_ADDR: &str = "127.0.0.1:33211";
pub const API_HEALTH: &str = "/health";
pub const API_GET_CLASH: &str = "/get_clash";
pub const API_START_CLASH: &str = "/start_clash";
pub const API_STOP_CLASH: &str = "/stop_clash";
pub const API_STOP_SERVICE: &str = "/stop_service";

pub const API_TOKEN_DIR: &str = "service-data";
pub const API_TOKEN_FILE: &str = "service-api-token";

pub fn api_token_path() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|path| path.join(API_TOKEN_DIR).join(API_TOKEN_FILE))
}

pub fn remove_token_file_and_empty_parent(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}
