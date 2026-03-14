use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::decision::DecisionHandler;
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

fn make_state_with_vars(vars: serde_json::Map<String, Value>) -> ExecutionState {
    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_decision",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": vars,
        "stepHistory": [],
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "test-process".to_string())
}

fn make_decision_step(conditions: Vec<Value>, fallback_next: Option<&str>) -> ResolvedStep {
    ResolvedStep {
        id: "step_decision".to_string(),
        process_id: "proc_test".to_string(),
        name: "Decision".to_string(),
        step_type: "decision".to_string(),
        action: None,
        inputs: None,
        outputs: vec![],
        output_extraction: None,
        conditions,
        next_step_id: fallback_next.map(String::from),
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    }
}

#[tokio::test]
async fn test_decision_first_matching_condition() {
    let handler = DecisionHandler;
    let conditions = vec![
        json!({ "field": "status", "operator": "equals", "value": "approved", "targetStepId": "step_approved" }),
        json!({ "field": "status", "operator": "equals", "value": "rejected", "targetStepId": "step_rejected" }),
    ];
    let step = make_decision_step(conditions, Some("step_default"));
    let mut vars = Map::new();
    vars.insert("status".to_string(), json!("approved"));
    let state = make_state_with_vars(vars);
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(
                outputs.get("_next_step_override"),
                Some(&json!("step_approved"))
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_decision_second_condition_matches() {
    let handler = DecisionHandler;
    let conditions = vec![
        json!({ "field": "status", "operator": "equals", "value": "approved", "targetStepId": "step_approved" }),
        json!({ "field": "status", "operator": "equals", "value": "rejected", "targetStepId": "step_rejected" }),
    ];
    let step = make_decision_step(conditions, Some("step_default"));
    let mut vars = Map::new();
    vars.insert("status".to_string(), json!("rejected"));
    let state = make_state_with_vars(vars);
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(
                outputs.get("_next_step_override"),
                Some(&json!("step_rejected"))
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_decision_no_match_uses_fallback() {
    let handler = DecisionHandler;
    let conditions = vec![
        json!({ "field": "status", "operator": "equals", "value": "approved", "targetStepId": "step_approved" }),
    ];
    let step = make_decision_step(conditions, Some("step_default"));
    let mut vars = Map::new();
    vars.insert("status".to_string(), json!("pending"));
    let state = make_state_with_vars(vars);
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(outputs.get("_next_step_override"), None);
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_decision_empty_conditions_uses_fallback() {
    let handler = DecisionHandler;
    let step = make_decision_step(vec![], Some("step_default"));
    let state = make_state_with_vars(Map::new());
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(outputs.get("_next_step_override"), None);
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_decision_with_value_from() {
    let handler = DecisionHandler;
    let conditions = vec![
        json!({ "field": "score", "operator": "gte", "value": 0, "valueFrom": "threshold", "targetStepId": "step_pass" }),
    ];
    let step = make_decision_step(conditions, Some("step_fail"));
    let mut vars = Map::new();
    vars.insert("score".to_string(), json!(85));
    vars.insert("threshold".to_string(), json!(70));
    let state = make_state_with_vars(vars);
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(
                outputs.get("_next_step_override"),
                Some(&json!("step_pass"))
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}
