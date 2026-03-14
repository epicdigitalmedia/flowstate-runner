use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::start::StartHandler;
use flowstate_runner::handlers::{Handler, RunContext};
use flowstate_runner::models::execution::{
    ExecutionState, ProcessExecutionRecord, ResolvedStep, StepOutcome,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

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
        process_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        step_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
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
        "currentStepId": "step_start",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": { "existing_var": "keep_me" },
        "stepHistory": [],
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "test-process".to_string())
}

fn make_start_step(inputs: Option<Value>) -> ResolvedStep {
    ResolvedStep {
        id: "step_start".to_string(),
        process_id: "proc_test".to_string(),
        name: "Start".to_string(),
        step_type: "start".to_string(),
        action: None,
        inputs,
        outputs: vec![],
        output_extraction: None,
        conditions: vec![],
        next_step_id: Some("step_2".to_string()),
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    }
}

#[tokio::test]
async fn test_start_no_inputs() {
    let handler = StartHandler;
    let step = make_start_step(None);
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert!(outputs.is_empty());
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_start_merges_inputs() {
    let handler = StartHandler;
    let step = make_start_step(Some(json!({
        "project_name": "Test Project",
        "max_retries": 5
    })));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(outputs.get("project_name"), Some(&json!("Test Project")));
            assert_eq!(outputs.get("max_retries"), Some(&json!(5)));
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_start_interpolates_variables() {
    let handler = StartHandler;
    let step = make_start_step(Some(json!({
        "greeting": "Hello ${existing_var}",
        "full_ref": "${existing_var}"
    })));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(outputs.get("greeting"), Some(&json!("Hello keep_me")));
            assert_eq!(outputs.get("full_ref"), Some(&json!("keep_me")));
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_start_non_object_inputs_ignored() {
    let handler = StartHandler;
    let step = make_start_step(Some(json!("not an object")));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert!(outputs.is_empty());
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}
