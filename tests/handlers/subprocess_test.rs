use flowstate_runner::agent::NoopAgentExecutor;
use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::subprocess::{
    apply_output_mapping, check_depth_limit, resolve_input_mapping, SubprocessHandler,
};
use flowstate_runner::handlers::{Handler, RunContext};
use flowstate_runner::models::execution::{
    ExecutionContext, ExecutionState, PauseReason, ProcessExecutionRecord, ResolvedStep,
    StepOutcome,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use wiremock::matchers::{body_partial_json, method, path_regex};
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
            mcp_base_url: url.to_string(),
            obs_url: None,
            plan_base_dir: PathBuf::from("/tmp/plans"),
            worker_mode: false,
            health_port: 9090,
            max_subprocess_depth: 5,
            agent_executor: "noop".to_string(),
            auth_token: None,
            api_token: None,
            auth_url: None,
            persist_interval: 1,
        },
        rest: FlowstateRestClient::new(url),
        http: reqwest::Client::new(),
        mcp: McpClient::new(url, "org_test", "work_test"),
        agent_executor: Box::new(NoopAgentExecutor),
        attribute_map: AttributeMap::default(),
        process_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        step_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        token_exchanger: None,
    }
}

fn make_state() -> ExecutionState {
    let record_json = json!({
        "id": "exec_parent0001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_subprocess",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": {},
        "stepHistory": [],
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "parent-process".to_string())
}

fn make_state_with_vars(vars: Value) -> ExecutionState {
    let record_json = json!({
        "id": "exec_parent0001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_subprocess",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": vars,
        "stepHistory": [],
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "parent-process".to_string())
}

fn make_state_with_depth(depth: u32) -> ExecutionState {
    let record_json = json!({
        "id": "exec_parent0001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_subprocess",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": {},
        "stepHistory": [],
        "archived": false,
        "metadata": {},
        "context": {
            "entityType": "project",
            "entityId": "proj_test001",
            "depth": depth,
            "maxDepth": 5
        },
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });
    let record: ProcessExecutionRecord = serde_json::from_value(record_json).unwrap();
    ExecutionState::from_record(record, "parent-process".to_string())
}

fn make_step(action: Value) -> ResolvedStep {
    ResolvedStep {
        id: "step_subprocess".to_string(),
        process_id: "proc_test".to_string(),
        name: "Run sub-workflow".to_string(),
        step_type: "subprocess".to_string(),
        action: Some(action),
        inputs: None,
        outputs: vec![],
        output_extraction: None,
        conditions: vec![],
        next_step_id: Some("step_next".to_string()),
        required_variables: vec![],
        estimated_duration_minutes: None,
        metadata: Map::new(),
    }
}

fn make_step_no_action() -> ResolvedStep {
    ResolvedStep {
        id: "step_subprocess".to_string(),
        process_id: "proc_test".to_string(),
        name: "Run sub-workflow".to_string(),
        step_type: "subprocess".to_string(),
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

// ---------------------------------------------------------------------------
// resolve_input_mapping — pure function unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_resolve_input_mapping_basic() {
    let mut parent_vars = Map::new();
    parent_vars.insert("userId".to_string(), json!("user_abc123"));
    parent_vars.insert("projectId".to_string(), json!("proj_xyz"));

    let mapping = json!({
        "childUserId": "${userId}",
        "childProjectId": "${projectId}"
    });

    let result = resolve_input_mapping(&mapping, &parent_vars);
    assert_eq!(result.get("childUserId"), Some(&json!("user_abc123")));
    assert_eq!(result.get("childProjectId"), Some(&json!("proj_xyz")));
}

#[test]
fn test_resolve_input_mapping_strips_wrapper() {
    let mut parent_vars = Map::new();
    parent_vars.insert("planDir".to_string(), json!("/tmp/plans/abc"));

    let mapping = json!({ "inputDir": "${planDir}" });
    let result = resolve_input_mapping(&mapping, &parent_vars);
    assert_eq!(result.get("inputDir"), Some(&json!("/tmp/plans/abc")));
}

#[test]
fn test_resolve_input_mapping_missing_variable() {
    let parent_vars = Map::new();
    let mapping = json!({ "childVar": "${missingVar}" });
    let result = resolve_input_mapping(&mapping, &parent_vars);
    // Missing variable should not be inserted
    assert!(
        result.get("childVar").is_none(),
        "Missing variable should be skipped, not inserted"
    );
}

#[test]
fn test_resolve_input_mapping_literal_values() {
    let parent_vars = Map::new();
    let mapping = json!({
        "literalStr": "hello",
        "literalNum": 42,
        "literalBool": true
    });
    let result = resolve_input_mapping(&mapping, &parent_vars);
    assert_eq!(result.get("literalStr"), Some(&json!("hello")));
    assert_eq!(result.get("literalNum"), Some(&json!(42)));
    assert_eq!(result.get("literalBool"), Some(&json!(true)));
}

#[test]
fn test_resolve_input_mapping_empty() {
    let parent_vars = Map::new();
    let mapping = json!({});
    let result = resolve_input_mapping(&mapping, &parent_vars);
    assert!(
        result.is_empty(),
        "Empty mapping should produce empty result"
    );
}

// ---------------------------------------------------------------------------
// apply_output_mapping — pure function unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_apply_output_mapping_basic() {
    let mut child_vars = Map::new();
    child_vars.insert("approvalStatus".to_string(), json!("approved"));
    child_vars.insert("resultDoc".to_string(), json!("doc_xyz"));

    let mapping = json!({
        "parentApprovalStatus": "approvalStatus",
        "parentResultDoc": "resultDoc"
    });
    let mut parent_vars = Map::new();
    apply_output_mapping(&mapping, &child_vars, &mut parent_vars);

    assert_eq!(
        parent_vars.get("parentApprovalStatus"),
        Some(&json!("approved"))
    );
    assert_eq!(parent_vars.get("parentResultDoc"), Some(&json!("doc_xyz")));
}

#[test]
fn test_apply_output_mapping_missing_child_var() {
    let child_vars = Map::new(); // empty — no vars at all
    let mapping = json!({ "parentVar": "missingChildVar" });
    let mut parent_vars = Map::new();
    apply_output_mapping(&mapping, &child_vars, &mut parent_vars);
    assert!(
        parent_vars.get("parentVar").is_none(),
        "Missing child variable should be silently skipped"
    );
}

#[test]
fn test_apply_output_mapping_empty() {
    let mut child_vars = Map::new();
    child_vars.insert("someVar".to_string(), json!("value"));
    let mapping = json!({});
    let mut parent_vars = Map::new();
    apply_output_mapping(&mapping, &child_vars, &mut parent_vars);
    assert!(
        parent_vars.is_empty(),
        "Empty mapping should not modify parent vars"
    );
}

// ---------------------------------------------------------------------------
// check_depth_limit — pure function unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_depth_limit_within_bounds() {
    let ctx = ExecutionContext {
        entity_type: "project".to_string(),
        entity_id: "proj_test".to_string(),
        user_id: None,
        tags: vec![],
        category: None,
        depth: 2,
        max_depth: 5,
        process_name: None,
    };
    let result = check_depth_limit(Some(&ctx), 5);
    assert_eq!(result.unwrap(), 3, "Should return depth + 1");
}

#[test]
fn test_depth_limit_exceeded() {
    let ctx = ExecutionContext {
        entity_type: "project".to_string(),
        entity_id: "proj_test".to_string(),
        user_id: None,
        tags: vec![],
        category: None,
        depth: 5,
        max_depth: 5,
        process_name: None,
    };
    let result = check_depth_limit(Some(&ctx), 5);
    assert!(result.is_err(), "Should error when depth >= max");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("depth limit"),
        "Error should mention depth limit: {err}"
    );
}

#[test]
fn test_depth_limit_no_context() {
    // No context means top-level (depth = 0)
    let result = check_depth_limit(None, 5);
    assert_eq!(
        result.unwrap(),
        1,
        "No context should default to depth 0, return 1"
    );
}

// ---------------------------------------------------------------------------
// SubprocessHandler::execute() — wiremock integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_subprocess_execute_creates_child_and_pauses() {
    let mock_server = MockServer::start().await;

    // processexecutions is VCA — create goes through MCP collection-create
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-create"))
        .and(body_partial_json(json!({ "collection": "processexecutions" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documentId": "exec_new_child1"
        })))
        .mount(&mock_server)
        .await;

    let handler = SubprocessHandler;
    let step = make_step(json!({
        "processId": "proc_child0001",
        "waitForCompletion": true
    }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();

    match result {
        StepOutcome::Paused(PauseReason::Subprocess { child_execution_id }) => {
            assert!(
                child_execution_id.starts_with("exec_"),
                "child_execution_id should start with 'exec_', got: {child_execution_id}"
            );
        }
        other => panic!("Expected Paused(Subprocess), got {:?}", other),
    }

    // Verify the MCP collection-create endpoint was called
    let requests = mock_server.received_requests().await.unwrap();
    let has_create = requests
        .iter()
        .any(|r| r.url.path().contains("tools/collection-create"));
    assert!(has_create, "Should have called MCP collection-create");
}

#[tokio::test]
async fn test_subprocess_execute_no_action_fails() {
    let handler = SubprocessHandler;
    let step = make_step_no_action();
    let state = make_state();
    let ctx = make_run_context_with_url("http://localhost:99999");

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
async fn test_subprocess_execute_depth_limit() {
    // Set depth to max — should fail without hitting the network
    let state = make_state_with_depth(5);
    let handler = SubprocessHandler;
    let step = make_step(json!({ "processId": "proc_child0001" }));
    let ctx = make_run_context_with_url("http://localhost:99999");

    let result = handler.execute(&step, &state, &ctx).await;
    assert!(result.is_err(), "Should error when depth limit is reached");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("depth"),
        "Error should mention depth: {err}"
    );
}

// ---------------------------------------------------------------------------
// SubprocessHandler::check_resume() — wiremock integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_subprocess_resume_completed_with_output_mapping() {
    let mock_server = MockServer::start().await;

    // processexecutions is VCA — get goes through MCP collection-get
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(json!({ "collection": "processexecutions", "id": "exec_child00001" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "exec_child00001",
                "status": "completed",
                "variables": {
                    "approvalResult": "approved",
                    "outputDoc": "doc_xyz123"
                }
            }
        })))
        .mount(&mock_server)
        .await;

    let handler = SubprocessHandler;
    let step = make_step(json!({
        "processId": "proc_child0001",
        "outputMapping": {
            "parentApproval": "approvalResult",
            "parentDoc": "outputDoc"
        }
    }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::Subprocess {
        child_execution_id: "exec_child00001".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    match result {
        Some(StepOutcome::Completed { outputs }) => {
            assert_eq!(
                outputs.get("childExecutionId"),
                Some(&json!("exec_child00001"))
            );
            assert_eq!(outputs.get("childStatus"), Some(&json!("completed")));
            assert_eq!(outputs.get("parentApproval"), Some(&json!("approved")));
            assert_eq!(outputs.get("parentDoc"), Some(&json!("doc_xyz123")));
        }
        other => panic!(
            "Expected Some(Completed) with mapped outputs, got {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn test_subprocess_resume_failed() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(json!({ "collection": "processexecutions", "id": "exec_child_fail" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "exec_child_fail",
                "status": "failed",
                "error": { "message": "Step 3 threw an unhandled exception" }
            }
        })))
        .mount(&mock_server)
        .await;

    let handler = SubprocessHandler;
    let step = make_step(json!({ "processId": "proc_child0001" }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::Subprocess {
        child_execution_id: "exec_child_fail".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    match result {
        Some(StepOutcome::Failed { error }) => {
            assert!(
                error.contains("exec_child_fail"),
                "Error should include child ID: {error}"
            );
            assert!(
                error.contains("unhandled exception"),
                "Error should include the child's error message: {error}"
            );
        }
        other => panic!("Expected Some(Failed), got {:?}", other),
    }
}

#[tokio::test]
async fn test_subprocess_resume_still_running() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(json!({ "collection": "processexecutions", "id": "exec_child_run" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "exec_child_run",
                "status": "running"
            }
        })))
        .mount(&mock_server)
        .await;

    let handler = SubprocessHandler;
    let step = make_step(json!({ "processId": "proc_child0001" }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::Subprocess {
        child_execution_id: "exec_child_run".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "Running child should return None (still waiting)"
    );
}

#[tokio::test]
async fn test_subprocess_resume_wrong_pause_reason_returns_none() {
    let handler = SubprocessHandler;
    let step = make_step(json!({ "processId": "proc_child0001" }));
    let state = make_state();
    let ctx = make_run_context_with_url("http://localhost:99999");

    let reason = PauseReason::Approval {
        approval_id: "appr_abc123".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "Non-Subprocess pause reason should return None"
    );
}

// ---------------------------------------------------------------------------
// Idempotency tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_subprocess_idempotency_completed_child() {
    // When childExecutionId is already in parent vars and child is completed,
    // execute() should apply output mapping and return Completed without
    // creating a new child.
    let mock_server = MockServer::start().await;

    // MCP get to fetch existing child
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(json!({ "collection": "processexecutions", "id": "exec_existing001" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "exec_existing001",
                "status": "completed",
                "variables": { "childResult": "success" }
            }
        })))
        .mount(&mock_server)
        .await;

    let handler = SubprocessHandler;
    let step = make_step(json!({
        "processId": "proc_child0001",
        "outputMapping": { "parentResult": "childResult" }
    }));
    let state = make_state_with_vars(json!({ "childExecutionId": "exec_existing001" }));
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();

    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(
                outputs.get("childExecutionId"),
                Some(&json!("exec_existing001")),
                "Should include child ID in outputs"
            );
            assert_eq!(
                outputs.get("parentResult"),
                Some(&json!("success")),
                "Should apply output mapping"
            );
        }
        other => panic!("Expected Completed with mapped outputs, got {:?}", other),
    }

    // No POST to MCP collection-create — child already exists
    let requests = mock_server.received_requests().await.unwrap();
    let has_create = requests
        .iter()
        .any(|r| r.url.path().contains("tools/collection-create"));
    assert!(
        !has_create,
        "Should NOT create a new child when existing is completed"
    );
}

#[tokio::test]
async fn test_subprocess_idempotency_running_child_returns_paused() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(json!({ "collection": "processexecutions", "id": "exec_existing002" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "exec_existing002",
                "status": "running"
            }
        })))
        .mount(&mock_server)
        .await;

    let handler = SubprocessHandler;
    let step = make_step(json!({ "processId": "proc_child0001" }));
    let state = make_state_with_vars(json!({ "childExecutionId": "exec_existing002" }));
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();

    match result {
        StepOutcome::Paused(PauseReason::Subprocess { child_execution_id }) => {
            assert_eq!(
                child_execution_id, "exec_existing002",
                "Should reuse the existing child ID"
            );
        }
        other => panic!("Expected Paused(Subprocess), got {:?}", other),
    }

    // No POST to MCP collection-create — child already exists
    let requests = mock_server.received_requests().await.unwrap();
    let has_create = requests
        .iter()
        .any(|r| r.url.path().contains("tools/collection-create"));
    assert!(
        !has_create,
        "Should NOT create a new child when existing is still running"
    );
}

#[tokio::test]
async fn test_subprocess_idempotency_failed_child_creates_new() {
    let mock_server = MockServer::start().await;

    // MCP get returns failed child
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(json!({ "collection": "processexecutions", "id": "exec_existing003" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "exec_existing003",
                "status": "failed",
                "error": { "message": "previous run failed" }
            }
        })))
        .mount(&mock_server)
        .await;

    // MCP create for new child
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-create"))
        .and(body_partial_json(json!({ "collection": "processexecutions" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documentId": "exec_new_child3"
        })))
        .mount(&mock_server)
        .await;

    let handler = SubprocessHandler;
    let step = make_step(json!({ "processId": "proc_child0001" }));
    let state = make_state_with_vars(json!({ "childExecutionId": "exec_existing003" }));
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();

    match result {
        StepOutcome::Paused(PauseReason::Subprocess { child_execution_id }) => {
            assert_ne!(
                child_execution_id, "exec_existing003",
                "Should create a NEW child ID, not reuse the failed one"
            );
            assert!(
                child_execution_id.starts_with("exec_"),
                "New child ID should start with 'exec_': {child_execution_id}"
            );
        }
        other => panic!(
            "Expected Paused(Subprocess) with new child, got {:?}",
            other
        ),
    }

    // Verify MCP collection-create was called for the new child
    let requests = mock_server.received_requests().await.unwrap();
    let has_create = requests
        .iter()
        .any(|r| r.url.path().contains("tools/collection-create"));
    assert!(
        has_create,
        "Should create a new child when previous one failed"
    );
}
