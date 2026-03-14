use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::handlers::{dispatch_handler, RunContext};
use flowstate_runner::models::execution::{ExecutionState, ResolvedStep, StepOutcome};
use serde_json::{json, Map};

fn make_run_context() -> RunContext {
    use flowstate_runner::clients::mcp::McpClient;
    use flowstate_runner::clients::rest::FlowstateRestClient;
    use flowstate_runner::config::Config;
    use std::path::PathBuf;

    RunContext {
        config: Config {
            org_id: "org_test".to_string(),
            workspace_id: "work_test".to_string(),
            rest_base_url: "http://localhost:7080".to_string(),
            mcp_base_url: "http://localhost:7080/mcp".to_string(),
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
        rest: FlowstateRestClient::new("http://localhost:7080"),
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

fn make_resolved_step(step_type: &str) -> ResolvedStep {
    ResolvedStep {
        id: "step_test".to_string(),
        process_id: "proc_test".to_string(),
        name: "test-step".to_string(),
        step_type: step_type.to_string(),
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

fn make_execution_state() -> ExecutionState {
    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "variables": {},
        "stepHistory": [],
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z"
    });
    let record = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "test-process".to_string())
}

#[test]
fn test_dispatch_known_types() {
    let known = [
        "start",
        "end",
        "action",
        "script",
        "api-call",
        "decision",
        "delay",
        "notification",
        "agent-task",
        "approval",
        "human-task",
        "subprocess",
        "parallel-gateway",
        "join-gateway",
    ];
    for step_type in known {
        let result = dispatch_handler(step_type);
        assert!(
            result.is_ok(),
            "dispatch_handler('{}') should succeed",
            step_type
        );
    }
}

#[test]
fn test_dispatch_unknown_type_fails() {
    let result = dispatch_handler("nonexistent");
    assert!(result.is_err());
    let err_msg = result.err().unwrap().to_string();
    assert!(
        err_msg.contains("nonexistent"),
        "error should mention the step type"
    );
}

#[tokio::test]
async fn test_stub_handler_returns_completed() {
    let handler = dispatch_handler("start").unwrap();
    let step = make_resolved_step("start");
    let state = make_execution_state();
    let ctx = make_run_context();

    let outcome = handler.execute(&step, &state, &ctx).await.unwrap();
    assert!(matches!(outcome, StepOutcome::Completed { .. }));
}

#[tokio::test]
async fn test_stub_check_resume_returns_none() {
    use flowstate_runner::models::execution::PauseReason;

    // ApprovalHandler is now a real handler, not a stub.
    // A non-Approval PauseReason should return None immediately without any
    // network call — confirm the dispatch path routes through the real handler.
    let handler = dispatch_handler("approval").unwrap();
    let step = make_resolved_step("approval");
    let state = make_execution_state();
    let ctx = make_run_context();
    let reason = PauseReason::Subprocess {
        child_execution_id: "exec_child".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "Non-Approval pause reason should return None without network call"
    );
}
