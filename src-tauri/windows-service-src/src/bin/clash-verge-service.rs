use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use std::{fs, path::PathBuf, process::{Child, Command, Stdio}};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tiny_http::{Method, Response, Server, StatusCode};
use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use clash_verge_windows_service_src::{API_ADDR, API_GET_CLASH, API_HEALTH, API_START_CLASH, API_STOP_CLASH, SERVICE_NAME};

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
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_for_handler = Arc::clone(&stop_flag);

    let status_handle = service_control_handler::register(SERVICE_NAME, move |control_event| match control_event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            eprintln!("stop/shutdown received");
            stop_for_handler.store(true, Ordering::SeqCst);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;

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
            return Err(anyhow::anyhow!("failed to bind service API on {API_ADDR}: {err}"));
        }
    };

    let (server_done_tx, server_done_rx) = mpsc::channel();
    let clash_state: Arc<std::sync::Mutex<Option<ClashState>>> = Arc::new(std::sync::Mutex::new(None));
    let stop_for_server = Arc::clone(&stop_flag);
    let clash_state_for_server = Arc::clone(&clash_state);
    let server_thread = thread::spawn(move || {
        while !stop_for_server.load(Ordering::Relaxed) {
            match server.recv_timeout(Duration::from_millis(300)) {
                Ok(Some(mut req)) => {
                    let method = req.method().clone();
                    let url = req.url().to_string();
                    let (status, body) = match (method, url.as_str()) {
                        (Method::Get, API_HEALTH) => {
                            (200, serde_json::to_string(&JsonResponse::<serde_json::Value> {
                                code: 0,
                                msg: "ok".into(),
                                data: Some(serde_json::json!({"service": "running"})),
                            }))
                        }
                        (Method::Get, API_GET_CLASH) => {
                            let mut state = clash_state_for_server.lock().unwrap();
                            if let Some(state_ref) = state.as_mut() {
                                let running = state_ref.child.try_wait().ok().flatten().is_none();
                                if running {
                                    eprintln!("/get_clash running=true pid={}", state_ref.meta.pid);
                                    (200, serde_json::to_string(&JsonResponse {
                                        code: 0,
                                        msg: "ok".into(),
                                        data: Some(state_ref.meta.clone()),
                                    }))
                                } else {
                                    eprintln!("/get_clash running=false, clearing state");
                                    *state = None;
                                    (500, serde_json::to_string(&JsonResponse::<()> { code: 500, msg: "clash core is not running".into(), data: None }))
                                }
                            } else {
                                eprintln!("/get_clash running=false state=null");
                                (500, serde_json::to_string(&JsonResponse::<()> { code: 500, msg: "clash core is not started".into(), data: None }))
                            }
                        }
                        (Method::Post, API_START_CLASH) => {
                            eprintln!("/start_clash received");
                            let request: Result<StartClashRequest, _> = serde_json::from_reader(req.as_reader());
                            match request {
                                Ok(payload) => (200, start_clash(payload, &clash_state_for_server)),
                                Err(err) => (400, serde_json::to_string(&JsonResponse::<()> { code: 400, msg: format!("invalid request body: {err}"), data: None })),
                            }
                        }
                        (Method::Post, API_STOP_CLASH) => {
                            eprintln!("/stop_clash called");
                            (200, stop_clash(&clash_state_for_server))
                        }
                        _ => (404, serde_json::to_string(&JsonResponse::<()> { code: 404, msg: "not found".into(), data: None })),
                    };
                    let body = body.unwrap_or_else(|_| "{\"code\":500,\"msg\":\"serialize error\",\"data\":null}".into());
                    let _ = req.respond(Response::from_string(body).with_status_code(StatusCode(status)));
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

fn start_clash(payload: StartClashRequest, state: &Arc<std::sync::Mutex<Option<ClashState>>>) -> Result<String, serde_json::Error> {
    let bin_path = PathBuf::from(&payload.bin_path);
    let config_dir = PathBuf::from(&payload.config_dir);
    let config_file = PathBuf::from(&payload.config_file);
    let log_file = PathBuf::from(&payload.log_file);

    let bin_exists = bin_path.exists();
    let config_dir_exists = config_dir.exists();
    let config_file_exists = config_file.exists();
    let log_parent_exists = log_file.parent().map(|p| p.exists()).unwrap_or(false);
    eprintln!("start_clash fields: core_type={}, bin_path exists={}, config_dir exists={}, config_file exists={}, log_file={}", payload.core_type, bin_exists, config_dir_exists, config_file_exists, payload.log_file);

    if !bin_exists { return serde_json::to_string(&JsonResponse::<()> { code: 400, msg: format!("bin_path not found: {}", payload.bin_path), data: None }); }
    if !config_dir_exists { return serde_json::to_string(&JsonResponse::<()> { code: 400, msg: format!("config_dir not found: {}", payload.config_dir), data: None }); }
    if !config_file_exists { return serde_json::to_string(&JsonResponse::<()> { code: 400, msg: format!("config_file not found: {}", payload.config_file), data: None }); }
    if !log_parent_exists { return serde_json::to_string(&JsonResponse::<()> { code: 400, msg: format!("log_file parent not found: {}", payload.log_file), data: None }); }

    let mut locked = state.lock().unwrap();
    if let Some(existing) = locked.as_mut() {
        let _ = existing.child.kill();
    }
    *locked = None;

    let log_out = match fs::OpenOptions::new().create(true).append(true).open(&log_file) {
        Ok(file) => file,
        Err(err) => return serde_json::to_string(&JsonResponse::<()> { code: 500, msg: format!("open log file failed: {err}"), data: None }),
    };
    let log_err = match log_out.try_clone() {
        Ok(file) => file,
        Err(err) => return serde_json::to_string(&JsonResponse::<()> { code: 500, msg: format!("clone log file failed: {err}"), data: None }),
    };

    eprintln!("spawning clash-meta");
    let mut child = match Command::new(&bin_path)
        .args(["-d", &payload.config_dir, "-f", &payload.config_file])
        .stdout(Stdio::from(log_out))
        .stderr(Stdio::from(log_err))
        .spawn() {
        Ok(child) => child,
        Err(err) => return serde_json::to_string(&JsonResponse::<()> { code: 500, msg: format!("spawn clash core failed: {err}"), data: None }),
    };
    let pid = child.id();
    eprintln!("spawn pid={pid}");
    if let Ok(Some(status)) = child.try_wait() {
        eprintln!("child exited immediately: {status}");
        return serde_json::to_string(&JsonResponse::<()> { code: 500, msg: format!("clash core exited immediately: {status}"), data: None });
    }

    let meta = ClashStateData {
        core_type: payload.core_type,
        bin_path: payload.bin_path,
        config_dir: payload.config_dir,
        config_file: payload.config_file,
        log_file: payload.log_file,
        pid,
        running: true,
    };
    *locked = Some(ClashState { child, meta: meta.clone() });

    serde_json::to_string(&JsonResponse { code: 0, msg: "started".into(), data: Some(meta) })
}

fn stop_clash(state: &Arc<std::sync::Mutex<Option<ClashState>>>) -> Result<String, serde_json::Error> {
    let mut locked = state.lock().unwrap();
    if let Some(mut running) = locked.take() {
        let _ = running.child.kill();
    }
    serde_json::to_string(&JsonResponse::<()> { code: 0, msg: "stopped".into(), data: None })
}
