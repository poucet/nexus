//! REST frontend over the nexus-tools registry, plus the human debug page.
//!
//! This is the first (and for now only) transport — the instrument for the
//! manual investigation loop. It is deliberately thin: `GET /tools` lists the
//! registry's descriptors, `POST /tools/{name}` invokes one, `GET /debug`
//! serves the single-file debug UI. The MCP frontend later consumes the same
//! registry.
//!
//! Server shape (proven in the sibling droplets project): its own OS thread
//! with its own tokio runtime, loopback only, bound on BOTH 127.0.0.1 and
//! [::1] — macOS resolves `localhost` to ::1 first, so a v4-only bind reads
//! as "connection refused" in a browser.

use std::future::IntoFuture;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use nexus_tools::{ErrorCode, TimeoutClass, ToolError, ToolRegistry};
use serde_json::{Value, json};

pub const DEFAULT_PORT: u16 = 9996;

/// Port from `NEXUS_REST_PORT`, falling back to [`DEFAULT_PORT`].
pub fn port_from_env() -> u16 {
    std::env::var("NEXUS_REST_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT)
}

static STARTED: OnceLock<()> = OnceLock::new();

/// Start the server on a background thread (idempotent — later calls no-op).
pub fn start_server(registry: Arc<ToolRegistry>, port: u16) {
    STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("nexus-rest".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("build tokio runtime for nexus-rest");
                runtime.block_on(run(registry, port));
            })
            .expect("spawn nexus-rest thread");
    });
}

async fn run(registry: Arc<ToolRegistry>, port: u16) {
    let app = Router::new()
        .route("/", get(|| async { Redirect::temporary("/debug") }))
        .route("/debug", get(debug_page))
        .route("/tools", get(list_tools))
        .route("/tools/:name", post(invoke_tool))
        .with_state(registry);

    let v4 = tokio::net::TcpListener::bind(("127.0.0.1", port)).await;
    let v6 = tokio::net::TcpListener::bind(("::1", port)).await;
    match (v4, v6) {
        (Ok(v4), Ok(v6)) => {
            log::info!("nexus-rest: debug UI on http://127.0.0.1:{port}/debug");
            let _ = tokio::join!(
                axum::serve(v4, app.clone()).into_future(),
                axum::serve(v6, app).into_future()
            );
        }
        (Ok(v4), Err(e)) => {
            log::warn!("nexus-rest: [::1]:{port} bind failed ({e}); serving IPv4 only");
            log::info!("nexus-rest: debug UI on http://127.0.0.1:{port}/debug");
            let _ = axum::serve(v4, app).await;
        }
        (Err(e4), Ok(v6)) => {
            log::warn!("nexus-rest: 127.0.0.1:{port} bind failed ({e4}); serving IPv6 only");
            let _ = axum::serve(v6, app).await;
        }
        (Err(e4), Err(e6)) => {
            log::error!("nexus-rest: could not bind port {port} (v4: {e4}, v6: {e6}) — is another nexus running? Set NEXUS_REST_PORT to change it.");
        }
    }
}

async fn debug_page() -> Html<&'static str> {
    Html(include_str!("debug.html"))
}

async fn list_tools(State(registry): State<Arc<ToolRegistry>>) -> Response {
    (StatusCode::OK, axum::Json(json!({ "tools": registry.descriptors() }))).into_response()
}

async fn invoke_tool(
    State(registry): State<Arc<ToolRegistry>>,
    Path(name): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(def) = registry.get(&name) else {
        return error_response(ToolError::not_found(format!(
            "no tool named {name:?} — GET /tools lists them"
        )));
    };
    // Tolerant body handling: empty = {}, anything else must be JSON.
    let input: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(e) => return error_response(ToolError::invalid_input(format!("body is not JSON: {e}"))),
        }
    };
    let timeout = match def.timeout_class {
        TimeoutClass::Default => Duration::from_secs(5),
        TimeoutClass::Load => Duration::from_secs(30),
    };
    match tokio::time::timeout(timeout, def.invoke(input)).await {
        Ok(Ok(value)) => (StatusCode::OK, axum::Json(value)).into_response(),
        Ok(Err(error)) => error_response(error),
        Err(_) => error_response(ToolError::internal(format!(
            "tool {name:?} timed out after {}s — the app's main thread may be busy (plugin load?)",
            timeout.as_secs()
        ))),
    }
}

fn error_response(error: ToolError) -> Response {
    let status = match error.code {
        ErrorCode::InvalidInput | ErrorCode::InvalidInstance => StatusCode::BAD_REQUEST,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Conflict | ErrorCode::NoPlugin => StatusCode::CONFLICT,
        ErrorCode::Unsupported => StatusCode::UNPROCESSABLE_ENTITY,
        ErrorCode::Plugin | ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, axum::Json(json!({ "error": error }))).into_response()
}
