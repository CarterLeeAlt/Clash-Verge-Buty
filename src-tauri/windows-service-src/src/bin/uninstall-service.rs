use anyhow::Result;
use windows_service::service::ServiceAccess;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use clash_verge_service_src::{LEGACY_SERVICE_NAME, SERVICE_NAME};

fn remove_one(manager: &ServiceManager, name: &str) -> Result<()> {
    if let Ok(service) = manager.open_service(name, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
        let _ = service.stop();
        service.delete()?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    remove_one(&manager, SERVICE_NAME)?;
    remove_one(&manager, LEGACY_SERVICE_NAME)?;
    Ok(())
}
