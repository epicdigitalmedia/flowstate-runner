// src/health.rs
use axum::{extract::State, routing::get, Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

/// State shared with the health endpoint handler.
struct HealthState {
    start_time: Instant,
}

/// Create an axum Router with the `/health` endpoint.
///
/// The returned router can be served directly or composed with other routes.
/// `start_time` is captured at process start to calculate uptime.
pub fn health_router(start_time: Instant) -> Router {
    let state = Arc::new(HealthState { start_time });
    Router::new()
        .route("/health", get(health_handler))
        .with_state(state)
}

async fn health_handler(State(state): State<Arc<HealthState>>) -> Json<Value> {
    let uptime = state.start_time.elapsed().as_secs_f64();
    Json(json!({
        "status": "ok",
        "uptime_secs": uptime,
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Spawn the health server in the background on the given port.
///
/// Returns a `JoinHandle` that resolves when the server shuts down.
/// The server binds to `0.0.0.0:{port}` for Docker compatibility.
pub async fn spawn_health_server(
    port: u16,
    start_time: Instant,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let app = health_router(start_time);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    tracing::info!(port, "Health server listening");

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "Health server error");
        }
    });

    Ok(handle)
}
