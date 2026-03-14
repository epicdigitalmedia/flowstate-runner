#[test]
fn test_execution_record_with_string_tags_in_context() {
    use flowstate_runner::models::execution::ProcessExecutionRecord;
    use serde_json::json;

    let flat = json!({
        "id": "exec_0kIHXNcdQJ",
        "processId": "test",
        "orgId": "org_9f3omFEY2H",
        "workspaceId": "work_ojk4TWK5D2",
        "status": "paused",
        "createdAt": "2026-03-01T00:00:00Z",
        "updatedAt": "2026-03-01T00:00:00Z",
        "context": {
            "entityType": "task",
            "userId": "user_123",
            "tags": "brainstorm",
            "category": "",
            "processName": "brainstorming-workflow"
        }
    });

    let result: Result<ProcessExecutionRecord, _> = serde_json::from_value(flat);
    match &result {
        Ok(r) => {
            let ctx = r.context.as_ref().unwrap();
            assert_eq!(ctx.tags, vec!["brainstorm".to_string()]);
        }
        Err(e) => panic!("Deserialization failed: {}", e),
    }
}
