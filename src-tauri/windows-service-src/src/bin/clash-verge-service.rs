use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use serde::Serialize;
use tiny_http::{Method, Response, Server, StatusCode};
use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use clash_verge_service_src::{API_ADDR, API_GET_CLASH, API_START_CLASH, API_STOP_CLASH, SERVICE_NAME};

#[derive(Serialize)]
struct JsonResponse<T> {
    code: u64,
    msg: String,
    data: Option<T>,
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
    let stop_for_server = Arc::clone(&stop_flag);
    let server_thread = thread::spawn(move || {
        while !stop_for_server.load(Ordering::Relaxed) {
            match server.recv_timeout(Duration::from_millis(300)) {
                Ok(Some(req)) => {
                    let (code, msg) = match (req.method(), req.url()) {
                        (&Method::Get, API_GET_CLASH) => (0, "ok"),
                        (&Method::Post, API_START_CLASH) => (0, "started"),
                        (&Method::Post, API_STOP_CLASH) => (0, "stopped"),
                        _ => (404, "not found"),
                    };
                    let body = serde_json::to_string(&JsonResponse::<()> { code, msg: msg.into(), data: None })
                        .unwrap_or_else(|_| "{\"code\":500,\"msg\":\"serialize error\",\"data\":null}".into());
                    let status = if code == 0 { 200 } else { 404 };
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
