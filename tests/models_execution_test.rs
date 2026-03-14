use flowstate_runner::models::execution::*;
use serde_json::json;

#[test]
fn test_execution_status_serialize() {
    let status = ExecutionStatus::Paused {
        step_id: "step_abc".to_string(),
        reason: PauseReason::Approval {
            approval_id: "appr_xyz".to_string(),
        },
    };
    let json = serde_json::to_value(&status).unwrap();
    let deserialized: ExecutionStatus = serde_json::from_value(json).unwrap();
    assert_eq!(status, deserialized);
}

#[test]
fn test_step_history_entry_deserialize() {
    let json = json!({
        "stepId": "step_step123abc",
        "stepName": "collect-ideas",
        "stepType": "action",
        "status": "completed",
        "startedAt": "2026-01-15T10:05:00Z",
        "completedAt": "2026-01-15T10:10:00Z",
        "durationMs": 300000,
        "retryCount": 0
    });

    let entry: StepHistoryEntry = serde_json::from_value(json).unwrap();
    assert_eq!(entry.step_id, "step_step123abc");
    assert_eq!(entry.status, "completed");
    assert_eq!(entry.duration_ms, Some(300000));
}

#[test]
fn test_execution_context_deserialize() {
    let json = json!({
        "entityType": "task",
        "entityId": "task_task123",
        "userId": "use__user123id",
        "tags": ["feature-request", "backlog"],
        "category": "planning",
        "depth": 0,
        "maxDepth": 5
    });

    let ctx: ExecutionContext = serde_json::from_value(json).unwrap();
    assert_eq!(ctx.entity_type, "task");
    assert_eq!(ctx.depth, 0);
    assert_eq!(ctx.max_depth, 5);
    assert_eq!(ctx.tags.len(), 2);
}

#[test]
fn test_process_execution_record_deserialize() {
    let json = json!({
        "id": "exec_exec123abc",
        "processId": "proc_abc123xyz0",
        "processVersion": "1.0.0",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "userId": "use__user123id",
        "status": "running",
        "progress": 35,
        "startedAt": "2026-01-15T10:05:00Z",
        "currentStepId": "step_step456def",
        "variables": {
            "taskId": "task_task123",
            "taskTitle": "Brainstorm New Features"
        },
        "stepHistory": [
            {
                "stepId": "step_step123abc",
                "stepName": "start",
                "stepType": "start",
                "status": "completed",
                "startedAt": "2026-01-15T10:05:00Z",
                "completedAt": "2026-01-15T10:05:01Z",
                "durationMs": 1000,
                "retryCount": 0
            }
        ],
        "context": {
            "entityType": "task",
            "entityId": "task_task123",
            "depth": 0,
            "maxDepth": 5
        },
        "externalId": "task_task123",
        "retryCount": 0,
        "maxRetries": 3,
        "archived": false,
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:30:00Z",
        "metadata": {}
    });

    let record: ProcessExecutionRecord = serde_json::from_value(json).unwrap();
    assert_eq!(record.id, "exec_exec123abc");
    assert_eq!(record.status, "running");
    assert_eq!(record.progress, Some(35));
    assert_eq!(record.step_history.len(), 1);
    assert_eq!(record.variables["taskTitle"], "Brainstorm New Features");
}
