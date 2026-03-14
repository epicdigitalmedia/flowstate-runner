use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::end::EndHandler;
use flowstate_runner::handlers::{Handler, RunContext};
use flowstate_runner::models::execution::{
    ExecutionState, ProcessExecutionRecord, ResolvedStep, StepOutcome,
};
use serde_json::{json, Map};
use std::path::PathBuf;
use wiremock::matchers::{body_string_contains, method, path, path_regex};
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
        process_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        step_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        token_exchanger: None,
    }
}

fn make_state_with_history() -> ExecutionState {
    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_end",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": { "result": "success" },
        "stepHistory": [
            {
                "stepId": "step_1",
                "stepName": "Start",
                "stepType": "start",
                "status": "completed",
                "startedAt": "2026-01-15T10:00:00Z",
                "completedAt": "2026-01-15T10:00:01Z"
            },
            {
                "stepId": "step_2",
                "stepName": "Action",
                "stepType": "action",
                "status": "completed",
                "startedAt": "2026-01-15T10:00:01Z",
                "completedAt": "2026-01-15T10:00:05Z"
            }
        ],
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

fn make_end_step() -> ResolvedStep {
    ResolvedStep {
        id: "step_end".to_string(),
        process_id: "proc_test".to_string(),
        name: "End".to_string(),
        step_type: "end".to_string(),
        action: None,
        inputs: None,
        outputs: vec![],
        output_extraction: None,
        conditions: vec![],
        next_step_id: None,
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    }
}

#[tokio::test]
async fn test_end_returns_metrics() {
    let handler = EndHandler;
    let step = make_end_step();
    let state = make_state_with_history();
    let ctx = make_run_context_with_url("http://localhost:99999");

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(outputs.get("_total_steps"), Some(&json!(2)));
            assert!(outputs.contains_key("_completed_steps"));
            assert!(
                outputs.contains_key("_elapsed_ms"),
                "Should compute elapsed time"
            );
            let elapsed = outputs.get("_elapsed_ms").unwrap().as_i64().unwrap();
            assert!(elapsed > 0, "Elapsed ms should be positive");
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_end_posts_discussion() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp/tools/collection-create"))
        .and(body_string_contains("\"collection\":\"discussions\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&mock_server)
        .await;

    let handler = EndHandler;
    let step = make_end_step();
    let state = make_state_with_history();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    assert!(matches!(result, StepOutcome::Completed { .. }));
}

#[tokio::test]
async fn test_end_updates_entity_status() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex(r"/tasks-rest/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "task_test12345",
            "title": "Test Task",
            "status": "In Progress",
            "orgId": "org_test",
            "workspaceId": "work_test",
            "createdAt": "2026-01-15T10:00:00Z",
            "updatedAt": "2026-01-15T10:00:00Z"
        })))
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path_regex(r".*-rest/.*/set"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&mock_server)
        .await;

    let handler = EndHandler;
    let step = make_end_step();
    let state = make_state_with_history();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    assert!(matches!(result, StepOutcome::Completed { .. }));
}

#[tokio::test]
async fn test_end_no_context_still_completes() {
    let handler = EndHandler;
    let step = make_end_step();

    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_end",
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
        StepOutcome::Completed { outputs } => {
            assert_eq!(outputs.get("_total_steps"), Some(&json!(0)));
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}
