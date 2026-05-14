use anyhow::{Context, Result};
use clash_verge_service_src::{LEGACY_SERVICE_NAME, SERVICE_DISPLAY_NAME, SERVICE_NAME};
use std::ffi::OsString;
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

fn main() -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("failed to connect ServiceManager")?;

    if manager
        .open_service(LEGACY_SERVICE_NAME, ServiceAccess::QUERY_STATUS)
        .is_ok()
    {
        eprintln!(
            "legacy service '{}' exists; please migrate to '{}' before installing",
            LEGACY_SERVICE_NAME, SERVICE_NAME
        );
    }

    let service = match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::START,
    ) {
        Ok(existing) => existing,
        Err(_) => {
            let exe = std::env::current_exe()
                .context("failed to resolve install-service.exe path")?
                .with_file_name("clash-verge-service.exe");
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
            manager
                .create_service(&info, ServiceAccess::QUERY_STATUS | ServiceAccess::START)
                .with_context(|| format!("failed to create service '{}'", SERVICE_NAME))?
        }
    };

    let status = service.query_status().context("failed to query service status")?;
    if status.current_state != ServiceState::Running {
        let args: Vec<OsString> = Vec::new();
        service
            .start(&args)
            .with_context(|| format!("failed to start service '{}'", SERVICE_NAME))?;
        std::thread::sleep(Duration::from_millis(500));
    }

    Ok(())
}
