use flowstate_runner::agent::NoopAgentExecutor;
use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::human_task::{build_discussion_content, HumanTaskHandler};
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
        "currentStepId": "step_human",
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
    ResolvedStep {
        id: "step_human".to_string(),
        process_id: "proc_test".to_string(),
        name: "Ask Human".to_string(),
        step_type: "human-task".to_string(),
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

// ---------------------------------------------------------------------------
// build_discussion_content — pure function unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_discussion_content_with_interpolation() {
    let action = json!({ "content": "Hello ${userName}, please review ${taskTitle}." });
    let mut variables = Map::new();
    variables.insert("userName".to_string(), Value::String("Alice".to_string()));
    variables.insert(
        "taskTitle".to_string(),
        Value::String("Design Doc".to_string()),
    );

    let result = build_discussion_content(&action, &variables);
    assert_eq!(result, "Hello Alice, please review Design Doc.");
}

#[test]
fn test_build_discussion_content_no_template() {
    // No content field at all → empty string
    let action = json!({ "entityType": "task" });
    let variables = Map::new();

    let result = build_discussion_content(&action, &variables);
    assert_eq!(result, "");
}

#[test]
fn test_build_discussion_content_empty_string() {
    let action = json!({ "content": "" });
    let variables = Map::new();

    let result = build_discussion_content(&action, &variables);
    assert_eq!(result, "");
}

#[test]
fn test_build_discussion_content_no_variables_needed() {
    let action = json!({ "content": "Please review the attached design document." });
    let variables = Map::new();

    let result = build_discussion_content(&action, &variables);
    assert_eq!(result, "Please review the attached design document.");
}

// ---------------------------------------------------------------------------
// HumanTaskHandler::execute() — wiremock integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_human_task_execute_posts_discussion_and_pauses() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/discussions-rest/\d+/set"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&mock_server)
        .await;

    let handler = HumanTaskHandler;
    let step = make_step(json!({
        "content": "Please review this and let me know.",
        "entityType": "task",
        "entityId": "task_abc123"
    }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let result = handler.execute(&step, &state, &ctx).await.unwrap();

    match result {
        StepOutcome::Paused(PauseReason::HumanTask {
            discussion_id,
            posted_at,
        }) => {
            assert!(
                discussion_id.starts_with("disc_"),
                "discussion_id should have 'disc_' prefix, got: {discussion_id}"
            );
            assert!(
                !posted_at.is_empty(),
                "posted_at should be a non-empty RFC3339 timestamp"
            );
        }
        other => panic!("Expected Paused(HumanTask), got {:?}", other),
    }
}

#[tokio::test]
async fn test_human_task_execute_no_action() {
    let handler = HumanTaskHandler;
    let step = ResolvedStep {
        id: "step_human".to_string(),
        process_id: "proc_test".to_string(),
        name: "Ask Human".to_string(),
        step_type: "human-task".to_string(),
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
async fn test_human_task_execute_empty_content() {
    let handler = HumanTaskHandler;
    let step = make_step(json!({ "content": "" }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();

    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.contains("content") || error.contains("empty"),
                "Error should mention content or empty: {error}"
            );
        }
        other => panic!("Expected Failed for empty content, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// HumanTaskHandler::check_resume() — wiremock integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_human_task_resume_with_replies() {
    let mock_server = MockServer::start().await;

    // Reply posted after the discussion — should trigger resume
    Mock::given(method("POST"))
        .and(path_regex(r"/discussions-rest/\d+/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {
                    "id": "disc_reply001",
                    "parentId": "disc_original1",
                    "content": "I have reviewed it and it looks good.",
                    "createdAt": "2026-01-15T11:00:00Z"
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let handler = HumanTaskHandler;
    let step = make_step(json!({ "content": "Please review." }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::HumanTask {
        discussion_id: "disc_original1".to_string(),
        // posted_at is before the reply's createdAt
        posted_at: "2026-01-15T10:00:00Z".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    match result {
        Some(StepOutcome::Completed { outputs }) => {
            assert_eq!(
                outputs.get("humanReply"),
                Some(&json!("I have reviewed it and it looks good."))
            );
            assert_eq!(outputs.get("replyCount"), Some(&json!(1)));
        }
        other => panic!("Expected Some(Completed), got {:?}", other),
    }
}

#[tokio::test]
async fn test_human_task_resume_no_replies() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/discussions-rest/\d+/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": []
        })))
        .mount(&mock_server)
        .await;

    let handler = HumanTaskHandler;
    let step = make_step(json!({ "content": "Please review." }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::HumanTask {
        discussion_id: "disc_noreply1".to_string(),
        posted_at: "2026-01-15T10:00:00Z".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "No replies should return None (still waiting)"
    );
}

#[tokio::test]
async fn test_human_task_resume_filters_old_replies() {
    let mock_server = MockServer::start().await;

    // Reply with createdAt BEFORE posted_at — must be filtered out
    Mock::given(method("POST"))
        .and(path_regex(r"/discussions-rest/\d+/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {
                    "id": "disc_old_reply",
                    "parentId": "disc_witholdreply",
                    "content": "This reply was from before we paused.",
                    // Before posted_at of 2026-01-15T10:00:00Z
                    "createdAt": "2026-01-15T09:00:00Z"
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let handler = HumanTaskHandler;
    let step = make_step(json!({ "content": "Please review." }));
    let state = make_state();
    let ctx = make_run_context_with_url(&mock_server.uri());

    let reason = PauseReason::HumanTask {
        discussion_id: "disc_witholdreply".to_string(),
        // posted_at is AFTER the reply's createdAt
        posted_at: "2026-01-15T10:00:00Z".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "Replies older than posted_at should be filtered out — still waiting"
    );
}

#[tokio::test]
async fn test_human_task_resume_wrong_reason() {
    let handler = HumanTaskHandler;
    let step = make_step(json!({ "content": "Please review." }));
    let state = make_state();
    let ctx = make_run_context();

    let reason = PauseReason::Subprocess {
        child_execution_id: "exec_child001".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "Non-HumanTask pause reason should return None"
    );
}
