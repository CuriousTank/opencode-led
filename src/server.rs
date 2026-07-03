use crate::store::{HeartbeatUpdate, RemoveUpdate, StatusUpdate, Store};
use std::sync::Arc;

pub fn spawn(
    store: Arc<Store>,
    port: u16,
    on_change: Box<dyn Fn() + Send + Sync + 'static>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let addr = format!("127.0.0.1:{}", port);
        let server = match tiny_http::Server::http(addr) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[traffic-light] failed to bind 127.0.0.1:{}: {}", port, e);
                eprintln!("    another monitor instance may be running, or set a different port via $OPENCODE_TL_PORT");
                std::process::exit(1);
            }
        };
        eprintln!("[traffic-light] listening on http://127.0.0.1:{} (plugin pushes here)", port);

        for request in server.incoming_requests() {
            handle(&store, request, &on_change);
        }
    })
}

fn handle(
    store: &Arc<Store>,
    mut req: tiny_http::Request,
    on_change: &dyn Fn(),
) {
    let url = req.url().to_string();
    let mut body = String::new();
    let _ = req.as_reader().read_to_string(&mut body);

    let changed = match (req.method().as_str(), url.as_str()) {
        ("POST", "/status") => serde_json::from_str::<StatusUpdate>(&body)
            .map(|u| {
                if u.session_id.is_empty() {
                    false
                } else {
                    store.set(u)
                }
            })
            .unwrap_or(false),
        ("POST", "/remove") => serde_json::from_str::<RemoveUpdate>(&body)
            .map(|u| store.remove(&u.session_id))
            .unwrap_or(false),
        ("POST", "/heartbeat") => serde_json::from_str::<HeartbeatUpdate>(&body)
            .map(|u| {
                if u.session_id.is_empty() {
                    false
                } else {
                    store.heartbeat(&u.session_id)
                }
            })
            .unwrap_or(false),
        ("GET", "/health") => {
            let resp = tiny_http::Response::from_string("ok");
            let _ = req.respond(resp);
            return;
        }
        _ => {
            let _ = req.respond(tiny_http::Response::empty(404));
            return;
        }
    };

    let code = if changed { 200 } else { 204 };
    let _ = req.respond(tiny_http::Response::empty(code));
    if changed {
        on_change();
    }
}
