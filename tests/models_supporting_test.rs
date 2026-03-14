use flowstate_runner::models::agent::*;
use flowstate_runner::models::approval::*;
use serde_json::json;

#[test]
fn test_approval_deserialize() {
    let json = json!({
        "id": "appr_approval123",
        "processExecutionId": "exec_exec123abc",
        "stepId": "step_approval456",
        "status": "pending",
        "category": "spec",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });

    let approval: ApprovalRecord = serde_json::from_value(json).unwrap();
    assert_eq!(approval.id, "appr_approval123");
    assert_eq!(approval.status, "pending");
    assert_eq!(approval.category, Some("spec".to_string()));
}

#[test]
fn test_approval_with_response() {
    let json = json!({
        "id": "appr_approval123",
        "processExecutionId": "exec_exec123abc",
        "stepId": "step_approval456",
        "status": "approved",
        "response": "Looks good, approved.",
        "annotations": { "quality": "high" },
        "reviewerId": "use__reviewer1",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T11:00:00Z"
    });

    let approval: ApprovalRecord = serde_json::from_value(json).unwrap();
    assert_eq!(approval.status, "approved");
    assert_eq!(approval.response, Some("Looks good, approved.".to_string()));
}

#[test]
fn test_agent_config_deserialize() {
    let json = json!({
        "agentName": "brainstorm-agent",
        "provider": "claude-code",
        "model": "claude-opus-4.6",
        "timeout": 300,
        "workingDir": "/workspace",
        "permissionMode": "acceptEdits"
    });

    let config: AgentConfig = serde_json::from_value(json).unwrap();
    assert_eq!(config.agent_name, Some("brainstorm-agent".to_string()));
    assert_eq!(config.provider, Some("claude-code".to_string()));
    assert_eq!(config.timeout, Some(300));
}

#[test]
fn test_agent_metrics_deserialize() {
    let json = json!({
        "inputTokens": 1500,
        "outputTokens": 3200,
        "cacheReadTokens": 500,
        "model": "claude-opus-4.6",
        "durationMs": 45000,
        "cost": 0.15
    });

    let metrics: AgentMetrics = serde_json::from_value(json).unwrap();
    assert_eq!(metrics.input_tokens, 1500);
    assert_eq!(metrics.output_tokens, 3200);
    assert_eq!(metrics.cost, Some(0.15));
}

#[test]
fn test_output_spec_deserialize() {
    let json = json!({
        "name": "result",
        "source": "commandOutput",
        "jsonPath": ".documents[0].id"
    });

    let spec: OutputSpec = serde_json::from_value(json).unwrap();
    assert_eq!(spec.name, "result");
    assert_eq!(spec.source, Some("commandOutput".to_string()));
    assert_eq!(spec.json_path, Some(".documents[0].id".to_string()));
}

#[test]
fn test_output_extraction_deserialize() {
    let json = json!({
        "mode": "jq",
        "source": "commandOutput",
        "expression": ".results[]",
        "mergeResult": true
    });

    let extraction: OutputExtraction = serde_json::from_value(json).unwrap();
    assert_eq!(extraction.mode, ExtractionMode::Jq);
    assert!(extraction.merge_result);
}
