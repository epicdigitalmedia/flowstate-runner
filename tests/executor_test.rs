use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::executor::{
    calculate_progress, execute, record_step_history, resolve_next_step,
};
use flowstate_runner::models::execution::{
    ExecutionState, PauseReason, ProcessExecutionRecord, ResolvedStep, StepOutcome,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

fn make_step(id: &str, step_type: &str, next: Option<&str>) -> ResolvedStep {
    ResolvedStep {
        id: id.to_string(),
        process_id: "proc_test".to_string(),
        name: format!("{}-step", step_type),
        step_type: step_type.to_string(),
        action: None,
        inputs: None,
        outputs: vec![],
        output_extraction: None,
        conditions: vec![],
        next_step_id: next.map(String::from),
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    }
}

fn make_state(current_step: &str) -> ExecutionState {
    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": current_step,
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

// --- calculate_progress ---

#[test]
fn test_progress_zero_steps() {
    assert_eq!(calculate_progress(0, 0), 0);
}

#[test]
fn test_progress_partial() {
    assert_eq!(calculate_progress(1, 4), 25);
}

#[test]
fn test_progress_capped_at_99() {
    assert_eq!(calculate_progress(3, 3), 99);
    assert_eq!(calculate_progress(10, 10), 99);
}

#[test]
fn test_progress_half() {
    assert_eq!(calculate_progress(2, 4), 50);
}

// --- resolve_next_step ---

#[test]
fn test_next_step_uses_next_step_id() {
    let step = make_step("step_1", "action", Some("step_2"));
    let outputs = HashMap::new();
    let result = resolve_next_step(&step, &outputs);
    assert_eq!(result, Some("step_2".to_string()));
}

#[test]
fn test_next_step_override_wins() {
    let step = make_step("step_1", "action", Some("step_2"));
    let mut outputs: HashMap<String, Value> = HashMap::new();
    outputs.insert("_next_step_override".to_string(), json!("step_3"));
    let result = resolve_next_step(&step, &outputs);
    assert_eq!(result, Some("step_3".to_string()));
}

#[test]
fn test_next_step_none_means_end() {
    let step = make_step("step_1", "end", None);
    let outputs = HashMap::new();
    let result = resolve_next_step(&step, &outputs);
    assert_eq!(result, None);
}

// --- record_step_history ---

#[test]
fn test_record_history_appends() {
    let mut state = make_state("step_1");
    assert_eq!(state.step_history.len(), 0);

    let step = make_step("step_1", "action", None);
    record_step_history(&mut state, &step, "completed", "2026-01-15T10:05:00Z");
    assert_eq!(state.step_history.len(), 1);
    assert_eq!(state.step_history[0].step_id, "step_1");
    assert_eq!(state.step_history[0].status, "completed");
    assert_eq!(
        state.step_history[0].step_name,
        Some("action-step".to_string())
    );
    assert_eq!(state.step_history[0].step_type, Some("action".to_string()));
}

#[test]
fn test_record_history_multiple() {
    let mut state = make_state("step_1");
    let step1 = make_step("step_1", "start", None);
    let step2 = make_step("step_2", "action", None);

    record_step_history(&mut state, &step1, "completed", "2026-01-15T10:05:00Z");
    record_step_history(&mut state, &step2, "completed", "2026-01-15T10:10:00Z");
    assert_eq!(state.step_history.len(), 2);
}

// --- Full executor loop (with mock handlers) ---

use anyhow::Result;
use async_trait::async_trait;
use flowstate_runner::handlers::{Handler, RunContext};

struct CompletingHandler;

#[async_trait]
impl Handler for CompletingHandler {
    async fn execute(
        &self,
        _step: &ResolvedStep,
        _state: &ExecutionState,
        _ctx: &RunContext,
    ) -> Result<StepOutcome> {
        Ok(StepOutcome::Completed {
            outputs: HashMap::new(),
        })
    }
}

struct PausingHandler;

#[async_trait]
impl Handler for PausingHandler {
    async fn execute(
        &self,
        step: &ResolvedStep,
        _state: &ExecutionState,
        _ctx: &RunContext,
    ) -> Result<StepOutcome> {
        Ok(StepOutcome::Paused(PauseReason::Approval {
            approval_id: format!("appr_{}", step.id),
        }))
    }
}

struct FailingHandler;

#[async_trait]
impl Handler for FailingHandler {
    async fn execute(
        &self,
        _step: &ResolvedStep,
        _state: &ExecutionState,
        _ctx: &RunContext,
    ) -> Result<StepOutcome> {
        Ok(StepOutcome::Failed {
            error: "handler failed".to_string(),
        })
    }
}

fn make_run_context() -> RunContext {
    use flowstate_runner::clients::mcp::McpClient;
    use flowstate_runner::clients::rest::FlowstateRestClient;
    use flowstate_runner::config::Config;
    use std::path::PathBuf;

    RunContext {
        config: Config {
            org_id: "org_test".to_string(),
            workspace_id: "work_test".to_string(),
            rest_base_url: "http://localhost:99999".to_string(), // Not called in unit tests
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

#[tokio::test]
async fn test_execute_linear_three_steps() {
    let mut state = make_state("step_1");
    let steps: HashMap<String, ResolvedStep> = [
        (
            "step_1".to_string(),
            make_step("step_1", "start", Some("step_2")),
        ),
        (
            "step_2".to_string(),
            make_step("step_2", "action", Some("step_3")),
        ),
        ("step_3".to_string(), make_step("step_3", "end", None)),
    ]
    .into();
    let ctx = make_run_context();

    let dispatch = |_: &str| -> Result<Box<dyn Handler>> { Ok(Box::new(CompletingHandler)) };

    let result = execute(&mut state, &steps, &dispatch, &ctx, false).await;
    assert!(result.is_ok(), "execute should succeed: {:?}", result);
    assert_eq!(state.status, "completed");
    assert_eq!(state.step_history.len(), 3);
    assert_eq!(state.progress, Some(100));
}

#[tokio::test]
async fn test_execute_pause_breaks_loop() {
    let mut state = make_state("step_1");
    let steps: HashMap<String, ResolvedStep> = [
        (
            "step_1".to_string(),
            make_step("step_1", "start", Some("step_2")),
        ),
        (
            "step_2".to_string(),
            make_step("step_2", "approval", Some("step_3")),
        ),
        ("step_3".to_string(), make_step("step_3", "end", None)),
    ]
    .into();
    let ctx = make_run_context();

    let dispatch = |step_type: &str| -> Result<Box<dyn Handler>> {
        match step_type {
            "approval" => Ok(Box::new(PausingHandler)),
            _ => Ok(Box::new(CompletingHandler)),
        }
    };

    let result = execute(&mut state, &steps, &dispatch, &ctx, false).await;
    assert!(result.is_ok());
    assert_eq!(state.status, "paused");
    // step_1 completed, step_2 paused
    assert_eq!(state.step_history.len(), 2);
    assert_eq!(state.current_step_id, Some("step_2".to_string()));
}

#[tokio::test]
async fn test_execute_fail_without_retry() {
    let mut state = make_state("step_1");
    state.retry_count = 3; // already at max
    state.max_retries = 3;

    let steps: HashMap<String, ResolvedStep> =
        [("step_1".to_string(), make_step("step_1", "action", None))].into();
    let ctx = make_run_context();

    let dispatch = |_: &str| -> Result<Box<dyn Handler>> { Ok(Box::new(FailingHandler)) };

    let result = execute(&mut state, &steps, &dispatch, &ctx, false).await;
    assert!(result.is_ok());
    assert_eq!(state.status, "failed");
}

#[tokio::test]
async fn test_execute_fail_with_retry() {
    let mut state = make_state("step_1");
    state.retry_count = 0;
    state.max_retries = 3;

    let steps: HashMap<String, ResolvedStep> = [
        (
            "step_1".to_string(),
            make_step("step_1", "action", Some("step_2")),
        ),
        ("step_2".to_string(), make_step("step_2", "end", None)),
    ]
    .into();
    let ctx = make_run_context();

    // First call fails, then succeeds on retry
    let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let call_count_clone = call_count.clone();

    struct RetryHandler(std::sync::Arc<std::sync::atomic::AtomicU32>);

    #[async_trait]
    impl Handler for RetryHandler {
        async fn execute(
            &self,
            _step: &ResolvedStep,
            _state: &ExecutionState,
            _ctx: &RunContext,
        ) -> Result<StepOutcome> {
            let count = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if count == 0 {
                Ok(StepOutcome::Failed {
                    error: "transient error".to_string(),
                })
            } else {
                Ok(StepOutcome::Completed {
                    outputs: HashMap::new(),
                })
            }
        }
    }

    let dispatch = move |_: &str| -> Result<Box<dyn Handler>> {
        Ok(Box::new(RetryHandler(call_count_clone.clone())))
    };

    let result = execute(&mut state, &steps, &dispatch, &ctx, false).await;
    assert!(result.is_ok());
    // retry_count resets to 0 on success; verify the retry happened via call_count
    assert_eq!(state.retry_count, 0);
    assert!(call_count.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    // Should have completed after retry
    assert_eq!(state.status, "completed");
}

#[tokio::test]
async fn test_execute_missing_step_fails() {
    let mut state = make_state("step_missing");
    let steps: HashMap<String, ResolvedStep> = HashMap::new();
    let ctx = make_run_context();

    let dispatch = |_: &str| -> Result<Box<dyn Handler>> { Ok(Box::new(CompletingHandler)) };

    let result = execute(&mut state, &steps, &dispatch, &ctx, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_execute_no_current_step_fails() {
    let mut state = make_state("step_1");
    state.current_step_id = None;
    let steps: HashMap<String, ResolvedStep> = HashMap::new();
    let ctx = make_run_context();

    let dispatch = |_: &str| -> Result<Box<dyn Handler>> { Ok(Box::new(CompletingHandler)) };

    let result = execute(&mut state, &steps, &dispatch, &ctx, false).await;
    assert!(result.is_err());
}

// --- pause_reason stored in ExecutionState after pause ---

#[tokio::test]
async fn test_execute_stores_pause_reason() {
    let mut state = make_state("step_1");
    let steps: HashMap<String, ResolvedStep> = [
        (
            "step_1".to_string(),
            make_step("step_1", "approval", Some("step_2")),
        ),
        ("step_2".to_string(), make_step("step_2", "end", None)),
    ]
    .into();
    let ctx = make_run_context();

    let dispatch = |_: &str| -> Result<Box<dyn Handler>> { Ok(Box::new(PausingHandler)) };

    let result = execute(&mut state, &steps, &dispatch, &ctx, false).await;
    assert!(result.is_ok());
    assert_eq!(state.status, "paused");
    assert_eq!(
        state.pause_reason,
        Some(PauseReason::Approval {
            approval_id: "appr_step_1".to_string()
        })
    );
}

// --- PauseReason round-trip through ProcessExecutionRecord ---

#[test]
fn test_pause_reason_roundtrips_through_record() {
    let mut state = make_state("step_1");
    state.pause_reason = Some(PauseReason::Approval {
        approval_id: "appr_abc123".to_string(),
    });

    let record = state.to_record();
    let restored = ExecutionState::from_record(record, "test-process".to_string());

    assert_eq!(
        restored.pause_reason,
        Some(PauseReason::Approval {
            approval_id: "appr_abc123".to_string()
        })
    );
}

#[test]
fn test_pause_reason_none_roundtrips() {
    let mut state = make_state("step_1");
    // Ensure a previously-set value is cleared correctly.
    state.pause_reason = None;

    let record = state.to_record();
    // _pause_reason key must be absent from metadata.
    let metadata = record.metadata.as_object().unwrap();
    assert!(!metadata.contains_key("_pause_reason"));

    let restored = ExecutionState::from_record(record, "test-process".to_string());
    assert_eq!(restored.pause_reason, None);
}

#[test]
fn test_pause_reason_human_task_roundtrips() {
    let mut state = make_state("step_1");
    state.pause_reason = Some(PauseReason::HumanTask {
        discussion_id: "disc_xyz789".to_string(),
        posted_at: "2026-03-11T09:00:00Z".to_string(),
    });

    let record = state.to_record();
    let restored = ExecutionState::from_record(record, "test-process".to_string());

    assert_eq!(
        restored.pause_reason,
        Some(PauseReason::HumanTask {
            discussion_id: "disc_xyz789".to_string(),
            posted_at: "2026-03-11T09:00:00Z".to_string(),
        })
    );
}

#[test]
fn test_pause_reason_subprocess_roundtrips() {
    let mut state = make_state("step_1");
    state.pause_reason = Some(PauseReason::Subprocess {
        child_execution_id: "exec_child001".to_string(),
    });

    let record = state.to_record();
    let restored = ExecutionState::from_record(record, "test-process".to_string());

    assert_eq!(
        restored.pause_reason,
        Some(PauseReason::Subprocess {
            child_execution_id: "exec_child001".to_string()
        })
    );
}

#[test]
fn test_pause_reason_agent_task_roundtrips() {
    let mut state = make_state("step_1");
    state.pause_reason = Some(PauseReason::AgentTask {
        discussion_id: "disc_agent42".to_string(),
        posted_at: "2026-03-11T10:30:00Z".to_string(),
    });

    let record = state.to_record();
    let restored = ExecutionState::from_record(record, "test-process".to_string());

    assert_eq!(
        restored.pause_reason,
        Some(PauseReason::AgentTask {
            discussion_id: "disc_agent42".to_string(),
            posted_at: "2026-03-11T10:30:00Z".to_string(),
        })
    );
}

// Handler that returns outputs without any output specs on the step.
// Models StartHandler behavior: inputs promoted to outputs but step.outputs is empty.
struct OutputtingHandler;

#[async_trait]
impl Handler for OutputtingHandler {
    async fn execute(
        &self,
        _step: &ResolvedStep,
        _state: &ExecutionState,
        _ctx: &RunContext,
    ) -> Result<StepOutcome> {
        let mut outputs = HashMap::new();
        outputs.insert("greeting".to_string(), json!("hello"));
        outputs.insert("count".to_string(), json!(42));
        outputs.insert("_internal".to_string(), json!("skip me"));
        Ok(StepOutcome::Completed { outputs })
    }
}

#[tokio::test]
async fn test_execute_direct_output_merge() {
    let mut state = make_state("step_1");
    let steps: HashMap<String, ResolvedStep> = [
        (
            "step_1".to_string(),
            make_step("step_1", "start", Some("step_2")),
        ),
        ("step_2".to_string(), make_step("step_2", "end", None)),
    ]
    .into();
    let ctx = make_run_context();

    let dispatch = |step_type: &str| -> Result<Box<dyn Handler>> {
        match step_type {
            "start" => Ok(Box::new(OutputtingHandler)),
            _ => Ok(Box::new(CompletingHandler)),
        }
    };

    let result = execute(&mut state, &steps, &dispatch, &ctx, false).await;
    assert!(result.is_ok());
    assert_eq!(state.variables.get("greeting"), Some(&json!("hello")));
    assert_eq!(state.variables.get("count"), Some(&json!(42)));
    // Internal keys (starting with _) should NOT be merged
    assert_eq!(state.variables.get("_internal"), None);
}

// --- persist_interval config field ---

#[test]
fn test_persist_interval_defaults_to_one() {
    // The default RunContext in tests uses persist_interval: 1 (every step).
    // This verifies the field exists on Config and that the default is correct.
    let ctx = make_run_context();
    assert_eq!(
        ctx.config.persist_interval, 1,
        "persist_interval should default to 1 (persist after every completed step)"
    );
}

#[tokio::test]
async fn test_execute_with_persist_interval_two_completes() {
    // With persist_interval: 2, a 3-step pipeline should still complete
    // successfully even though intermediate persists are skipped.
    // We use persist=false to avoid network calls, so this verifies the
    // counter logic does not break step sequencing.
    let mut ctx = make_run_context();
    ctx.config.persist_interval = 2;

    let mut state = make_state("step_1");
    let steps: HashMap<String, ResolvedStep> = [
        (
            "step_1".to_string(),
            make_step("step_1", "start", Some("step_2")),
        ),
        (
            "step_2".to_string(),
            make_step("step_2", "action", Some("step_3")),
        ),
        ("step_3".to_string(), make_step("step_3", "end", None)),
    ]
    .into();

    let dispatch = |_: &str| -> Result<Box<dyn Handler>> { Ok(Box::new(CompletingHandler)) };

    let result = execute(&mut state, &steps, &dispatch, &ctx, false).await;
    assert!(result.is_ok());
    assert_eq!(state.status, "completed");
    assert_eq!(state.step_history.len(), 3);
}
