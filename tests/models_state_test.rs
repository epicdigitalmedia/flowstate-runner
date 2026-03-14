use flowstate_runner::models::execution::*;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_step_outcome_variants() {
    let completed = StepOutcome::Completed {
        outputs: HashMap::from([("result".to_string(), json!("ok"))]),
    };
    assert!(matches!(completed, StepOutcome::Completed { .. }));

    let paused = StepOutcome::Paused(PauseReason::Approval {
        approval_id: "appr_test".to_string(),
    });
    assert!(matches!(paused, StepOutcome::Paused(_)));

    let failed = StepOutcome::Failed {
        error: "timeout".to_string(),
    };
    assert!(matches!(failed, StepOutcome::Failed { .. }));
}

#[test]
fn test_execution_state_from_record() {
    let record_json = json!({
        "id": "exec_exec123abc",
        "processId": "proc_abc123xyz0",
        "processVersion": "1.0.0",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "status": "running",
        "progress": 35,
        "startedAt": "2026-01-15T10:05:00Z",
        "currentStepId": "step_step456def",
        "variables": { "taskId": "task_task123" },
        "stepHistory": [],
        "externalId": "task_task123",
        "retryCount": 0,
        "maxRetries": 3,
        "archived": false,
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:30:00Z"
    });

    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    let state = ExecutionState::from_record(record, "brainstorm-planning".to_string());

    assert_eq!(state.id, "exec_exec123abc");
    assert_eq!(state.process_id, "proc_abc123xyz0");
    assert_eq!(state.process_name, "brainstorm-planning");
    assert_eq!(state.current_step_id, Some("step_step456def".to_string()));
    assert_eq!(state.variables["taskId"], "task_task123");
    assert_eq!(state.retry_count, 0);
    assert_eq!(state.max_retries, 3);
}

#[test]
fn test_resolved_step_basic() {
    let step = ResolvedStep {
        id: "step_test".to_string(),
        process_id: "proc_test".to_string(),
        name: "collect-ideas".to_string(),
        step_type: "action".to_string(),
        action: Some(json!({ "type": "command", "command": { "executable": "echo" } })),
        inputs: Some(json!({ "prompt": "hello" })),
        outputs: vec![json!({ "name": "result", "source": "commandOutput" })],
        output_extraction: None,
        conditions: vec![],
        next_step_id: Some("step_next".to_string()),
        required_variables: vec!["prompt".to_string()],
        estimated_duration_minutes: None,
        metadata: serde_json::Map::new(),
    };

    assert_eq!(step.id, "step_test");
    assert_eq!(step.step_type, "action");
    assert!(step.next_step_id.is_some());
}
