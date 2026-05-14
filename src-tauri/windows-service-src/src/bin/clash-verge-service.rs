use serde::Serialize;
use tiny_http::{Method, Response, Server};

use clash_verge_service_src::{API_ADDR, API_GET_CLASH, API_START_CLASH, API_STOP_CLASH};

#[derive(Serialize)]
struct JsonResponse<T> { code: u64, msg: String, data: Option<T> }

fn main() -> anyhow::Result<()> {
    let server = Server::http(API_ADDR)?;
    for req in server.incoming_requests() {
        let (code,msg) = match (req.method(), req.url()) {
            (&Method::Get, API_GET_CLASH) => (0, "ok"),
            (&Method::Post, API_START_CLASH) => (0, "started"),
            (&Method::Post, API_STOP_CLASH) => (0, "stopped"),
            _ => (404, "not found"),
        };
        let body = serde_json::to_string(&JsonResponse::<()> { code, msg: msg.into(), data: None })?;
        let _ = req.respond(Response::from_string(body).with_status_code(if code==0 {200} else {404}));
    }
    Ok(())
}
