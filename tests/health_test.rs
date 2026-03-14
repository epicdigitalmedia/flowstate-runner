// tests/health_test.rs
use axum::http::StatusCode;
use std::time::Instant;

#[tokio::test]
async fn test_health_endpoint_returns_ok() {
    use tower::ServiceExt;
    let start_time = Instant::now();
    let app = flowstate_runner::health::health_router(start_time);
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/health")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ok");
    assert!(json["uptime_secs"].is_number());
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn test_health_uptime_increases() {
    use tower::ServiceExt;
    let start_time = Instant::now();

    // Wait a tiny bit so uptime is non-zero
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let app = flowstate_runner::health::health_router(start_time);
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/health")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let uptime = json["uptime_secs"].as_f64().unwrap();
    assert!(uptime >= 0.0, "uptime should be non-negative");
}
