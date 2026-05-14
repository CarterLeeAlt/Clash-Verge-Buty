// Keep these constants aligned with src-tauri/src/core/win_service.rs.
pub const SERVICE_NAME: &str = "clash-verge-service";
pub const LEGACY_SERVICE_NAME: &str = "clash_verge_service";
pub const SERVICE_DISPLAY_NAME: &str = "clash-verge-service";
pub const SERVICE_BINARY: &str = "clash-verge-service.exe";
pub const INSTALL_HELPER: &str = "install-service.exe";
pub const UNINSTALL_HELPER: &str = "uninstall-service.exe";

pub const API_ADDR: &str = "127.0.0.1:33211";
pub const API_GET_CLASH: &str = "/get_clash";
pub const API_START_CLASH: &str = "/start_clash";
pub const API_STOP_CLASH: &str = "/stop_clash";
