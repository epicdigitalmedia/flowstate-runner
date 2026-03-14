use flowstate_runner::agent::NoopAgentExecutor;
use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::approval::{
    build_approval_record, find_condition_target, ApprovalHandler,
};
use flowstate_runner::handlers::{Handler, RunContext};
use flowstate_runner::models::execution::{
    ExecutionState, PauseReason, ProcessExecutionRecord, ResolvedStep, StepOutcome,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_run_context_with_url(url: &str) -> RunContext {
    RunContext {
        config: Config {
            org_id: "org_test".to_string(),
            workspace_id: "work_test".to_string(),
            rest_base_url: url.to_string(),
            mcp_base_url: format!("{}/mcp", url),
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
        rest: FlowstateRestClient::new(url),
        http: reqwest::Client::new(),
        mcp: McpClient::new("http://localhost:9999/mcp", "test-org", "test-workspace"),
        agent_executor: Box::new(NoopAgentExecutor),
        attribute_map: AttributeMap::default(),
        process_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        step_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        token_exchanger: None,
    }
}

fn make_run_context() -> RunContext {
    make_run_context_with_url("http://localhost:99999")
}

fn make_state() -> ExecutionState {
    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_approval",
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

fn make_step(action: Value) -> ResolvedStep {
    make_step_with_conditions(action, vec![])
}

fn make_step_with_conditions(action: Value, conditions: Vec<Value>) -> ResolvedStep {
    ResolvedStep {
        id: "step_approval".to_string(),
        process_id: "proc_test".to_string(),
        name: "Review Design".to_string(),
        step_type: "approval".to_string(),
        action: Some(action),
        inputs: None,
        outputs: vec![],
        output_extraction: None,
        conditions,
        next_step_id: Some("step_next".to_string()),
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    }
}

// ---------------------------------------------------------------------------
// build_approval_record — pure function unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_approval_record_human_strategy_default() {
    let action = json!({ "title": "Review the spec" });
    let record = build_approval_record(
        &action,
        "exec_abc123",
        "step_approve",
        "org_test",
        "work_test",
    );

    assert!(
        record["id"].as_str().unwrap_or("").starts_with("appr_"),
        "ID should start with 'appr_'"
    );
    assert_eq!(record["strategy"], json!("human"));
    assert_eq!(record["status"], json!("pending"));
    assert_eq!(record["executionId"], json!("exec_abc123"));
    assert_eq!(record["stepId"], json!("step_approve"));
    assert_eq!(record["orgId"], json!("org_test"));
    assert_eq!(record["workspaceId"], json!("work_test"));
    assert_eq!(record["title"], json!("Review the spec"));
}

#[test]
fn test_build_approval_record_default_category_is_spec() {
    let action = json!({});
    let record = build_approval_record(&action, "exec_x", "step_x", "org_x", "work_x");
    assert_eq!(record["category"], json!("spec"));
}

#[test]
fn test_build_approval_record_custom_strategy_and_category() {
    let action = json!({ "strategy": "agent_approve", "category": "steering" });
    let record = build_approval_record(&action, "exec_x", "step_x", "org_x", "work_x");
    assert_eq!(record["strategy"], json!("agent_approve"));
    assert_eq!(record["category"], json!("steering"));
}

#[test]
fn test_build_approval_record_ids_are_unique() {
    let action = json!({});
    let r1 = build_approval_record(&action, "exec_x", "step_x", "org_x", "work_x");
    let r2 = build_approval_record(&action, "exec_x", "step_x", "org_x", "work_x");
    assert_ne!(r1["id"], r2["id"], "Each call should produce a unique ID");
}

// ---------------------------------------------------------------------------
// find_condition_target — pure function unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_find_condition_target_rejected() {
    let conditions = vec![
        json!({ "value": "approved", "targetStepId": "step_done" }),
        json!({ "value": "rejected", "targetStepId": "step_revise" }),
    ];
    let result = find_condition_target(&conditions, "rejected");
    assert_eq!(result, Some("step_revise".to_string()));
}

#[test]
fn test_find_condition_target_needs_revision() {
    let conditions = vec![json!({ "value": "needs-revision", "targetStepId": "step_rework" })];
    let result = find_condition_target(&conditions, "needs-revision");
    assert_eq!(result, Some("step_rework".to_string()));
}

#[test]
fn test_find_condition_target_not_found() {
    let conditions = vec![json!({ "value": "approved", "targetStepId": "step_done" })];
    let result = find_condition_target(&conditions, "rejected");
    assert!(result.is_none(), "Should return None when no match");
}

#[test]
fn test_find_condition_target_empty_conditions() {
    let result = find_condition_target(&[], "rejected");
    assert!(result.is_none(), "Should return None for empty slice");
}

// ---------------------------------------------------------------------------
// ApprovalHandler::execute() — wiremock integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_no_action_fails() {
    let handler = ApprovalHandler;
    let step = ResolvedStep {
        id: "step_approval".to_string(),
        process_id: "proc_test".to_string(),
        name: "Review".to_string(),
        step_type: "approval".to_string(),
        action: None,
        inputs: None,
        outputs: vec![],
        output_extraction: None,
        conditions: vec![],
        next_step_id: None,
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    };
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.contains("action"),
                "Error should mention 'action': {error}"
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_creates_record_and_pauses() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/approvals-rest/\d+/set"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&mock_server)
        .await;

    let handler = ApprovalHandler;
    let step = make_step(json!({ "title": "Approve design" }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();

    match result {
        StepOutcome::Paused(PauseReason::Approval { approval_id }) => {
            assert!(
                approval_id.starts_with("appr_"),
                "approval_id should have 'appr_' prefix, got: {approval_id}"
            );
        }
        other => panic!("Expected Paused(Approval), got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_agent_approve_creates_bridge_task() {
    let mock_server = MockServer::start().await;

    // Approvals set endpoint
    Mock::given(method("POST"))
        .and(path_regex(r"/approvals-rest/\d+/set"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&mock_server)
        .await;

    // Tasks set endpoint — must also be called for agent_approve
    Mock::given(method("POST"))
        .and(path_regex(r"/tasks-rest/\d+/set"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&mock_server)
        .await;

    let handler = ApprovalHandler;
    let step = make_step(json!({
        "title": "Agent review",
        "strategy": "agent_approve"
    }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();

    match result {
        StepOutcome::Paused(PauseReason::Approval { approval_id }) => {
            assert!(approval_id.starts_with("appr_"));
        }
        other => panic!("Expected Paused(Approval), got {:?}", other),
    }

    // Verify both endpoints were called
    let approval_requests = mock_server.received_requests().await.unwrap();
    let set_paths: Vec<&str> = approval_requests.iter().map(|r| r.url.path()).collect();

    let has_approval_set = approval_requests
        .iter()
        .any(|r| r.url.path().contains("approvals-rest") && r.url.path().contains("/set"));
    let has_task_set = approval_requests
        .iter()
        .any(|r| r.url.path().contains("tasks-rest") && r.url.path().contains("/set"));

    assert!(
        has_approval_set,
        "Should have called approvals-rest set. Paths: {:?}",
        set_paths
    );
    assert!(
        has_task_set,
        "Should have called tasks-rest set for agent_approve. Paths: {:?}",
        set_paths
    );
}

// ---------------------------------------------------------------------------
// ApprovalHandler::check_resume() — wiremock integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_check_resume_wrong_pause_reason_returns_none() {
    let handler = ApprovalHandler;
    let step = make_step(json!({}));
    let state = make_state();
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
        "Non-Approval pause reason should return None"
    );
}

#[tokio::test]
async fn test_check_resume_approved() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex(r"/approvals-rest/\d+/appr_approved1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "appr_approved1",
            "status": "approved",
            "feedback": "Looks great!",
            "annotations": { "score": 5 }
        })))
        .mount(&mock_server)
        .await;

    let handler = ApprovalHandler;
    let step = make_step(json!({}));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::Approval {
        approval_id: "appr_approved1".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    match result {
        Some(StepOutcome::Completed { outputs }) => {
            assert_eq!(outputs.get("approvalStatus"), Some(&json!("approved")));
            assert_eq!(
                outputs.get("approvalFeedback"),
                Some(&json!("Looks great!"))
            );
            assert_eq!(
                outputs.get("approvalAnnotations"),
                Some(&json!({ "score": 5 }))
            );
        }
        other => panic!("Expected Some(Completed), got {:?}", other),
    }
}

#[tokio::test]
async fn test_check_resume_rejected_with_routing() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex(r"/approvals-rest/\d+/appr_rejected1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "appr_rejected1",
            "status": "rejected",
            "feedback": "Not ready yet"
        })))
        .mount(&mock_server)
        .await;

    let handler = ApprovalHandler;
    let conditions = vec![json!({ "value": "rejected", "targetStepId": "step_revise" })];
    let step = make_step_with_conditions(json!({}), conditions);
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::Approval {
        approval_id: "appr_rejected1".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    match result {
        Some(StepOutcome::Completed { outputs }) => {
            assert_eq!(outputs.get("approvalStatus"), Some(&json!("rejected")));
            assert_eq!(
                outputs.get("approvalFeedback"),
                Some(&json!("Not ready yet"))
            );
            assert_eq!(
                outputs.get("_next_step_override"),
                Some(&json!("step_revise")),
                "Should route to step_revise via condition"
            );
        }
        other => panic!("Expected Some(Completed), got {:?}", other),
    }
}

#[tokio::test]
async fn test_check_resume_needs_revision_with_routing() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex(r"/approvals-rest/\d+/appr_revision1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "appr_revision1",
            "status": "needs-revision",
            "feedback": "Please update section 3"
        })))
        .mount(&mock_server)
        .await;

    let handler = ApprovalHandler;
    let conditions = vec![json!({ "value": "needs-revision", "targetStepId": "step_rework" })];
    let step = make_step_with_conditions(json!({}), conditions);
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::Approval {
        approval_id: "appr_revision1".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    match result {
        Some(StepOutcome::Completed { outputs }) => {
            assert_eq!(
                outputs.get("approvalStatus"),
                Some(&json!("needs-revision"))
            );
            assert_eq!(
                outputs.get("_next_step_override"),
                Some(&json!("step_rework"))
            );
        }
        other => panic!("Expected Some(Completed), got {:?}", other),
    }
}

#[tokio::test]
async fn test_check_resume_pending_returns_none() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex(r"/approvals-rest/\d+/appr_pending1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "appr_pending1",
            "status": "pending"
        })))
        .mount(&mock_server)
        .await;

    let handler = ApprovalHandler;
    let step = make_step(json!({}));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::Approval {
        approval_id: "appr_pending1".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "Pending approval should return None (still waiting)"
    );
}

#[tokio::test]
async fn test_check_resume_rejected_no_matching_condition() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_regex(r"/approvals-rest/\d+/appr_rej_nc1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "appr_rej_nc1",
            "status": "rejected",
            "feedback": "Rejected"
        })))
        .mount(&mock_server)
        .await;

    let handler = ApprovalHandler;
    // No conditions defined — no routing override
    let step = make_step(json!({}));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::Approval {
        approval_id: "appr_rej_nc1".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    match result {
        Some(StepOutcome::Completed { outputs }) => {
            assert_eq!(outputs.get("approvalStatus"), Some(&json!("rejected")));
            assert!(
                outputs.get("_next_step_override").is_none(),
                "Should have no _next_step_override when no matching condition"
            );
        }
        other => panic!("Expected Some(Completed), got {:?}", other),
    }
}
