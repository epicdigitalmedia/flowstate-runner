use flowstate_runner::clients::mcp::McpClient;
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_set_context() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/tools/set-context"))
        .and(body_json(json!({
            "orgId": "org_test",
            "workspaceId": "work_test"
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "success": true }))
                .append_header("mcp-session-id", "session_abc"),
        )
        .mount(&mock_server)
        .await;

    let mut client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    client.set_context().await.unwrap();
    assert_eq!(client.session_id(), Some("session_abc"));
}

#[tokio::test]
async fn test_call_tool_without_session() {
    // Tests the no-session-id path: call_tool works without prior set_context,
    // but the mcp-session-id header is not sent.
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/tools/proposal-create"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "documentId": "prop_new123"
        })))
        .mount(&mock_server)
        .await;

    let client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let result = client
        .call_tool(
            "proposal-create",
            json!({
                "title": "Test proposal",
                "agentId": "agent_1"
            }),
        )
        .await
        .unwrap();

    assert_eq!(result["documentId"], "prop_new123");
}

#[tokio::test]
async fn test_call_tool_sends_session_header() {
    let mock_server = MockServer::start().await;

    // First set context to get a session ID
    Mock::given(method("POST"))
        .and(path("/tools/set-context"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "success": true }))
                .append_header("mcp-session-id", "session_xyz"),
        )
        .mount(&mock_server)
        .await;

    // Then call_tool should include the session header
    Mock::given(method("POST"))
        .and(path("/tools/mission-status"))
        .and(header("mcp-session-id", "session_xyz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "running" })))
        .mount(&mock_server)
        .await;

    let mut client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    client.set_context().await.unwrap();

    let result = client
        .call_tool("mission-status", json!({ "missionId": "miss_1" }))
        .await
        .unwrap();

    assert_eq!(result["status"], "running");
}

// ---------------------------------------------------------------------------
// Typed convenience method tests
// ---------------------------------------------------------------------------

/// Minimal struct used for deserialization in tests.
#[derive(Debug, Deserialize, PartialEq)]
struct TaskDoc {
    id: String,
    title: String,
}

#[tokio::test]
async fn test_query_returns_deserialized_vec() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/tools/collection-query"))
        .and(body_json(json!({
            "collection": "tasks",
            "selector": { "status": "In Progress" },
            "limit": 5
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                { "id": "task_1", "title": "Task One" },
                { "id": "task_2", "title": "Task Two" }
            ]
        })))
        .mount(&mock_server)
        .await;

    let client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let docs: Vec<TaskDoc> = client
        .query("tasks", json!({ "status": "In Progress" }), Some(5))
        .await
        .unwrap();

    assert_eq!(docs.len(), 2);
    assert_eq!(
        docs[0],
        TaskDoc {
            id: "task_1".into(),
            title: "Task One".into()
        }
    );
    assert_eq!(
        docs[1],
        TaskDoc {
            id: "task_2".into(),
            title: "Task Two".into()
        }
    );
}

#[tokio::test]
async fn test_query_without_limit_omits_limit_field() {
    let mock_server = MockServer::start().await;

    // The body must NOT include a "limit" key when None is passed.
    Mock::given(method("POST"))
        .and(path("/tools/collection-query"))
        .and(body_json(json!({
            "collection": "tasks",
            "selector": {}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "documents": [] })))
        .mount(&mock_server)
        .await;

    let client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let docs: Vec<TaskDoc> = client.query("tasks", json!({}), None).await.unwrap();

    assert!(docs.is_empty());
}

#[tokio::test]
async fn test_query_missing_documents_field_returns_empty_vec() {
    let mock_server = MockServer::start().await;

    // Response without "documents" key — should gracefully return empty Vec.
    Mock::given(method("POST"))
        .and(path("/tools/collection-query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
        .mount(&mock_server)
        .await;

    let client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let docs: Vec<TaskDoc> = client.query("tasks", json!({}), None).await.unwrap();

    assert!(docs.is_empty());
}

#[tokio::test]
async fn test_get_returns_deserialized_document() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/tools/collection-get"))
        .and(body_json(json!({
            "collection": "tasks",
            "id": "task_abc"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "document": { "id": "task_abc", "title": "My Task" }
        })))
        .mount(&mock_server)
        .await;

    let client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let doc: TaskDoc = client.get("tasks", "task_abc").await.unwrap();

    assert_eq!(
        doc,
        TaskDoc {
            id: "task_abc".into(),
            title: "My Task".into()
        }
    );
}

#[tokio::test]
async fn test_get_missing_document_field_returns_error() {
    let mock_server = MockServer::start().await;

    // Response without "document" key — should return an Err.
    Mock::given(method("POST"))
        .and(path("/tools/collection-get"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
        .mount(&mock_server)
        .await;

    let client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let result: Result<TaskDoc, _> = client.get("tasks", "task_abc").await;

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("missing 'document' field"));
}

#[tokio::test]
async fn test_create_returns_document_id() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/tools/collection-create"))
        .and(body_json(json!({
            "collection": "tasks",
            "data": { "title": "New Task" }
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "documentId": "task_new999" })),
        )
        .mount(&mock_server)
        .await;

    let client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let id = client
        .create("tasks", &json!({ "title": "New Task" }))
        .await
        .unwrap();

    assert_eq!(id, "task_new999");
}

#[tokio::test]
async fn test_create_missing_document_id_returns_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/tools/collection-create"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
        .mount(&mock_server)
        .await;

    let client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let result = client
        .create("tasks", &json!({ "title": "New Task" }))
        .await;

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("missing 'documentId'"));
}

#[tokio::test]
async fn test_update_sends_correct_body_and_returns_unit() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/tools/collection-update"))
        .and(body_json(json!({
            "collection": "tasks",
            "id": "task_abc",
            "data": { "status": "Complete" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
        .mount(&mock_server)
        .await;

    let client = McpClient::new(&mock_server.uri(), "org_test", "work_test");
    let result = client
        .update("tasks", "task_abc", &json!({ "status": "Complete" }))
        .await;

    assert!(result.is_ok());
}
