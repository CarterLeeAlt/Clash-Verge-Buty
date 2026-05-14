use anyhow::Result;
use std::ffi::OsString;
use std::time::Duration;
use windows_service::service::{ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use clash_verge_service_src::{SERVICE_DISPLAY_NAME, SERVICE_NAME};

fn main() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let exe = std::env::current_exe()?.with_file_name("clash-verge-service.exe");
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec![],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let service = manager.create_service(&info, ServiceAccess::QUERY_STATUS | ServiceAccess::START)?;
    service.start(&[])?;
    std::thread::sleep(Duration::from_millis(500));
    Ok(())
}
