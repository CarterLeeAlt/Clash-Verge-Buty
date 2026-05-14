use anyhow::Result;
use clash_verge_service_src::{LEGACY_SERVICE_NAME, SERVICE_NAME};
use windows_service::service::{ServiceAccess, ServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

fn remove_one(manager: &ServiceManager, name: &str) -> Result<()> {
    let Ok(service) = manager.open_service(
        name,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) else {
        eprintln!("service '{}' does not exist, skip", name);
        return Ok(());
    };

    if let Ok(status) = service.query_status() {
        if status.current_state == ServiceState::Running {
            let _ = service.stop();
        }
    }

    service.delete()?;
    eprintln!("service '{}' deleted", name);
    Ok(())
}

fn main() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    remove_one(&manager, SERVICE_NAME)?;
    remove_one(&manager, LEGACY_SERVICE_NAME)?;
    Ok(())
}
