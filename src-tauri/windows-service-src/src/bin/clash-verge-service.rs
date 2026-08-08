use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use std::{
    ffi::OsString,
    fs,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tiny_http::{Method, Response, Server, StatusCode};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use clash_verge_windows_service_src::{
    api_token_path, API_ADDR, API_GET_CLASH, API_HEALTH, API_START_CLASH, API_STOP_CLASH,
    API_STOP_SERVICE, SERVICE_NAME,
};

#[derive(Serialize)]
struct JsonResponse<T> {
    code: u64,
    msg: String,
    data: Option<T>,
}

#[derive(Deserialize)]
struct StartClashRequest {
    core_type: String,
    bin_path: String,
    config_dir: String,
    config_file: String,
    log_file: String,
}

struct ValidatedStartClashRequest {
    core_type: String,
    bin_path: PathBuf,
    config_dir: PathBuf,
    config_file: PathBuf,
    log_file: PathBuf,
}

#[derive(Serialize, Clone)]
struct ClashStateData {
    core_type: String,
    bin_path: String,
    config_dir: String,
    config_file: String,
    log_file: String,
    pid: u32,
    running: bool,
}

struct ClashState {
    child: Child,
    meta: ClashStateData,
}

define_windows_service!(ffi_service_main, service_main);

fn main() -> Result<()> {
    eprintln!("service starting: dispatcher start for {}", SERVICE_NAME);
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

fn service_main(_arguments: Vec<std::ffi::OsString>) {
    if let Err(err) = run_service() {
        eprintln!("service main failed: {err}");
    }
}

fn run_service() -> Result<()> {
    let api_token_path =
        api_token_path().context("failed to resolve the local service token path")?;
    let api_token = fs::read_to_string(&api_token_path)
        .with_context(|| format!("failed to read API token from {}", api_token_path.display()))?;
    let api_token = api_token
        .trim()
        .split_once(':')
        .map(|(_, token)| token.to_string())
        .context("service API token owner is missing")?;
    if api_token.len() != 64 || !api_token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("service API token is invalid; reinstall the service");
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_for_handler = Arc::clone(&stop_flag);

    let status_handle =
        service_control_handler::register(
            SERVICE_NAME,
            move |control_event| match control_event {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    eprintln!("stop/shutdown received");
                    stop_for_handler.store(true, Ordering::SeqCst);
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            },
        )?;

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 1,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    })?;

    eprintln!("binding API_ADDR={API_ADDR}");
    let server = match Server::http(API_ADDR) {
        Ok(server) => {
            eprintln!("API bind success");
            server
        }
        Err(err) => {
            eprintln!("API bind failed: {err}");
            return Err(anyhow::anyhow!(
                "failed to bind service API on {API_ADDR}: {err}"
            ));
        }
    };

    let (server_done_tx, server_done_rx) = mpsc::channel();
    let clash_state: Arc<std::sync::Mutex<Option<ClashState>>> =
        Arc::new(std::sync::Mutex::new(None));
    let stop_for_server = Arc::clone(&stop_flag);
    let clash_state_for_server = Arc::clone(&clash_state);
    let server_thread = thread::spawn(move || {
        while !stop_for_server.load(Ordering::Relaxed) {
            match server.recv_timeout(Duration::from_millis(300)) {
                Ok(Some(mut req)) => {
                    if !request_is_authorized(&req, &api_token) {
                        let body = serde_json::to_string(&JsonResponse::<()> {
                            code: 401,
                            msg: "unauthorized".into(),
                            data: None,
                        })
                        .unwrap_or_else(|_| {
                            "{\"code\":401,\"msg\":\"unauthorized\",\"data\":null}".into()
                        });
                        let _ = req
                            .respond(Response::from_string(body).with_status_code(StatusCode(401)));
                        continue;
                    }

                    let method = req.method().clone();
                    let url = req.url().to_string();
                    let (status, body) = match (method, url.as_str()) {
                        (Method::Get, API_HEALTH) => (
                            200,
                            serde_json::to_string(&JsonResponse::<serde_json::Value> {
                                code: 0,
                                msg: "ok".into(),
                                data: Some(serde_json::json!({"service": "running"})),
                            }),
                        ),
                        (Method::Get, API_GET_CLASH) => {
                            let mut state = clash_state_for_server.lock().unwrap();
                            if let Some(state_ref) = state.as_mut() {
                                let running = state_ref.child.try_wait().ok().flatten().is_none();
                                if running {
                                    eprintln!("/get_clash running=true pid={}", state_ref.meta.pid);
                                    (
                                        200,
                                        serde_json::to_string(&JsonResponse {
                                            code: 0,
                                            msg: "ok".into(),
                                            data: Some(state_ref.meta.clone()),
                                        }),
                                    )
                                } else {
                                    eprintln!("/get_clash running=false, clearing state");
                                    *state = None;
                                    (
                                        500,
                                        serde_json::to_string(&JsonResponse::<()> {
                                            code: 500,
                                            msg: "Mihomo core is not running".into(),
                                            data: None,
                                        }),
                                    )
                                }
                            } else {
                                eprintln!("/get_clash running=false state=null");
                                (
                                    500,
                                    serde_json::to_string(&JsonResponse::<()> {
                                        code: 500,
                                        msg: "Mihomo core is not started".into(),
                                        data: None,
                                    }),
                                )
                            }
                        }
                        (Method::Post, API_START_CLASH) => {
                            eprintln!("/start_clash received");
                            let request: Result<StartClashRequest, _> =
                                serde_json::from_reader(req.as_reader());
                            match request {
                                Ok(payload) => (200, start_clash(payload, &clash_state_for_server)),
                                Err(err) => (
                                    400,
                                    serde_json::to_string(&JsonResponse::<()> {
                                        code: 400,
                                        msg: format!("invalid request body: {err}"),
                                        data: None,
                                    }),
                                ),
                            }
                        }
                        (Method::Post, API_STOP_CLASH) => {
                            eprintln!("/stop_clash called");
                            (200, stop_clash(&clash_state_for_server))
                        }
                        (Method::Post, API_STOP_SERVICE) => {
                            eprintln!("/stop_service called");
                            let body = stop_clash(&clash_state_for_server);
                            stop_for_server.store(true, Ordering::SeqCst);
                            (200, body)
                        }
                        _ => (
                            404,
                            serde_json::to_string(&JsonResponse::<()> {
                                code: 404,
                                msg: "not found".into(),
                                data: None,
                            }),
                        ),
                    };
                    let body = body.unwrap_or_else(|_| {
                        "{\"code\":500,\"msg\":\"serialize error\",\"data\":null}".into()
                    });
                    let _ = req
                        .respond(Response::from_string(body).with_status_code(StatusCode(status)));
                }
                Ok(None) => {}
                Err(err) => {
                    eprintln!("API server loop error: {err}");
                    break;
                }
            }
        }
        let _ = server_done_tx.send(());
    });

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    eprintln!("service status set to Running");

    while !stop_flag.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(200));
    }

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StopPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 1,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    })?;

    let _ = server_done_rx.recv_timeout(Duration::from_secs(3));
    let _ = server_thread.join();

    let _ = stop_clash(&clash_state);

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    eprintln!("service stopped");

    Ok(())
}

fn request_is_authorized(request: &tiny_http::Request, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    request
        .headers()
        .iter()
        .any(|header| header.field.equiv("Authorization") && header.value.as_str() == expected)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

/// Mihomo joins files below its data directory with `/`. A verbatim Windows
/// path such as `\\?\C:\data` therefore becomes the invalid mixed path
/// `\\?\C:\data/file`. Keep canonical paths for validation, but pass their
/// equivalent ordinary Win32 form to the child process.
fn win32_compatible_path(path: &Path) -> PathBuf {
    const VERBATIM_PREFIX: [u16; 4] =
        [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const VERBATIM_UNC_PREFIX: [u16; 8] = [
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];

    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let normalized = if wide.starts_with(&VERBATIM_UNC_PREFIX) {
        let mut result = vec![b'\\' as u16, b'\\' as u16];
        result.extend_from_slice(&wide[VERBATIM_UNC_PREFIX.len()..]);
        result
    } else if wide.starts_with(&VERBATIM_PREFIX)
        && wide.get(5).copied() == Some(b':' as u16)
    {
        wide[VERBATIM_PREFIX.len()..].to_vec()
    } else {
        return path.to_path_buf();
    };

    PathBuf::from(OsString::from_wide(&normalized))
}

fn validate_start_request(payload: StartClashRequest) -> Result<ValidatedStartClashRequest> {
    let expected_binary_name = match payload.core_type.as_str() {
        "mihomo" => "mihomo.exe",
        "mihomo-alpha" => "mihomo-alpha.exe",
        other => bail!("unsupported core_type: {other}"),
    };

    let service_exe = fs::canonicalize(std::env::current_exe()?)?;
    let service_dir = service_exe
        .parent()
        .context("service executable has no parent directory")?;
    let install_dir = if service_dir
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("resources"))
    {
        service_dir
            .parent()
            .context("service resources directory has no parent")?
    } else {
        service_dir
    };

    let expected_binary = fs::canonicalize(install_dir.join(expected_binary_name))
        .with_context(|| format!("approved core binary not found: {expected_binary_name}"))?;
    let bin_path = fs::canonicalize(&payload.bin_path)
        .with_context(|| format!("bin_path not found: {}", payload.bin_path))?;
    if !paths_equal(&bin_path, &expected_binary) || !bin_path.is_file() {
        bail!("bin_path is not the approved installed core binary");
    }

    let config_dir = fs::canonicalize(&payload.config_dir)
        .with_context(|| format!("config_dir not found: {}", payload.config_dir))?;
    let expected_config_parent = fs::canonicalize(install_dir.join(".config"))
        .context("portable .config directory not found beside the installed executable")?;
    let approved_config_name = config_dir.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.eq_ignore_ascii_case("io.github.clash-verge-buty.data")
            || name.eq_ignore_ascii_case("io.github.clash-verge-buty.data.dev")
    });
    if !config_dir.is_dir()
        || !approved_config_name
        || !config_dir
            .parent()
            .is_some_and(|parent| paths_equal(parent, &expected_config_parent))
    {
        bail!("config_dir is not the portable data directory beside the installed executable");
    }

    let config_file = fs::canonicalize(&payload.config_file)
        .with_context(|| format!("config_file not found: {}", payload.config_file))?;
    if !config_file.is_file()
        || config_file.file_name().and_then(|name| name.to_str()) != Some("clash-verge-buty.yaml")
        || !config_file
            .parent()
            .is_some_and(|parent| paths_equal(parent, &config_dir))
    {
        bail!("config_file is outside the approved data directory");
    }

    let log_file = PathBuf::from(&payload.log_file);
    let log_parent = log_file
        .parent()
        .context("log_file has no parent directory")?;
    let log_parent = fs::canonicalize(log_parent)
        .with_context(|| format!("log_file parent not found: {}", payload.log_file))?;
    let expected_log_parent = fs::canonicalize(config_dir.join("logs").join("service"))
        .context("approved service log directory not found")?;
    if !paths_equal(&log_parent, &expected_log_parent)
        || log_file.extension().and_then(|ext| ext.to_str()) != Some("log")
    {
        bail!("log_file is outside the approved service log directory");
    }
    if log_file.exists() {
        let canonical_log = fs::canonicalize(&log_file)?;
        if !canonical_log
            .parent()
            .is_some_and(|parent| paths_equal(parent, &expected_log_parent))
        {
            bail!("log_file resolves outside the approved service log directory");
        }
    }

    Ok(ValidatedStartClashRequest {
        core_type: payload.core_type,
        bin_path: win32_compatible_path(&bin_path),
        config_dir: win32_compatible_path(&config_dir),
        config_file: win32_compatible_path(&config_file),
        log_file: win32_compatible_path(&log_file),
    })
}

fn start_clash(
    payload: StartClashRequest,
    state: &Arc<std::sync::Mutex<Option<ClashState>>>,
) -> Result<String, serde_json::Error> {
    let payload = match validate_start_request(payload) {
        Ok(payload) => payload,
        Err(err) => {
            return serde_json::to_string(&JsonResponse::<()> {
                code: 400,
                msg: err.to_string(),
                data: None,
            })
        }
    };
    eprintln!(
        "start_clash validated: core_type={}, bin_path={}, config_dir={}, config_file={}, log_file={}",
        payload.core_type,
        payload.bin_path.display(),
        payload.config_dir.display(),
        payload.config_file.display(),
        payload.log_file.display()
    );

    let mut locked = state.lock().unwrap();
    if let Some(existing) = locked.as_mut() {
        let _ = existing.child.kill();
        let _ = existing.child.wait();
    }
    *locked = None;

    let log_out = match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&payload.log_file)
    {
        Ok(file) => file,
        Err(err) => {
            return serde_json::to_string(&JsonResponse::<()> {
                code: 500,
                msg: format!("open log file failed: {err}"),
                data: None,
            })
        }
    };
    let log_err = match log_out.try_clone() {
        Ok(file) => file,
        Err(err) => {
            return serde_json::to_string(&JsonResponse::<()> {
                code: 500,
                msg: format!("clone log file failed: {err}"),
                data: None,
            })
        }
    };

    eprintln!("spawning mihomo");
    let mut child = match Command::new(&payload.bin_path)
        .arg("-d")
        .arg(&payload.config_dir)
        .arg("-f")
        .arg(&payload.config_file)
        .stdout(Stdio::from(log_out))
        .stderr(Stdio::from(log_err))
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return serde_json::to_string(&JsonResponse::<()> {
                code: 500,
                msg: format!("spawn Mihomo core failed: {err}"),
                data: None,
            })
        }
    };
    let pid = child.id();
    eprintln!("spawn pid={pid}");
    if let Ok(Some(status)) = child.try_wait() {
        eprintln!("child exited immediately: {status}");
        return serde_json::to_string(&JsonResponse::<()> {
            code: 500,
            msg: format!("Mihomo core exited immediately: {status}"),
            data: None,
        });
    }

    let meta = ClashStateData {
        core_type: payload.core_type,
        bin_path: payload.bin_path.to_string_lossy().into_owned(),
        config_dir: payload.config_dir.to_string_lossy().into_owned(),
        config_file: payload.config_file.to_string_lossy().into_owned(),
        log_file: payload.log_file.to_string_lossy().into_owned(),
        pid,
        running: true,
    };
    *locked = Some(ClashState {
        child,
        meta: meta.clone(),
    });

    serde_json::to_string(&JsonResponse {
        code: 0,
        msg: "started".into(),
        data: Some(meta),
    })
}

fn stop_clash(
    state: &Arc<std::sync::Mutex<Option<ClashState>>>,
) -> Result<String, serde_json::Error> {
    let mut locked = state.lock().unwrap();
    if let Some(mut running) = locked.take() {
        let _ = running.child.kill();
        let _ = running.child.wait();
    }
    serde_json::to_string(&JsonResponse::<()> {
        code: 0,
        msg: "stopped".into(),
        data: None,
    })
}

#[cfg(test)]
mod tests {
    use super::win32_compatible_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn strips_verbatim_disk_prefix_for_child_processes() {
        assert_eq!(
            win32_compatible_path(Path::new(r"\\?\C:\Portable\Clash")),
            PathBuf::from(r"C:\Portable\Clash")
        );
    }

    #[test]
    fn converts_verbatim_unc_prefix_for_child_processes() {
        assert_eq!(
            win32_compatible_path(Path::new(r"\\?\UNC\server\share\Clash")),
            PathBuf::from(r"\\server\share\Clash")
        );
    }

    #[test]
    fn preserves_regular_windows_paths() {
        assert_eq!(
            win32_compatible_path(Path::new(r"C:\Portable\Clash")),
            PathBuf::from(r"C:\Portable\Clash")
        );
    }
}
