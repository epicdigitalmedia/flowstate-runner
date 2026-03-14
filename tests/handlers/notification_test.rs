use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::notification::NotificationHandler;
use flowstate_runner::handlers::{Handler, RunContext};
use flowstate_runner::models::execution::{
    ExecutionState, ProcessExecutionRecord, ResolvedStep, StepOutcome,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_run_context_with_url(base_url: &str) -> RunContext {
    RunContext {
        config: Config {
            org_id: "org_test".to_string(),
            workspace_id: "work_test".to_string(),
            rest_base_url: base_url.to_string(),
            mcp_base_url: format!("{}/mcp", base_url),
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
        rest: FlowstateRestClient::new(base_url),
        http: reqwest::Client::new(),
        mcp: McpClient::new(&format!("{}/mcp", base_url), "test-org", "test-workspace"),
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

fn make_state_with_context() -> ExecutionState {
    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_notify",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": { "project_name": "Test Project" },
        "stepHistory": [],
        "context": {
            "entityType": "task",
            "entityId": "task_test12345",
            "userId": "user_test"
        },
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "test-process".to_string())
}

fn make_notification_step(inputs: Option<Value>) -> ResolvedStep {
    ResolvedStep {
        id: "step_notify".to_string(),
        process_id: "proc_test".to_string(),
        name: "Notify".to_string(),
        step_type: "notification".to_string(),
        action: None,
        inputs,
        outputs: vec![],
        output_extraction: None,
        conditions: vec![],
        next_step_id: Some("step_next".to_string()),
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    }
}

#[tokio::test]
async fn test_notification_posts_discussion() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp/tools/collection-create"))
        .and(body_string_contains("Process started for Test Project"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&mock_server)
        .await;

    let handler = NotificationHandler;
    let step = make_notification_step(Some(json!({
        "content": "Process started for ${project_name}"
    })));
    let state = make_state_with_context();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert!(outputs.is_empty() || outputs.contains_key("result"));
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_notification_succeeds_even_on_rest_failure() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp/tools/collection-create"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let handler = NotificationHandler;
    let step = make_notification_step(Some(json!({
        "content": "This will fail to post"
    })));
    let state = make_state_with_context();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { .. } => {}
        other => panic!("Expected Completed even on REST failure, got {:?}", other),
    }
}

#[tokio::test]
async fn test_notification_no_context_still_completes() {
    let handler = NotificationHandler;
    let step = make_notification_step(Some(json!({ "content": "Hello" })));

    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_notify",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": {},
        "stepHistory": [],
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    let state = ExecutionState::from_record(record, "test-process".to_string());

    let ctx = make_run_context_with_url("http://localhost:99999");

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { .. } => {}
        other => panic!("Expected Completed, got {:?}", other),
    }
}
