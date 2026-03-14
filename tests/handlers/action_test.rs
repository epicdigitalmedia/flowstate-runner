use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::action::ActionHandler;
use flowstate_runner::handlers::{Handler, RunContext};
use flowstate_runner::models::execution::{
    ExecutionState, ProcessExecutionRecord, ResolvedStep, StepOutcome,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_run_context() -> RunContext {
    RunContext {
        config: Config {
            org_id: "org_test".to_string(),
            workspace_id: "work_test".to_string(),
            rest_base_url: "http://localhost:99999".to_string(),
            mcp_base_url: "http://localhost:99999/mcp".to_string(),
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
        },
        rest: FlowstateRestClient::new("http://localhost:99999"),
        http: reqwest::Client::new(),
        mcp: McpClient::new("http://localhost:9999/mcp", "test-org", "test-workspace"),
        agent_executor: Box::new(flowstate_runner::agent::NoopAgentExecutor),
        attribute_map: AttributeMap::default(),
        process_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(
            std::time::Duration::from_secs(60),
        )),
        step_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(
            std::time::Duration::from_secs(60),
        )),
        token_exchanger: None,
    }
}

fn make_state() -> ExecutionState {
    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_action",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": { "name": "world" },
        "stepHistory": [],
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "test-process".to_string())
}

fn make_action_step(action: Value) -> ResolvedStep {
    ResolvedStep {
        id: "step_action".to_string(),
        process_id: "proc_test".to_string(),
        name: "Run Action".to_string(),
        step_type: "action".to_string(),
        action: Some(action),
        inputs: None,
        outputs: vec![],
        output_extraction: None,
        conditions: vec![],
        next_step_id: Some("step_next".to_string()),
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    }
}

// --- Command sub-type tests ---

#[tokio::test]
async fn test_action_command_echo() {
    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "command",
        "command": "echo",
        "args": ["hello"]
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            let stdout = outputs.get("stdout").and_then(Value::as_str).unwrap_or("");
            assert!(
                stdout.contains("hello"),
                "stdout should contain 'hello', got: {:?}",
                stdout
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_command_nonzero_exit_fails() {
    let handler = ActionHandler;
    // Use sh -c "exit 42" to produce a nonzero exit
    let step = make_action_step(json!({
        "type": "command",
        "command": "sh",
        "args": ["-c", "exit 42"]
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.contains("42"),
                "Error should mention exit code 42: {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_command_with_interpolated_args() {
    let handler = ActionHandler;
    // state has name="world"; interpolate into the arg
    let step = make_action_step(json!({
        "type": "command",
        "command": "echo",
        "args": ["hello ${name}"]
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            let stdout = outputs.get("stdout").and_then(Value::as_str).unwrap_or("");
            assert!(
                stdout.contains("hello world"),
                "stdout should contain 'hello world' after interpolation, got: {:?}",
                stdout
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_command_captures_stderr() {
    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "command",
        "command": "sh",
        "args": ["-c", "echo error_text >&2"]
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            let stderr = outputs.get("stderr").and_then(Value::as_str).unwrap_or("");
            assert!(
                stderr.contains("error_text"),
                "stderr should contain 'error_text', got: {:?}",
                stderr
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

// --- Script sub-type tests ---

#[tokio::test]
async fn test_action_script_inline() {
    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "script",
        "script": "#!/bin/sh\necho script_output"
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            let stdout = outputs.get("stdout").and_then(Value::as_str).unwrap_or("");
            assert!(
                stdout.contains("script_output"),
                "stdout should contain 'script_output', got: {:?}",
                stdout
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_script_with_interpolation() {
    let handler = ActionHandler;
    // The script content itself gets interpolated before writing to file
    let step = make_action_step(json!({
        "type": "script",
        "script": "#!/bin/sh\necho hello ${name}"
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            let stdout = outputs.get("stdout").and_then(Value::as_str).unwrap_or("");
            assert!(
                stdout.contains("hello world"),
                "stdout should contain 'hello world', got: {:?}",
                stdout
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_script_failure() {
    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "script",
        "script": "#!/bin/sh\nexit 1"
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                !error.is_empty(),
                "Failed outcome should have a non-empty error"
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

// --- Error cases ---

#[tokio::test]
async fn test_action_missing_type_fails() {
    let handler = ActionHandler;
    // action exists but has no "type" field
    let step = make_action_step(json!({
        "command": "echo",
        "args": ["hello"]
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.to_lowercase().contains("type"),
                "Error should mention 'type': {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_no_action_fails() {
    let handler = ActionHandler;
    let step = ResolvedStep {
        id: "step_action".to_string(),
        process_id: "proc_test".to_string(),
        name: "Run Action".to_string(),
        step_type: "action".to_string(),
        action: None,
        inputs: None,
        outputs: vec![],
        output_extraction: None,
        conditions: vec![],
        next_step_id: None,
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    };
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.to_lowercase().contains("action"),
                "Error should mention 'action': {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

// --- HTTP sub-type tests ---

fn make_run_context_with_mcp_url(mcp_url: &str) -> RunContext {
    RunContext {
        config: Config {
            org_id: "org_test".to_string(),
            workspace_id: "work_test".to_string(),
            rest_base_url: "http://localhost:99999".to_string(),
            mcp_base_url: mcp_url.to_string(),
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
        },
        rest: FlowstateRestClient::new("http://localhost:99999"),
        http: reqwest::Client::new(),
        mcp: McpClient::new("http://localhost:9999/mcp", "test-org", "test-workspace"),
        agent_executor: Box::new(flowstate_runner::agent::NoopAgentExecutor),
        attribute_map: AttributeMap::default(),
        process_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(
            std::time::Duration::from_secs(60),
        )),
        step_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(
            std::time::Duration::from_secs(60),
        )),
        token_exchanger: None,
    }
}

#[tokio::test]
async fn test_action_http_get() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/data"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"key":"value"}"#)
                .insert_header("content-type", "application/json"),
        )
        .mount(&mock_server)
        .await;

    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "http",
        "method": "GET",
        "url": format!("{}/api/data", mock_server.uri())
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            let status = outputs.get("status").and_then(Value::as_u64).unwrap();
            assert_eq!(status, 200, "status should be 200");

            // HTTP handler stores body as string; parse it to verify JSON content
            let body_str = outputs.get("body").and_then(Value::as_str).unwrap();
            let json_body: Value = serde_json::from_str(body_str).unwrap();
            assert_eq!(
                json_body.get("key"),
                Some(&json!("value")),
                "parsed body should contain key=value"
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_http_post_with_body() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/submit"))
        .and(body_string_contains("world"))
        .respond_with(ResponseTemplate::new(201).set_body_string(r#"{"created":true}"#))
        .mount(&mock_server)
        .await;

    let handler = ActionHandler;
    // state has name="world"; body gets interpolated before sending
    let step = make_action_step(json!({
        "type": "http",
        "method": "POST",
        "url": format!("{}/api/submit", mock_server.uri()),
        "body": { "name": "${name}" }
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            let status = outputs.get("status").and_then(Value::as_u64).unwrap();
            assert_eq!(status, 201, "status should be 201");
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_http_error_status() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/broken"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&mock_server)
        .await;

    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "http",
        "method": "GET",
        "url": format!("{}/api/broken", mock_server.uri())
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.contains("500"),
                "Error should mention status 500: {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_http_missing_url_fails() {
    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "http",
        "method": "GET"
        // no "url" field
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.to_lowercase().contains("url"),
                "Error should mention 'url': {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_unknown_type_fails() {
    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "unknown"
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.to_lowercase().contains("unknown"),
                "Error should mention 'unknown': {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

// --- MCP-tool sub-type tests ---

#[tokio::test]
async fn test_action_mcp_tool_missing_tool_fails() {
    let handler = ActionHandler;
    // "toolName" is wrong; the implementation checks for "tool"
    let step = make_action_step(json!({
        "type": "mcp-tool",
        "toolName": "collection-query"
    }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.to_lowercase().contains("tool"),
                "Error should mention 'tool': {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_action_mcp_tool_connection_failure() {
    let handler = ActionHandler;
    // Point at a port nothing is listening on
    let step = make_action_step(json!({
        "type": "mcp-tool",
        "tool": "collection-query",
        "args": { "collection": "tasks" }
    }));
    let state = make_state();
    // Use an unreachable MCP URL — set_context will fail immediately
    let ctx = make_run_context_with_mcp_url("http://127.0.0.1:19999/mcp");

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.to_lowercase().contains("mcp"),
                "Error should mention 'MCP': {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

// --- responseMapping tests ---

#[tokio::test]
async fn test_mcp_tool_with_response_mapping() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/mcp/tools/collection-query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {"id": "task_001", "title": "First task"}
            ],
            "total": 1
        })))
        .mount(&mock_server)
        .await;

    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "mcp-tool",
        "tool": "collection-query",
        "args": { "collection": "tasks", "selector": {} },
        "responseMapping": {
            "firstTaskId": "$.documents[0].id",
            "totalCount": "$.total"
        }
    }));
    let state = make_state();
    let mut ctx = make_run_context_with_mcp_url(&format!("{}/mcp", mock_server.uri()));
    ctx.mcp = McpClient::new(
        &format!("{}/mcp", mock_server.uri()),
        "test-org",
        "test-workspace",
    );

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(
                outputs.get("firstTaskId"),
                Some(&json!("task_001")),
                "firstTaskId should be mapped from documents[0].id"
            );
            assert_eq!(
                outputs.get("totalCount"),
                Some(&json!(1)),
                "totalCount should be mapped from total"
            );
            assert!(
                outputs.contains_key("result"),
                "raw result must always be present"
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_mcp_tool_response_mapping_nested_path() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/mcp/tools/collection-get"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "task_abc",
                "metadata": {
                    "priority": "high"
                }
            }
        })))
        .mount(&mock_server)
        .await;

    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "mcp-tool",
        "tool": "collection-get",
        "args": { "collection": "tasks", "id": "task_abc" },
        "responseMapping": {
            "docId": "$.document.id",
            "priority": "$.document.metadata.priority"
        }
    }));
    let state = make_state();
    let mut ctx = make_run_context_with_mcp_url(&format!("{}/mcp", mock_server.uri()));
    ctx.mcp = McpClient::new(
        &format!("{}/mcp", mock_server.uri()),
        "test-org",
        "test-workspace",
    );

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(
                outputs.get("docId"),
                Some(&json!("task_abc")),
                "docId should be extracted from nested path document.id"
            );
            assert_eq!(
                outputs.get("priority"),
                Some(&json!("high")),
                "priority should be extracted from document.metadata.priority"
            );
            assert!(outputs.contains_key("result"), "raw result must be present");
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_mcp_tool_response_mapping_missing_path() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/mcp/tools/collection-query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [],
            "total": 0
        })))
        .mount(&mock_server)
        .await;

    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "mcp-tool",
        "tool": "collection-query",
        "args": { "collection": "tasks", "selector": {} },
        "responseMapping": {
            "firstTaskId": "$.documents[0].id",
            "nonExistentField": "$.does.not.exist"
        }
    }));
    let state = make_state();
    let mut ctx = make_run_context_with_mcp_url(&format!("{}/mcp", mock_server.uri()));
    ctx.mcp = McpClient::new(
        &format!("{}/mcp", mock_server.uri()),
        "test-org",
        "test-workspace",
    );

    // Should complete without error even when paths resolve to nothing
    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert!(
                !outputs.contains_key("firstTaskId"),
                "missing path should be silently skipped, not inserted as null"
            );
            assert!(
                !outputs.contains_key("nonExistentField"),
                "non-existent path should be silently skipped"
            );
            assert!(outputs.contains_key("result"), "raw result must be present");
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_mcp_tool_backward_compat() {
    // No responseMapping — raw response must land in outputs["result"]
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/mcp/tools/collection-query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{"id": "task_zzz"}],
            "total": 1
        })))
        .mount(&mock_server)
        .await;

    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "mcp-tool",
        "tool": "collection-query",
        "args": { "collection": "tasks", "selector": {} }
        // no responseMapping field
    }));
    let state = make_state();
    let mut ctx = make_run_context_with_mcp_url(&format!("{}/mcp", mock_server.uri()));
    ctx.mcp = McpClient::new(
        &format!("{}/mcp", mock_server.uri()),
        "test-org",
        "test-workspace",
    );

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            let result_val = outputs.get("result").expect("result key must exist");
            assert_eq!(
                result_val.get("total"),
                Some(&json!(1)),
                "raw response should be intact in result"
            );
            // Only the raw result key — no extra mapped keys
            assert_eq!(
                outputs.len(),
                1,
                "without responseMapping only 'result' should be in outputs"
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

// --- mcpUrl routing tests ---

#[tokio::test]
async fn test_mcp_tool_with_external_url() {
    // Verify that when mcpUrl is set, the request goes to that server
    // (not to ctx.mcp, which points at an unreachable port).
    let external_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/tools/analyze"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "analysis": "done", "score": 95 })),
        )
        .mount(&external_server)
        .await;

    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "mcp-tool",
        "mcpUrl": external_server.uri(),
        "tool": "analyze",
        "args": { "input": "test data" },
        "responseMapping": {
            "analysisResult": "$.analysis",
            "confidence": "$.score"
        }
    }));
    let state = make_state();
    // ctx.mcp points at an unreachable port — proves we routed to external_server
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(
                outputs.get("analysisResult"),
                Some(&json!("done")),
                "analysisResult should be mapped from external response"
            );
            assert_eq!(
                outputs.get("confidence"),
                Some(&json!(95)),
                "confidence should be mapped from external response"
            );
            assert!(
                outputs.contains_key("result"),
                "raw result must always be present"
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_mcp_tool_with_external_url_interpolation() {
    // Verify that ${varName} references inside mcpUrl are resolved
    // against the execution state variables before the request is sent.
    let external_server = MockServer::start().await;
    let port = external_server.address().port();

    Mock::given(method("POST"))
        .and(path("/tools/greet"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "greeting": "hello world" })),
        )
        .mount(&external_server)
        .await;

    // Inject the port as a variable so we can reference it in mcpUrl
    let mut state = make_state();
    state
        .variables
        .insert("servicePort".to_string(), json!(port.to_string()));

    let handler = ActionHandler;
    let step = make_action_step(json!({
        "type": "mcp-tool",
        "mcpUrl": format!("http://127.0.0.1:{}", "${servicePort}"),
        "tool": "greet",
        "args": {}
    }));
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(
                outputs.get("result").and_then(|v| v.get("greeting")),
                Some(&json!("hello world")),
                "should receive greeting from interpolated external server URL"
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}
