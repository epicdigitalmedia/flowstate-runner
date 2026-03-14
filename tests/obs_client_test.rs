use flowstate_runner::clients::obs::{ObsClient, ObsEntry};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_obs_send_fires_request() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/logs"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = ObsClient::new(Some(mock_server.uri()));

    client.send(ObsEntry {
        level: "info".to_string(),
        message: "test log".to_string(),
        context: json!({
            "processId": "proc_test",
            "executionId": "exec_test"
        }),
    });

    // Give the fire-and-forget task time to complete
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // wiremock's expect(1) will verify the request was made
}

#[tokio::test]
async fn test_obs_disabled_when_no_url() {
    // Should not panic or error when obs_url is None
    let client = ObsClient::new(None);

    client.send(ObsEntry {
        level: "info".to_string(),
        message: "this goes nowhere".to_string(),
        context: json!({}),
    });

    // No crash = success
}
