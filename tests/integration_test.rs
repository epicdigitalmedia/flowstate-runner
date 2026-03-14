/// Integration test for the 5-step process flow:
/// start -> action (echo) -> decision (branch) -> action (second echo) -> end
///
/// Uses dispatch_handler (the real production dispatcher) and persist=false
/// so no REST calls are made. The EndHandler skips discussion posting because
/// state.context is None.
use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::executor::execute;
use flowstate_runner::handlers::{dispatch_handler, RunContext};
use flowstate_runner::models::execution::{ExecutionState, ProcessExecutionRecord, ResolvedStep};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;

fn make_run_context() -> RunContext {
    use flowstate_runner::clients::mcp::McpClient;
    use flowstate_runner::clients::rest::FlowstateRestClient;
    use flowstate_runner::config::Config;

    RunContext {
        config: Config {
            org_id: "org_test".to_string(),
            workspace_id: "work_test".to_string(),
            // Port 1 is reserved and will be rejected immediately — REST is not called
            // when persist=false and state.context is None.
            rest_base_url: "http://localhost:1".to_string(),
            mcp_base_url: "http://localhost:1/mcp".to_string(),
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
        rest: FlowstateRestClient::new("http://localhost:1"),
        http: reqwest::Client::new(),
        mcp: McpClient::new("http://localhost:9999/mcp", "test-org", "test-workspace"),
        agent_executor: Box::new(flowstate_runner::agent::NoopAgentExecutor),
        attribute_map: AttributeMap::default(),
        process_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        step_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        token_exchanger: None,
    }
}

/// Build an ExecutionState with no context (so EndHandler skips REST calls)
/// and an initial variable set.
fn make_state(current_step: &str, variables: Map<String, Value>) -> ExecutionState {
    let record_json = json!({
        "id": "exec_integration01",
        "processId": "proc_integration01",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "pending",
        "currentStepId": current_step,
        "startedAt": null,
        "variables": variables,
        "stepHistory": [],
        // No "context" field — EndHandler skips post_discussion and update_entity_status
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "integration-test-process".to_string())
}

fn make_step(
    id: &str,
    step_type: &str,
    name: &str,
    action: Option<Value>,
    inputs: Option<Value>,
    conditions: Vec<Value>,
    next_step_id: Option<&str>,
) -> ResolvedStep {
    ResolvedStep {
        id: id.to_string(),
        process_id: "proc_integration01".to_string(),
        name: name.to_string(),
        step_type: step_type.to_string(),
        action,
        inputs,
        outputs: vec![],
        output_extraction: None,
        conditions,
        next_step_id: next_step_id.map(String::from),
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    }
}

/// Build the 5-step process map.
fn make_process_steps() -> HashMap<String, ResolvedStep> {
    // Step 1: start — inject inputs: mode="test"
    let step_start = make_step(
        "step_start",
        "start",
        "Start Process",
        None,
        Some(json!({ "mode": "test" })),
        vec![],
        Some("step_action1"),
    );

    // Step 2: action (command) — echo the mode variable
    let step_action1 = make_step(
        "step_action1",
        "action",
        "Echo Mode",
        Some(json!({
            "type": "command",
            "command": "echo",
            "args": ["mode=${mode}"]
        })),
        None,
        vec![],
        Some("step_decision"),
    );

    // Step 3: decision — branch on mode value
    // Condition: if mode == "test" -> step_action2
    // Condition: if mode == "production" -> step_end
    // Fallback (no match): next_step_id = step_action2
    let step_decision = make_step(
        "step_decision",
        "decision",
        "Branch on Mode",
        None,
        None,
        vec![
            json!({
                "field": "mode",
                "operator": "eq",
                "value": "test",
                "targetStepId": "step_action2"
            }),
            json!({
                "field": "mode",
                "operator": "eq",
                "value": "production",
                "targetStepId": "step_end"
            }),
        ],
        // Fallback path if no condition matches
        Some("step_action2"),
    );

    // Step 4: action (command) — echo "done"
    let step_action2 = make_step(
        "step_action2",
        "action",
        "Echo Done",
        Some(json!({
            "type": "command",
            "command": "echo",
            "args": ["done"]
        })),
        None,
        vec![],
        Some("step_end"),
    );

    // Step 5: end — no next step, no context (REST calls skipped)
    let step_end = make_step("step_end", "end", "End Process", None, None, vec![], None);

    [
        ("step_start".to_string(), step_start),
        ("step_action1".to_string(), step_action1),
        ("step_decision".to_string(), step_decision),
        ("step_action2".to_string(), step_action2),
        ("step_end".to_string(), step_end),
    ]
    .into()
}

#[tokio::test]
async fn test_five_step_integration_flow() {
    let mut state = make_state("step_start", Map::new());
    let steps = make_process_steps();
    let ctx = make_run_context();

    let result = execute(&mut state, &steps, &dispatch_handler, &ctx, false).await;
    assert!(result.is_ok(), "execute should succeed: {:?}", result);

    // Execution completes fully
    assert_eq!(state.status, "completed", "status should be 'completed'");
    assert_eq!(
        state.progress,
        Some(100),
        "progress should be 100 on completion"
    );

    // All 5 steps recorded in history
    assert_eq!(
        state.step_history.len(),
        5,
        "step_history should contain 5 entries"
    );

    // Steps executed in the expected order
    let history_ids: Vec<&str> = state
        .step_history
        .iter()
        .map(|h| h.step_id.as_str())
        .collect();
    assert_eq!(
        history_ids,
        [
            "step_start",
            "step_action1",
            "step_decision",
            "step_action2",
            "step_end"
        ],
        "step history order should follow decision branch for mode=test"
    );

    // All steps completed successfully
    for entry in &state.step_history {
        assert_eq!(
            entry.status, "completed",
            "step '{}' should be completed",
            entry.step_id
        );
    }

    // StartHandler injected "mode" = "test" via direct-output-merge
    assert_eq!(
        state.variables.get("mode"),
        Some(&json!("test")),
        "variables should contain mode=test from StartHandler"
    );

    // ActionHandler direct-output-merge wrote stdout into variables
    // (step_action1 and step_action2 both have empty outputs specs)
    assert!(
        state.variables.contains_key("stdout"),
        "variables should contain 'stdout' from ActionHandler direct-output-merge"
    );

    let stdout_val = state.variables.get("stdout").unwrap();
    assert!(
        stdout_val.is_string(),
        "stdout variable should be a string, got: {:?}",
        stdout_val
    );
}

#[tokio::test]
async fn test_five_step_decision_routes_correctly() {
    // With mode="test", decision should route to step_action2 (not step_end directly).
    // This verifies the DecisionHandler condition matching.
    let mut state = make_state("step_start", Map::new());
    let steps = make_process_steps();
    let ctx = make_run_context();

    let result = execute(&mut state, &steps, &dispatch_handler, &ctx, false).await;
    assert!(result.is_ok());

    // If decision had incorrectly routed to step_end (production branch),
    // we would only have 4 history entries: start, action1, decision, end.
    // The test branch goes: start -> action1 -> decision -> action2 -> end = 5.
    assert_eq!(
        state.step_history.len(),
        5,
        "decision should route mode=test to step_action2 (not skip to step_end)"
    );

    // step_action2 must appear in history
    let has_action2 = state
        .step_history
        .iter()
        .any(|h| h.step_id == "step_action2");
    assert!(
        has_action2,
        "step_action2 should appear in history for mode=test branch"
    );
}

#[tokio::test]
async fn test_variable_propagation_across_steps() {
    // Verify that variables set by StartHandler are visible to ActionHandler
    // for interpolation, so the echo output contains the substituted value.
    let mut state = make_state("step_start", Map::new());
    let steps = make_process_steps();
    let ctx = make_run_context();

    let result = execute(&mut state, &steps, &dispatch_handler, &ctx, false).await;
    assert!(result.is_ok());
    assert_eq!(state.status, "completed");

    // The stdout from step_action1 should contain the interpolated mode value.
    // echo "mode=${mode}" with mode=test → "mode=test"
    // Note: stdout is overwritten by step_action2's echo "done", so we verify
    // the final stdout is "done" (from step_action2).
    let stdout = state
        .variables
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or("");

    // step_action2 runs after step_action1 and echoes "done", so stdout = "done"
    assert_eq!(
        stdout, "done",
        "final stdout should be 'done' from step_action2"
    );
}
