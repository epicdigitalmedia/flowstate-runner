use flowstate_runner::config::Config;
use flowstate_runner::context::{build_run_context, init_mcp_with_retry, load_templates};
use serde_json::json;
use std::path::PathBuf;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_load_templates_returns_hashmap() {
    let mock_server = MockServer::start().await;

    // steptemplates is a VCA collection — queried via MCP collection-query
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {
                    "id": "stpl_tmpl1",
                    "name": "Echo Template",
                    "stepType": "action",
                    "action": { "type": "command", "command": "echo" },
                    "orgId": "org_test",
                    "workspaceId": "work_test",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z"
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let mcp =
        flowstate_runner::clients::mcp::McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let templates = load_templates(&mcp).await.unwrap();

    assert_eq!(templates.len(), 1);
    assert!(templates.contains_key("stpl_tmpl1"));
    assert_eq!(templates["stpl_tmpl1"].name, "Echo Template");
}

#[tokio::test]
async fn test_load_templates_empty_returns_empty_map() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": []
        })))
        .mount(&mock_server)
        .await;

    let mcp =
        flowstate_runner::clients::mcp::McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let templates = load_templates(&mcp).await.unwrap();

    assert!(templates.is_empty());
}

#[tokio::test]
async fn test_load_templates_propagates_rest_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let mcp =
        flowstate_runner::clients::mcp::McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let result = load_templates(&mcp).await;

    assert!(result.is_err(), "Should propagate MCP error");
}

#[tokio::test]
async fn test_init_mcp_retry_succeeds_on_second_attempt() {
    let mock_server = MockServer::start().await;

    // First call returns 503, second returns 200
    Mock::given(method("POST"))
        .and(path_regex(r"tools/set-context"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path_regex(r"tools/set-context"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({}))
                .append_header("mcp-session-id", "sess_test123"),
        )
        .mount(&mock_server)
        .await;

    let mut mcp =
        flowstate_runner::clients::mcp::McpClient::new(&mock_server.uri(), "org_test", "work_test");

    let result = init_mcp_with_retry(&mut mcp, 3, 10).await;
    assert!(result.is_ok(), "Should succeed after retry: {:?}", result);
    assert!(mcp.session_id().is_some());
}

#[tokio::test]
async fn test_init_mcp_retry_exhausted_returns_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"tools/set-context"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_server)
        .await;

    let mut mcp =
        flowstate_runner::clients::mcp::McpClient::new(&mock_server.uri(), "org_test", "work_test");

    let result = init_mcp_with_retry(&mut mcp, 2, 10).await;
    assert!(result.is_err(), "Should fail after retries exhausted");
}

#[tokio::test]
async fn test_build_run_context_wires_all_dependencies() {
    let mock_server = MockServer::start().await;

    // MCP set-context
    Mock::given(method("POST"))
        .and(path_regex(r"tools/set-context"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({}))
                .append_header("mcp-session-id", "sess_ctx123"),
        )
        .mount(&mock_server)
        .await;

    // Schema loading for REST client VCA awareness
    Mock::given(method("POST"))
        .and(path_regex(r"schemas-rest/.*/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                { "id": "schm_stpl", "name": "steptemplates", "orgId": "org_test" }
            ]
        })))
        .mount(&mock_server)
        .await;

    // Templates query — steptemplates is a VCA collection, routed through MCP
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": []
        })))
        .mount(&mock_server)
        .await;

    // Attributes query
    Mock::given(method("POST"))
        .and(path_regex(r"attributes-rest/.*/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": []
        })))
        .mount(&mock_server)
        .await;

    let config = Config {
        org_id: "org_test".to_string(),
        workspace_id: "work_test".to_string(),
        rest_base_url: mock_server.uri(),
        mcp_base_url: mock_server.uri(),
        obs_url: None,
        plan_base_dir: PathBuf::from("/tmp/plans"),
        worker_mode: false,
        health_port: 9090,
        max_subprocess_depth: 5,
        agent_executor: "claude-cli".to_string(),
        auth_token: None,
        api_token: None,
        auth_url: None,
        persist_interval: 1,
    };

    let (ctx, templates) = build_run_context(config).await.unwrap();

    assert_eq!(ctx.config.org_id, "org_test");
    assert!(templates.is_empty());
}

#[tokio::test]
async fn test_build_run_context_fails_when_mcp_unreachable() {
    let mock_server = MockServer::start().await;

    // MCP set-context always returns 503
    Mock::given(method("POST"))
        .and(path_regex(r"tools/set-context"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_server)
        .await;

    let config = Config {
        org_id: "org_test".to_string(),
        workspace_id: "work_test".to_string(),
        rest_base_url: mock_server.uri(),
        mcp_base_url: mock_server.uri(),
        obs_url: None,
        plan_base_dir: PathBuf::from("/tmp/plans"),
        worker_mode: false,
        health_port: 9090,
        max_subprocess_depth: 5,
        agent_executor: "claude-cli".to_string(),
        auth_token: None,
        api_token: None,
        auth_url: None,
        persist_interval: 1,
    };

    let result = build_run_context(config).await;
    assert!(result.is_err(), "Should fail when MCP is unreachable");
}
