use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::delay::DelayHandler;
use flowstate_runner::handlers::{Handler, RunContext};
use flowstate_runner::models::execution::{
    ExecutionState, ProcessExecutionRecord, ResolvedStep, StepOutcome,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::time::Instant;

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
        "currentStepId": "step_delay",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": {},
        "stepHistory": [],
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "test-process".to_string())
}

fn make_delay_step(action: Option<Value>) -> ResolvedStep {
    ResolvedStep {
        id: "step_delay".to_string(),
        process_id: "proc_test".to_string(),
        name: "Wait".to_string(),
        step_type: "delay".to_string(),
        action,
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

#[tokio::test]
async fn test_delay_waits() {
    let handler = DelayHandler;
    let step = make_delay_step(Some(json!({ "duration": 0.05 })));
    let state = make_state();
    let ctx = make_run_context();

    let start = Instant::now();
    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    let elapsed = start.elapsed();

    match result {
        StepOutcome::Completed { outputs } => {
            assert!(outputs.is_empty());
            assert!(
                elapsed.as_millis() >= 40,
                "Should have waited ~50ms, waited {}ms",
                elapsed.as_millis()
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_delay_integer_duration() {
    let handler = DelayHandler;
    let step = make_delay_step(Some(json!({ "duration": 0 })));
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
async fn test_delay_missing_duration_fails() {
    let handler = DelayHandler;
    let step = make_delay_step(Some(json!({ "type": "delay" })));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.contains("duration"),
                "Error should mention duration: {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_delay_no_action_fails() {
    let handler = DelayHandler;
    let step = make_delay_step(None);
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.contains("action"),
                "Error should mention missing action: {}",
                error
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}
