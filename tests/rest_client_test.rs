use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::models::process::Process;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_query_returns_documents() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/processes-rest/0/query"))
        .and(body_json(
            json!({ "selector": { "status": "active" }, "limit": 100 }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {
                    "id": "proc_abc123xyz0",
                    "name": "test-process",
                    "status": "active",
                    "orgId": "org_test",
                    "workspaceId": "work_test",
                    "createdAt": "2026-01-15T10:00:00Z",
                    "updatedAt": "2026-01-15T10:00:00Z"
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let client = FlowstateRestClient::new(&mock_server.uri());
    let results: Vec<Process> = client
        .query("processes", json!({ "status": "active" }))
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "proc_abc123xyz0");
}

#[tokio::test]
async fn test_set_sends_array_body() {
    let mock_server = MockServer::start().await;

    let doc = json!({
        "id": "proc_abc123xyz0",
        "name": "test-process",
        "status": "active",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z"
    });

    Mock::given(method("POST"))
        .and(path("/processes-rest/0/set"))
        .and(body_json(json!([doc.clone()])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&mock_server)
        .await;

    let client = FlowstateRestClient::new(&mock_server.uri());
    client.set("processes", &[doc]).await.unwrap();
}

#[tokio::test]
async fn test_delete_does_get_then_set_deleted() {
    let mock_server = MockServer::start().await;

    let doc = json!({
        "id": "proc_to_delete",
        "name": "doomed",
        "status": "active",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "createdAt": "2026-01-15T10:00:00Z",
        "updatedAt": "2026-01-15T10:00:00Z",
        "_rev": "1-abc"
    });

    // Step 1: GET the document
    Mock::given(method("GET"))
        .and(path("/processes-rest/0/proc_to_delete"))
        .respond_with(ResponseTemplate::new(200).set_body_json(doc.clone()))
        .mount(&mock_server)
        .await;

    // Step 2: SET with _deleted: true — verify the payload includes _deleted
    let mut expected_doc = doc.clone();
    expected_doc["_deleted"] = json!(true);
    Mock::given(method("POST"))
        .and(path("/processes-rest/0/set"))
        .and(body_json(json!([expected_doc])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&mock_server)
        .await;

    let client = FlowstateRestClient::new(&mock_server.uri());
    client.delete("processes", "proc_to_delete").await.unwrap();
}

#[tokio::test]
async fn test_virtual_collection_routes_through_records() {
    let mock_server = MockServer::start().await;

    // Mock the schemas endpoint to register "myvcol" as virtual
    Mock::given(method("POST"))
        .and(path("/schemas-rest/0/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                { "id": "schm_test123", "name": "myvcol", "orgId": "org_test" }
            ]
        })))
        .mount(&mock_server)
        .await;

    // Mock the records-rest set endpoint for VCA writes
    Mock::given(method("POST"))
        .and(path("/records-rest/0/set"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&mock_server)
        .await;

    let mut client = FlowstateRestClient::new(&mock_server.uri());
    client.load_schemas("org_test").await.unwrap();

    // After loading schemas, "myvcol" is virtual — set() routes through records-rest
    let doc = json!({ "id": "rec_test", "orgId": "org_test", "customField": "value" });
    client.set("myvcol", &[doc]).await.unwrap();

    // Verify the request went to records-rest (the mock was matched)
    assert!(client.is_virtual("myvcol"));
    assert!(!client.is_virtual("tasks"));
}

#[tokio::test]
async fn test_query_empty_result() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/processes-rest/0/query"))
        .and(body_json(
            json!({ "selector": { "status": "nonexistent" }, "limit": 100 }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "documents": [] })))
        .mount(&mock_server)
        .await;

    let client = FlowstateRestClient::new(&mock_server.uri());
    let results: Vec<Process> = client
        .query("processes", json!({ "status": "nonexistent" }))
        .await
        .unwrap();

    assert!(results.is_empty());
}

#[tokio::test]
async fn test_get_single_document() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/processes-rest/0/proc_abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "proc_abc",
            "name": "test",
            "status": "active",
            "orgId": "org_test",
            "workspaceId": "work_test",
            "createdAt": "2026-01-15T10:00:00Z",
            "updatedAt": "2026-01-15T10:00:00Z"
        })))
        .mount(&mock_server)
        .await;

    let client = FlowstateRestClient::new(&mock_server.uri());
    let result: Process = client.get("processes", "proc_abc").await.unwrap();
    assert_eq!(result.id, "proc_abc");
}

#[tokio::test]
async fn test_virtual_query_flattens_data_bag() {
    let mock_server = MockServer::start().await;

    // Register "processes" as virtual
    Mock::given(method("POST"))
        .and(path("/schemas-rest/0/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                { "id": "schm_proc123", "name": "processes", "orgId": "org_test" }
            ]
        })))
        .mount(&mock_server)
        .await;

    // Mock records-rest query — returns VCA-formatted records
    Mock::given(method("POST"))
        .and(path("/records-rest/0/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {
                    "id": "proc_abc",
                    "orgId": "org_test",
                    "workspaceId": "work_test",
                    "schemaId": "schm_proc123",
                    "status": "active",
                    "title": "Test Process",
                    "createdAt": "2026-01-15T10:00:00Z",
                    "updatedAt": "2026-01-15T10:00:00Z",
                    "data": {
                        "name": "test-process",
                        "description": "A test process",
                        "version": "1"
                    }
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let mut client = FlowstateRestClient::new(&mock_server.uri());
    client.load_schemas("org_test").await.unwrap();

    let results: Vec<Process> = client
        .query("processes", json!({ "status": "active" }))
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "proc_abc");
    assert_eq!(results[0].name, "test-process");
}

/// Test that VCA flattening correctly handles processexecution records
/// with legacy bash-runner data (string tags instead of arrays).
#[tokio::test]
async fn test_virtual_query_processexecution_with_string_tags() {
    use flowstate_runner::models::execution::ProcessExecutionRecord;

    let mock_server = MockServer::start().await;

    // Register "processexecutions" as virtual
    Mock::given(method("POST"))
        .and(path("/schemas-rest/0/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                { "id": "schm_exec123", "name": "processexecutions", "orgId": "org_test" }
            ]
        })))
        .mount(&mock_server)
        .await;

    // Return a VCA record shaped like a bash-runner processexecution,
    // including string tags (not array) to test the string_or_vec deserializer
    // through the full VCA flatten pipeline.
    Mock::given(method("POST"))
        .and(path("/records-rest/0/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {
                    "id": "exec_test001",
                    "orgId": "org_test",
                    "workspaceId": "work_test",
                    "schemaId": "schm_exec123",
                    "status": "paused",
                    "title": "Test Execution",
                    "archived": false,
                    "completed": false,
                    "createdAt": "2026-03-01T00:00:00Z",
                    "updatedAt": "2026-03-01T00:00:00Z",
                    "data": {
                        "processId": "proc_abc",
                        "processVersion": "1.0.0",
                        "retryCount": 0,
                        "maxRetries": 3,
                        "variables": {
                            "tags": "brainstorm",
                            "processName": "brainstorming-workflow"
                        },
                        "stepHistory": [
                            {
                                "stepId": "step_1",
                                "stepName": "start",
                                "status": "completed",
                                "startedAt": "2026-03-01T00:00:01Z"
                            }
                        ],
                        "context": {
                            "entityType": "task",
                            "userId": "user_123",
                            "tags": "brainstorm",
                            "category": "",
                            "processName": "brainstorming-workflow"
                        }
                    }
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let mut client = FlowstateRestClient::new(&mock_server.uri());
    client.load_schemas("org_test").await.unwrap();

    let results: Vec<ProcessExecutionRecord> = client
        .query("processexecutions", json!({ "status": "paused" }))
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    let exec = &results[0];
    assert_eq!(exec.id, "exec_test001");
    assert_eq!(exec.process_id, "proc_abc");
    assert_eq!(exec.status, "paused");

    // Verify string tags were deserialized via string_or_vec
    let ctx = exec.context.as_ref().unwrap();
    assert_eq!(ctx.tags, vec!["brainstorm".to_string()]);

    // variables.tags is a Value::String — should not crash
    let vtags = exec.variables.get("tags").unwrap();
    assert_eq!(vtags.as_str().unwrap(), "brainstorm");
}

/// Test that query_virtual skips records that fail deserialization
/// instead of aborting the whole query.
#[tokio::test]
async fn test_virtual_query_skips_bad_records() {
    use flowstate_runner::models::execution::ProcessExecutionRecord;

    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/schemas-rest/0/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                { "id": "schm_exec123", "name": "processexecutions", "orgId": "org_test" }
            ]
        })))
        .mount(&mock_server)
        .await;

    // Two records: one valid, one with malformed stepHistory (string instead of array)
    Mock::given(method("POST"))
        .and(path("/records-rest/0/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {
                    "id": "exec_good",
                    "orgId": "org_test",
                    "workspaceId": "work_test",
                    "schemaId": "schm_exec123",
                    "status": "paused",
                    "createdAt": "2026-03-01T00:00:00Z",
                    "updatedAt": "2026-03-01T00:00:00Z",
                    "data": {
                        "processId": "proc_abc",
                        "stepHistory": []
                    }
                },
                {
                    "id": "exec_bad",
                    "orgId": "org_test",
                    "workspaceId": "work_test",
                    "schemaId": "schm_exec123",
                    "status": "paused",
                    "createdAt": "2026-03-01T00:00:00Z",
                    "updatedAt": "2026-03-01T00:00:00Z",
                    "data": {
                        "processId": "proc_abc",
                        "stepHistory": "this-is-not-an-array"
                    }
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let mut client = FlowstateRestClient::new(&mock_server.uri());
    client.load_schemas("org_test").await.unwrap();

    let results: Vec<ProcessExecutionRecord> = client
        .query("processexecutions", json!({ "status": "paused" }))
        .await
        .unwrap();

    // Bad record is skipped, good record is returned
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "exec_good");
}
