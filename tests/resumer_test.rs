use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::RunContext;
use flowstate_runner::models::process::StepTemplate;
use flowstate_runner::resumer::resume;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use wiremock::matchers::{body_partial_json, method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_test_context(base_url: &str) -> RunContext {
    RunContext {
        config: Config {
            org_id: "org_test".to_string(),
            workspace_id: "work_test".to_string(),
            rest_base_url: base_url.to_string(),
            mcp_base_url: base_url.to_string(),
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
        rest: FlowstateRestClient::new(base_url),
        http: reqwest::Client::new(),
        mcp: McpClient::new(base_url, "org_test", "work_test"),
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
async fn test_resume_no_paused_executions() {
    let mock = MockServer::start().await;

    // processexecutions is VCA — query goes through MCP
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": []
        })))
        .mount(&mock)
        .await;

    let ctx = make_test_context(&mock.uri());
    let templates: HashMap<String, StepTemplate> = HashMap::new();
    let report = resume(&ctx, &templates).await.unwrap();

    assert!(report.resumed.is_empty());
    assert_eq!(report.still_waiting, 0);
    assert!(report.errors.is_empty());
}

#[tokio::test]
async fn test_resume_approval_approved_continues_execution() {
    let mock = MockServer::start().await;

    // Paused execution with approval pause reason (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{
                "id": "exec_paused1",
                "processId": "proc_resume1",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "status": "paused",
                "currentStepId": "step_approval",
                "variables": {},
                "stepHistory": [],
                "retryCount": 0,
                "maxRetries": 3,
                "archived": false,
                "metadata": {
                    "_pause_reason": {
                        "type": "approval",
                        "approval_id": "appr_test123"
                    }
                },
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }]
        })))
        .mount(&mock)
        .await;

    // Process for loading name (VCA — MCP get)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(
            json!({ "collection": "processes", "id": "proc_resume1" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "proc_resume1",
                "name": "resume-test-process",
                "status": "active",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "startStepId": "step_approval",
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }
        })))
        .mount(&mock)
        .await;

    // Process steps (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(json!({ "collection": "processsteps" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{
                "id": "step_approval",
                "processId": "proc_resume1",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "name": "Approval Step",
                "stepType": "approval",
                "action": {
                    "strategy": "human",
                    "title": "Approve changes"
                },
                "conditions": [],
                "nextStepId": "step_end",
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }, {
                "id": "step_end",
                "processId": "proc_resume1",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "name": "End",
                "stepType": "end",
                "conditions": [],
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }]
        })))
        .mount(&mock)
        .await;

    // Approval record: approved (native collection — stays REST)
    Mock::given(method("GET"))
        .and(path_regex(r"approvals-rest/.*/appr_test123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "appr_test123",
            "status": "approved",
            "feedback": "Looks good"
        })))
        .mount(&mock)
        .await;

    // Accept execution updates via MCP collection-update (persist_state)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-update"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true
        })))
        .mount(&mock)
        .await;

    let ctx = make_test_context(&mock.uri());
    let templates: HashMap<String, StepTemplate> = HashMap::new();
    let report = resume(&ctx, &templates).await.unwrap();

    assert_eq!(report.resumed.len(), 1);
    assert_eq!(report.still_waiting, 0);
}

#[tokio::test]
async fn test_resume_still_waiting() {
    let mock = MockServer::start().await;

    // Paused execution with approval pending (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{
                "id": "exec_waiting",
                "processId": "proc_wait",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "status": "paused",
                "currentStepId": "step_appr",
                "variables": {},
                "stepHistory": [],
                "retryCount": 0,
                "maxRetries": 3,
                "archived": false,
                "metadata": {
                    "_pause_reason": {
                        "type": "approval",
                        "approval_id": "appr_pending"
                    }
                },
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }]
        })))
        .mount(&mock)
        .await;

    // Process (VCA — MCP get)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(
            json!({ "collection": "processes", "id": "proc_wait" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "proc_wait",
                "name": "wait-process",
                "status": "active",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "startStepId": "step_appr",
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }
        })))
        .mount(&mock)
        .await;

    // Steps (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(json!({ "collection": "processsteps" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{
                "id": "step_appr",
                "processId": "proc_wait",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "name": "Wait Approval",
                "stepType": "approval",
                "action": { "strategy": "human" },
                "conditions": [],
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }]
        })))
        .mount(&mock)
        .await;

    // Approval still pending (native collection — stays REST)
    Mock::given(method("GET"))
        .and(path_regex(r"approvals-rest/.*/appr_pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "appr_pending",
            "status": "pending"
        })))
        .mount(&mock)
        .await;

    let ctx = make_test_context(&mock.uri());
    let templates: HashMap<String, StepTemplate> = HashMap::new();
    let report = resume(&ctx, &templates).await.unwrap();

    assert!(report.resumed.is_empty(), "nothing should resume");
    assert_eq!(report.still_waiting, 1);
}

#[tokio::test]
async fn test_resume_subprocess_child_completed() {
    let mock = MockServer::start().await;

    // Paused parent execution with subprocess pause reason (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{
                "id": "exec_parent",
                "processId": "proc_sub",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "status": "paused",
                "currentStepId": "step_subprocess",
                "variables": { "childExecutionId": "exec_child1" },
                "stepHistory": [],
                "retryCount": 0,
                "maxRetries": 3,
                "archived": false,
                "metadata": {
                    "_pause_reason": {
                        "type": "subprocess",
                        "child_execution_id": "exec_child1"
                    }
                },
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }]
        })))
        .mount(&mock)
        .await;

    // Process (VCA — MCP get)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(
            json!({ "collection": "processes", "id": "proc_sub" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "proc_sub",
                "name": "subprocess-process",
                "status": "active",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "startStepId": "step_subprocess",
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }
        })))
        .mount(&mock)
        .await;

    // Steps (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(json!({ "collection": "processsteps" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{
                "id": "step_subprocess",
                "processId": "proc_sub",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "name": "Run Child",
                "stepType": "subprocess",
                "action": {
                    "processId": "proc_child",
                    "waitForCompletion": true
                },
                "conditions": [],
                "nextStepId": "step_end",
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }, {
                "id": "step_end",
                "processId": "proc_sub",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "name": "End",
                "stepType": "end",
                "conditions": [],
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }]
        })))
        .mount(&mock)
        .await;

    // Child execution: completed (VCA — MCP get)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-get"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions", "id": "exec_child1" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": {
                "id": "exec_child1",
                "processId": "proc_child",
                "orgId": "org_test",
                "workspaceId": "work_test",
                "status": "completed",
                "variables": { "childOutput": "result_data" },
                "stepHistory": [],
                "retryCount": 0,
                "maxRetries": 3,
                "archived": false,
                "metadata": {},
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }
        })))
        .mount(&mock)
        .await;

    // Accept execution updates via MCP collection-update (persist_state)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-update"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true
        })))
        .mount(&mock)
        .await;

    let ctx = make_test_context(&mock.uri());
    let templates: HashMap<String, StepTemplate> = HashMap::new();
    let report = resume(&ctx, &templates).await.unwrap();

    assert_eq!(
        report.resumed.len(),
        1,
        "parent should resume after child completes"
    );
    assert_eq!(report.still_waiting, 0);
}
