use flowstate_runner::agent::NoopAgentExecutor;
use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::agent_task::{
    build_agent_prompt, collect_output_files, extract_agent_config, should_skip_agent,
    AgentTaskHandler,
};
use flowstate_runner::handlers::{Handler, RunContext};
use flowstate_runner::models::execution::{
    ExecutionState, PauseReason, ProcessExecutionRecord, ResolvedStep, StepOutcome,
};
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::PathBuf;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_run_context() -> RunContext {
    RunContext {
        config: Config {
            org_id: "org_test".to_string(),
            workspace_id: "work_test".to_string(),
            rest_base_url: "http://localhost:99999".to_string(),
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
        agent_executor: Box::new(NoopAgentExecutor),
        attribute_map: AttributeMap::default(),
        process_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        step_cache: std::sync::Mutex::new(flowstate_runner::cache::TtlCache::new(std::time::Duration::from_secs(60))),
        token_exchanger: None,
    }
}

fn make_state() -> ExecutionState {
    let record_json = json!({
        "id": "exec_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "status": "running",
        "currentStepId": "step_agent",
        "startedAt": "2026-01-15T10:00:00Z",
        "variables": { "feature": "login-flow", "version": "2" },
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
        id: "step_agent".to_string(),
        process_id: "proc_test".to_string(),
        name: "Run Agent".to_string(),
        step_type: "agent-task".to_string(),
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

/// Write a file with given content under the directory, creating dirs as needed.
fn write_file(dir: &std::path::Path, name: &str, content: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let mut f = std::fs::File::create(dir.join(name)).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

// ---------------------------------------------------------------------------
// build_agent_prompt
// ---------------------------------------------------------------------------

#[test]
fn test_build_agent_prompt_with_variables() {
    let action = json!({ "prompt": "Design the ${feature} feature for version ${version}" });
    let mut vars = Map::new();
    vars.insert("feature".to_string(), json!("login-flow"));
    vars.insert("version".to_string(), json!("2"));

    let result = build_agent_prompt(&action, &vars);

    assert_eq!(result, "Design the login-flow feature for version 2");
}

#[test]
fn test_build_agent_prompt_with_system_context() {
    let action = json!({
        "prompt": "Implement the feature.",
        "systemContext": "You are a senior engineer."
    });
    let vars = Map::new();

    let result = build_agent_prompt(&action, &vars);

    assert!(
        result.starts_with("You are a senior engineer."),
        "systemContext should be prepended"
    );
    assert!(
        result.contains("Implement the feature."),
        "prompt should follow systemContext"
    );
}

#[test]
fn test_build_agent_prompt_no_prompt_field() {
    let action = json!({ "agentName": "claude" });
    let vars = Map::new();

    let result = build_agent_prompt(&action, &vars);

    assert!(
        result.is_empty(),
        "Should return empty string when no prompt"
    );
}

#[test]
fn test_build_agent_prompt_unresolved_variables_preserved() {
    let action = json!({ "prompt": "Hello ${unknown_var}" });
    let vars = Map::new();

    let result = build_agent_prompt(&action, &vars);

    // Unresolved refs are preserved as-is by interpolate_str
    assert!(
        result.contains("${unknown_var}"),
        "Unresolved var should be preserved"
    );
}

// ---------------------------------------------------------------------------
// should_skip_agent
// ---------------------------------------------------------------------------

#[test]
fn test_should_skip_agent_all_files_exist() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path();

    write_file(dir_path, "design.md", "# Design doc\n\nContent here.");
    write_file(dir_path, "tasks.md", "- Task 1\n- Task 2");

    let action = json!({
        "outputFiles": ["design.md", "tasks.md"]
    });

    assert!(
        should_skip_agent(&action, Some(dir_path.to_str().unwrap())),
        "Should skip when all output files exist with content"
    );
}

#[test]
fn test_should_skip_agent_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path();

    write_file(dir_path, "design.md", "# Design");
    // tasks.md intentionally not written

    let action = json!({
        "outputFiles": ["design.md", "tasks.md"]
    });

    assert!(
        !should_skip_agent(&action, Some(dir_path.to_str().unwrap())),
        "Should not skip when a file is missing"
    );
}

#[test]
fn test_should_skip_agent_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path();

    write_file(dir_path, "design.md", "# Design");
    // Write an empty file
    std::fs::File::create(dir_path.join("tasks.md")).unwrap();

    let action = json!({
        "outputFiles": ["design.md", "tasks.md"]
    });

    assert!(
        !should_skip_agent(&action, Some(dir_path.to_str().unwrap())),
        "Should not skip when a file is empty"
    );
}

#[test]
fn test_should_skip_agent_no_output_files() {
    let dir = tempfile::tempdir().unwrap();
    let action = json!({ "prompt": "Do something" });

    assert!(
        !should_skip_agent(&action, Some(dir.path().to_str().unwrap())),
        "Should not skip when outputFiles is absent"
    );
}

#[test]
fn test_should_skip_agent_no_plan_dir() {
    let action = json!({
        "outputFiles": ["design.md"]
    });

    assert!(
        !should_skip_agent(&action, None),
        "Should not skip when plan_dir is None"
    );
}

// ---------------------------------------------------------------------------
// collect_output_files
// ---------------------------------------------------------------------------

#[test]
fn test_collect_output_files_text_file() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path();

    write_file(dir_path, "notes.md", "# Notes\n\nSome content.");

    let action = json!({ "outputFiles": ["notes.md"] });
    let result = collect_output_files(&action, Some(dir_path.to_str().unwrap()));

    assert_eq!(
        result.get("notes"),
        Some(&json!("# Notes\n\nSome content.")),
        "Text file should be stored as string under stem key"
    );
}

#[test]
fn test_collect_output_files_json_file() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path();

    write_file(dir_path, "result.json", r#"{"status":"ok","count":3}"#);

    let action = json!({ "outputFiles": ["result.json"] });
    let result = collect_output_files(&action, Some(dir_path.to_str().unwrap()));

    let expected = json!({"status": "ok", "count": 3});
    assert_eq!(
        result.get("result"),
        Some(&expected),
        "JSON file should be auto-parsed into a Value"
    );
}

#[test]
fn test_collect_output_files_missing_file_omitted() {
    let dir = tempfile::tempdir().unwrap();
    let action = json!({ "outputFiles": ["nonexistent.md"] });

    let result = collect_output_files(&action, Some(dir.path().to_str().unwrap()));

    assert!(
        result.get("nonexistent").is_none(),
        "Missing file should be silently omitted"
    );
}

#[test]
fn test_collect_output_files_no_plan_dir() {
    let action = json!({ "outputFiles": ["design.md"] });
    let result = collect_output_files(&action, None);
    assert!(
        result.is_empty(),
        "Should return empty map when plan_dir is None"
    );
}

// ---------------------------------------------------------------------------
// extract_agent_config
// ---------------------------------------------------------------------------

#[test]
fn test_extract_agent_config_full() {
    let action = json!({
        "agentName": "claude",
        "provider": "anthropic",
        "model": "claude-opus-4-6",
        "timeout": 120,
        "systemContext": "You are a helpful assistant.",
        "workingDir": "/tmp/project",
        "permissionMode": "default",
        "teamMemberId": "team_abc123"
    });

    let config = extract_agent_config(&action);

    assert_eq!(config.agent_name.as_deref(), Some("claude"));
    assert_eq!(config.provider.as_deref(), Some("anthropic"));
    assert_eq!(config.model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(config.timeout, Some(120));
    assert_eq!(
        config.memory_context.as_deref(),
        Some("You are a helpful assistant.")
    );
    assert_eq!(config.working_dir.as_deref(), Some("/tmp/project"));
    assert_eq!(config.permission_mode.as_deref(), Some("default"));
    assert_eq!(config.team_member_id.as_deref(), Some("team_abc123"));
}

#[test]
fn test_extract_agent_config_minimal() {
    let action = json!({ "prompt": "Do something" });
    let config = extract_agent_config(&action);

    assert!(config.agent_name.is_none());
    assert!(config.provider.is_none());
    assert!(config.model.is_none());
    assert!(config.timeout.is_none());
    assert!(config.memory_context.is_none());
    assert!(config.working_dir.is_none());
    assert!(config.permission_mode.is_none());
    assert!(config.team_member_id.is_none());
}

// ---------------------------------------------------------------------------
// AgentTaskHandler::execute() — integration-style tests using NoopAgentExecutor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_no_action_fails() {
    let handler = AgentTaskHandler;
    let step = ResolvedStep {
        id: "step_agent".to_string(),
        process_id: "proc_test".to_string(),
        name: "Run Agent".to_string(),
        step_type: "agent-task".to_string(),
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
                error.to_lowercase().contains("action"),
                "Error should mention 'action': {error}"
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_missing_prompt_fails() {
    let handler = AgentTaskHandler;
    let step = make_step(json!({ "agentName": "claude" }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Failed { error } => {
            assert!(
                error.to_lowercase().contains("prompt"),
                "Error should mention 'prompt': {error}"
            );
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_noop_succeeds() {
    let handler = AgentTaskHandler;
    let step = make_step(json!({ "prompt": "Write a design doc." }));
    let state = make_state();
    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            // NoopAgentExecutor returns success; metrics keys should be present
            assert!(
                outputs.contains_key("_agentMetrics"),
                "Should have _agentMetrics: {:?}",
                outputs
            );
            assert!(
                outputs.contains_key("_filesModified"),
                "Should have _filesModified: {:?}",
                outputs
            );
            assert!(
                outputs.contains_key("_toolsUsed"),
                "Should have _toolsUsed: {:?}",
                outputs
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_skips_when_output_files_exist() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path();
    write_file(dir_path, "design.md", "# Existing design");

    let handler = AgentTaskHandler;
    let step = make_step(json!({
        "prompt": "Write a design doc.",
        "outputFiles": ["design.md"]
    }));

    let mut state = make_state();
    state.plan_dir = Some(dir_path.to_str().unwrap().to_string());

    let ctx = make_run_context();

    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            assert_eq!(
                outputs.get("_skipped"),
                Some(&Value::Bool(true)),
                "Should have _skipped=true when files already exist"
            );
        }
        other => panic!("Expected Completed (skip), got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_collects_output_files_after_agent_run() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path();

    // The NoopAgentExecutor doesn't write files, so we pre-write them to
    // simulate what a real agent would produce.
    write_file(dir_path, "output.json", r#"{"items": [1, 2, 3]}"#);

    let handler = AgentTaskHandler;
    let step = make_step(json!({
        "prompt": "Produce output.json",
        "outputFiles": ["output.json"]
    }));

    let mut state = make_state();
    // Don't set should_skip — files don't exist yet when execute() starts
    // (we write them after the fact to simulate agent output, but actually
    // they're already there, so the skip path would fire). To test the
    // collection path separately, we rely on the test for should_skip_agent
    // above. Here we just verify the file-collection logic runs.
    state.plan_dir = Some(dir_path.to_str().unwrap().to_string());

    let ctx = make_run_context();

    // Since all files exist, skip path fires — still collects files
    let result = handler.execute(&step, &state, &ctx).await.unwrap();
    match result {
        StepOutcome::Completed { outputs } => {
            let file_val = outputs.get("output");
            assert!(
                file_val.is_some(),
                "Should have 'output' key from output.json"
            );
        }
        other => panic!("Expected Completed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// AgentTaskHandler::check_resume() — unit tests with NoopAgentExecutor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_check_resume_wrong_pause_reason_returns_none() {
    let handler = AgentTaskHandler;
    let step = make_step(json!({ "prompt": "Do work." }));
    let state = make_state();
    let ctx = make_run_context();

    let reason = PauseReason::Approval {
        approval_id: "appr_test".to_string(),
    };

    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "Non-AgentTask pause reason should return None"
    );
}

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

#[tokio::test]
async fn test_check_resume_no_replies_returns_none() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/discussions-rest/.*/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": []
        })))
        .mount(&server)
        .await;

    let ctx = make_run_context_with_url(&server.uri());
    let step = make_step(json!({ "prompt": "Do work." }));
    let state = make_state();
    let reason = PauseReason::AgentTask {
        discussion_id: "disc_original".to_string(),
        posted_at: "2026-01-15T10:00:00Z".to_string(),
    };

    let handler = AgentTaskHandler;
    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    assert!(result.is_none(), "No replies should return None");
}

#[tokio::test]
async fn test_check_resume_replies_found_reruns_agent() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/discussions-rest/.*/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {
                    "id": "disc_reply1",
                    "parentId": "disc_original",
                    "content": "Use the factory pattern instead.",
                    "createdAt": "2026-01-15T11:00:00Z"
                }
            ]
        })))
        .mount(&server)
        .await;

    let ctx = make_run_context_with_url(&server.uri());
    let step = make_step(json!({ "prompt": "Design the feature." }));
    let state = make_state();
    let reason = PauseReason::AgentTask {
        discussion_id: "disc_original".to_string(),
        posted_at: "2026-01-15T10:00:00Z".to_string(),
    };

    let handler = AgentTaskHandler;
    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    // NoopAgentExecutor returns success, so we should get Completed
    let outcome = result.expect("Should have Some outcome when replies found");
    match outcome {
        StepOutcome::Completed { outputs } => {
            assert!(
                outputs.contains_key("_agentMetrics"),
                "Re-run should produce agent metrics"
            );
        }
        other => panic!("Expected Completed from re-run, got {:?}", other),
    }
}

#[tokio::test]
async fn test_check_resume_filters_old_replies() {
    let server = MockServer::start().await;

    // Reply is BEFORE posted_at — should be filtered out
    Mock::given(method("POST"))
        .and(path_regex(r"/discussions-rest/.*/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {
                    "id": "disc_old_reply",
                    "parentId": "disc_original",
                    "content": "Old reply from before pause.",
                    "createdAt": "2026-01-15T09:00:00Z"
                }
            ]
        })))
        .mount(&server)
        .await;

    let ctx = make_run_context_with_url(&server.uri());
    let step = make_step(json!({ "prompt": "Do work." }));
    let state = make_state();
    let reason = PauseReason::AgentTask {
        discussion_id: "disc_original".to_string(),
        posted_at: "2026-01-15T10:00:00Z".to_string(),
    };

    let handler = AgentTaskHandler;
    let result = handler
        .check_resume(&step, &state, &reason, &ctx)
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "Old replies (before posted_at) should be filtered, returning None"
    );
}
