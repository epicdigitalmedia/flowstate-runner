use flowstate_runner::models::process::{Process, ProcessStep, StepTemplate};
use serde_json::json;

#[test]
fn test_process_deserialize_from_db_record() {
    let json = json!({
        "id": "proc_abc123xyz0",
        "name": "brainstorm-planning",
        "title": "Brainstorm & Planning",
        "description": "Generate ideas and create planning documents",
        "version": "1.0.0",
        "status": "active",
        "category": "business-process",
        "startStepId": "step_start001",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "userId": "use__user123id",
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:30:00Z",
        "trigger": {
            "type": "entity",
            "entityTrigger": {
                "entityType": "task",
                "selector": { "status": "in-progress" },
                "conditions": [
                    {
                        "propertyPath": "status",
                        "operator": "changes-to",
                        "value": "in-progress"
                    }
                ]
            }
        },
        "executionConfig": {
            "maxConcurrentExecutions": 5,
            "queueBehavior": "wait",
            "timeoutMinutes": 60,
            "singleton": false,
            "priority": 5
        },
        "maxSubprocessDepth": 10,
        "archived": false,
        "metadata": {},
        "extended": {}
    });

    let process: Process = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(process.id, "proc_abc123xyz0");
    assert_eq!(process.name, "brainstorm-planning");
    assert_eq!(process.status, "active");
    assert_eq!(process.start_step_id, Some("step_start001".to_string()));

    // Round-trip: serialize back and compare
    let re_serialized = serde_json::to_value(&process).unwrap();
    assert_eq!(re_serialized["id"], "proc_abc123xyz0");
    assert_eq!(re_serialized["startStepId"], "step_start001");
}

#[test]
fn test_process_minimal_fields() {
    let json = json!({
        "id": "proc_minimal001",
        "name": "simple-process",
        "status": "draft",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });

    let process: Process = serde_json::from_value(json).unwrap();
    assert_eq!(process.id, "proc_minimal001");
    assert!(process.trigger.is_none());
    assert!(process.start_step_id.is_none());
    assert!(process.title.is_none());
}

#[test]
fn test_process_step_deserialize() {
    let json = json!({
        "id": "step_step123abc",
        "processId": "proc_abc123xyz0",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "name": "collect-ideas",
        "title": "Collect Ideas",
        "stepType": "action",
        "order": 0,
        "enabled": true,
        "nextStepId": "step_step456def",
        "action": {
            "type": "command",
            "command": {
                "executable": "echo",
                "args": ["hello"]
            }
        },
        "inputs": { "prompt": "${taskTitle}" },
        "outputs": [
            { "name": "result", "source": "commandOutput" }
        ],
        "requiredVariables": ["taskTitle"],
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:30:00Z"
    });

    let step: ProcessStep = serde_json::from_value(json).unwrap();
    assert_eq!(step.id, "step_step123abc");
    assert_eq!(step.step_type, "action");
    assert_eq!(step.next_step_id, Some("step_step456def".to_string()));
    assert_eq!(step.required_variables, vec!["taskTitle"]);
    assert!(step.enabled);
}

#[test]
fn test_step_template_deserialize() {
    let json = json!({
        "id": "stpl_tmpl123abc",
        "name": "agent-brainstorm",
        "stepType": "agent-task",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "action": {
            "type": "agent",
            "agent": {
                "prompt": "Brainstorm ideas about ${taskTitle}",
                "model": "claude-opus-4.6"
            }
        },
        "inputs": { "taskTitle": "${title}" },
        "outputs": [
            { "name": "ideas", "source": "agentOutput" }
        ],
        "requiredVariables": ["title"],
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });

    let template: StepTemplate = serde_json::from_value(json).unwrap();
    assert_eq!(template.id, "stpl_tmpl123abc");
    assert_eq!(template.step_type, "agent-task");
    assert_eq!(template.required_variables, vec!["title"]);
}
