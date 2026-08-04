use anyhow::{Context, Result};
use clash_verge_windows_service_src::{api_token_path, SERVICE_DISPLAY_NAME, SERVICE_NAME};
use rand::{rngs::OsRng, RngCore};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::process::Command;
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

fn current_user_sid() -> Result<String> {
    let output = Command::new("whoami.exe")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .context("failed to query current Windows user SID")?;
    if !output.status.success() {
        anyhow::bail!("whoami.exe failed while querying the current user SID");
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let line = line.trim().trim_matches('"');
    let sid = line
        .split("\",\"")
        .last()
        .map(str::trim)
        .filter(|sid| sid.starts_with("S-1-"))
        .context("failed to parse current Windows user SID")?;
    Ok(sid.to_string())
}

fn requested_user_sid() -> Result<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--user-sid" {
            let sid = args.next().context("--user-sid requires a value")?;
            if sid.starts_with("S-1-")
                && sid
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'-' || byte == b'S')
            {
                return Ok(sid);
            }
            anyhow::bail!("invalid --user-sid value");
        }
    }
    current_user_sid()
}

fn restrict_token_acl(path: &std::path::Path, user_sid: &str) -> Result<()> {
    let user_rule = format!("*{user_sid}:R");
    let status = Command::new("icacls.exe")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(&user_rule)
        .arg("*S-1-5-18:F")
        .arg("*S-1-5-32-544:F")
        .status()
        .context("failed to set service token ACL")?;
    if !status.success() {
        anyhow::bail!("icacls.exe failed to protect the service token");
    }
    Ok(())
}

fn restrict_token_directory_acl(path: &std::path::Path, user_sid: &str) -> Result<()> {
    let user_rule = format!("*{user_sid}:(OI)(CI)R");
    let status = Command::new("icacls.exe")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(&user_rule)
        .arg("*S-1-5-18:(OI)(CI)F")
        .arg("*S-1-5-32-544:(OI)(CI)F")
        .status()
        .context("failed to set service token directory ACL")?;
    if !status.success() {
        anyhow::bail!("icacls.exe failed to protect the service token directory");
    }
    Ok(())
}

fn ensure_api_token(user_sid: &str) -> Result<()> {
    let path = api_token_path().context("failed to resolve the local service token path")?;
    let parent = path.parent().context("service token path has no parent")?;
    fs::create_dir_all(parent).context("failed to create service token directory")?;
    restrict_token_directory_acl(parent, user_sid)?;

    if let Ok(existing) = fs::read_to_string(&path) {
        let existing = existing.trim();
        if existing.split_once(':').is_some_and(|(owner, token)| {
            owner == user_sid
                && token.len() == 64
                && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            restrict_token_acl(&path, user_sid)?;
            return Ok(());
        }
    }

    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let token_record = format!("{user_sid}:{token}");
    let tmp_path = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = fs::File::create(&tmp_path).context("failed to create service token")?;
    file.write_all(token_record.as_bytes())
        .context("failed to write service token")?;
    file.sync_all().context("failed to flush service token")?;
    drop(file);
    if let Err(err) = restrict_token_acl(&tmp_path, user_sid) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    fs::rename(&tmp_path, &path).or_else(|_| {
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&tmp_path, &path)
    })?;
    if let Err(err) = restrict_token_acl(&path, user_sid) {
        let _ = fs::remove_file(&path);
        return Err(err);
    }
    Ok(())
}

fn main() -> Result<()> {
    let user_sid = requested_user_sid()?;
    ensure_api_token(&user_sid)?;

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("failed to connect ServiceManager")?;

    let service = match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP,
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
                .create_service(
                    &info,
                    ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP,
                )
                .with_context(|| format!("failed to create service '{}'", SERVICE_NAME))?
        }
    };

    let status = service
        .query_status()
        .context("failed to query service status")?;
    if status.current_state == ServiceState::Running {
        service
            .stop()
            .with_context(|| format!("failed to stop service '{}'", SERVICE_NAME))?;
        for _ in 0..40 {
            if service.query_status()?.current_state == ServiceState::Stopped {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if service.query_status()?.current_state != ServiceState::Stopped {
            anyhow::bail!("service '{}' did not stop in time", SERVICE_NAME);
        }
    }

    let args: Vec<OsString> = Vec::new();
    service
        .start(&args)
        .with_context(|| format!("failed to start service '{}'", SERVICE_NAME))?;
    std::thread::sleep(Duration::from_millis(500));

    Ok(())
}
