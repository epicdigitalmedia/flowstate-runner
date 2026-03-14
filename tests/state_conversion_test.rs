use flowstate_runner::models::execution::*;
use flowstate_runner::state::compute_plan_dir;
use serde_json::json;

#[test]
fn test_to_record_preserves_core_fields() {
    let record_json = json!({
        "id": "exec_exec123abc",
        "processId": "proc_abc123xyz0",
        "processVersion": "1.0.0",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "userId": "user_abc123",
        "status": "running",
        "progress": 35,
        "startedAt": "2026-01-15T10:05:00Z",
        "currentStepId": "step_step456def",
        "variables": { "taskId": "task_task123" },
        "stepHistory": [],
        "externalId": "task_task123",
        "parentExecutionId": "exec_parent00",
        "depth": 1,
        "retryCount": 0,
        "maxRetries": 3,
        "inputs": { "inputVar": "hello" },
        "archived": false,
        "metadata": { "source": "test" },
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:30:00Z"
    });

    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    let state = ExecutionState::from_record(record, "brainstorm-planning".to_string());
    let roundtripped = state.to_record();

    assert_eq!(roundtripped.id, "exec_exec123abc");
    assert_eq!(roundtripped.process_id, "proc_abc123xyz0");
    assert_eq!(roundtripped.org_id, "org_9f3omFEY2H");
    assert_eq!(roundtripped.workspace_id, "work_ojk4TWK5D2");
    assert_eq!(roundtripped.status, "running");
    assert_eq!(roundtripped.progress, Some(35));
    assert_eq!(
        roundtripped.current_step_id,
        Some("step_step456def".to_string())
    );
    assert_eq!(roundtripped.variables["taskId"], "task_task123");
    assert_eq!(roundtripped.external_id, Some("task_task123".to_string()));
    assert_eq!(
        roundtripped.parent_execution_id,
        Some("exec_parent00".to_string())
    );
    assert_eq!(roundtripped.depth, Some(1));
    assert_eq!(roundtripped.retry_count, 0);
    assert_eq!(roundtripped.max_retries, 3);
    assert_eq!(roundtripped.user_id, Some("user_abc123".to_string()));
    assert_eq!(roundtripped.inputs, Some(json!({ "inputVar": "hello" })));
    assert_eq!(roundtripped.created_at, "2026-01-15T10:00:00Z");
    // updatedAt should be refreshed to approximately now — check that it starts
    // with the current year rather than comparing exact timestamps (which is
    // fragile across clock skew and CI environments).
    let current_year = chrono::Utc::now().format("%Y").to_string();
    assert!(
        roundtripped.updated_at.starts_with(&current_year),
        "updatedAt should be set to current year, got: {}",
        roundtripped.updated_at
    );
    assert!(
        roundtripped.updated_at.as_str() >= "2026-01-15T10:30:00Z",
        "updatedAt should be newer than the original value"
    );
}

#[test]
fn test_to_record_and_back_preserves_plan_dir() {
    let record_json = json!({
        "id": "exec_roundtrip1",
        "processId": "proc_abc123xyz0",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "status": "running",
        "variables": {},
        "stepHistory": [],
        "externalId": "task_abc123",
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:30:00Z"
    });

    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    let state = ExecutionState::from_record(record, "brainstorm-planning".to_string());

    // plan_dir is computed, not stored in record — verify it survives round-trip
    let plan_dir_before = compute_plan_dir("/base/plans", state.external_id.as_deref());
    let roundtripped = state.to_record();
    let state2 = ExecutionState::from_record(roundtripped, "brainstorm-planning".to_string());
    let plan_dir_after = compute_plan_dir("/base/plans", state2.external_id.as_deref());
    assert_eq!(
        plan_dir_before, plan_dir_after,
        "plan_dir should be recomputable after round-trip"
    );
    assert_eq!(plan_dir_after, Some("/base/plans/task_abc123".to_string()));
}

#[test]
fn test_to_record_metadata_object() {
    let record_json = json!({
        "id": "exec_test001abc",
        "processId": "proc_test001abc",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "status": "pending",
        "variables": {},
        "stepHistory": [],
        "archived": false,
        "metadata": { "key": "val" },
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });

    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    let state = ExecutionState::from_record(record, "test".to_string());
    let out = state.to_record();

    assert!(out.metadata.is_object());
    assert_eq!(out.metadata["key"], "val");
}

#[test]
fn test_compute_plan_dir_with_external_id() {
    let dir = compute_plan_dir("/base/plans", Some("task_abc123"));
    assert_eq!(dir, Some("/base/plans/task_abc123".to_string()));
}

#[test]
fn test_compute_plan_dir_without_external_id() {
    let dir = compute_plan_dir("/base/plans", None);
    assert_eq!(dir, None);
}

#[test]
fn test_compute_plan_dir_empty_external_id() {
    let dir = compute_plan_dir("/base/plans", Some(""));
    assert_eq!(dir, None);
}

#[test]
fn test_compute_plan_dir_trailing_slash() {
    let dir = compute_plan_dir("/base/plans/", Some("task_abc123"));
    assert_eq!(
        dir,
        Some("/base/plans/task_abc123".to_string()),
        "trailing slash should be normalized"
    );
}

#[test]
fn test_compute_plan_dir_relative_path() {
    let dir = compute_plan_dir("./plans", Some("task_abc123"));
    assert_eq!(dir, Some("./plans/task_abc123".to_string()));
}
