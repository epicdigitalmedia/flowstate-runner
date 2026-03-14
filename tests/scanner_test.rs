use flowstate_runner::attributes::AttributeMap;
use flowstate_runner::clients::mcp::McpClient;
use flowstate_runner::clients::rest::FlowstateRestClient;
use flowstate_runner::config::Config;
use flowstate_runner::handlers::RunContext;
use flowstate_runner::models::process::EntityTriggerCondition;
use flowstate_runner::scanner::scan;
use flowstate_runner::scanner::{
    build_db_selector, evaluate_client_conditions, partition_conditions, pluralize_entity_type,
    seed_variables,
};
use serde_json::json;
use std::path::PathBuf;
use wiremock::matchers::{body_partial_json, method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn test_partition_eq_goes_to_db() {
    let conditions = vec![EntityTriggerCondition {
        property_path: "status".to_string(),
        operator: "eq".to_string(),
        value: json!("In Progress"),
    }];
    let map = AttributeMap::default();
    let (db, client) = partition_conditions(&conditions, &map);
    assert_eq!(db.len(), 1);
    assert_eq!(client.len(), 0);
    assert_eq!(db[0].property_path, "status");
}

#[test]
fn test_partition_neq_goes_to_db() {
    let conditions = vec![EntityTriggerCondition {
        property_path: "status".to_string(),
        operator: "neq".to_string(),
        value: json!("Complete"),
    }];
    let map = AttributeMap::default();
    let (db, client) = partition_conditions(&conditions, &map);
    assert_eq!(db.len(), 1);
    assert_eq!(client.len(), 0);
}

#[test]
fn test_partition_gt_goes_to_client() {
    let conditions = vec![EntityTriggerCondition {
        property_path: "progress".to_string(),
        operator: "gt".to_string(),
        value: json!(50),
    }];
    let map = AttributeMap::default();
    let (db, client) = partition_conditions(&conditions, &map);
    assert_eq!(db.len(), 0);
    assert_eq!(client.len(), 1);
}

#[test]
fn test_partition_tag_ids_goes_to_db_with_resolved_ids() {
    let attrs = vec![json!({"id": "attr_t1", "name": "pending", "type": "tag"})];
    let map = AttributeMap::from_records(&attrs);
    let conditions = vec![EntityTriggerCondition {
        property_path: "tagIds".to_string(),
        operator: "contains".to_string(),
        value: json!("pending"),
    }];
    let (db, client) = partition_conditions(&conditions, &map);
    assert_eq!(db.len(), 1);
    assert_eq!(client.len(), 0);
    assert_eq!(db[0].value, json!("attr_t1"));
}

#[test]
fn test_partition_tag_ids_unresolvable_goes_to_client() {
    let map = AttributeMap::default();
    let conditions = vec![EntityTriggerCondition {
        property_path: "tagIds".to_string(),
        operator: "contains".to_string(),
        value: json!("unknown_tag"),
    }];
    let (db, client) = partition_conditions(&conditions, &map);
    assert_eq!(db.len(), 0);
    assert_eq!(client.len(), 1);
}

#[test]
fn test_partition_multiple_tag_ids_only_first_goes_to_db() {
    let records = vec![
        json!({"id": "attr_t1", "name": "tag-a", "type": "tag"}),
        json!({"id": "attr_t2", "name": "tag-b", "type": "tag"}),
    ];
    let map = AttributeMap::from_records(&records);
    let conditions = vec![
        EntityTriggerCondition {
            property_path: "tagIds".to_string(),
            operator: "contains".to_string(),
            value: json!("tag-a"),
        },
        EntityTriggerCondition {
            property_path: "tagIds".to_string(),
            operator: "contains".to_string(),
            value: json!("tag-b"),
        },
    ];
    let (db, client) = partition_conditions(&conditions, &map);
    // Only first tagIds goes to DB; second must be evaluated client-side
    assert_eq!(db.len(), 1);
    assert_eq!(client.len(), 1);
    assert_eq!(db[0].value, json!("attr_t1"));
    // Second tag stays as original name for client-side evaluation
    assert_eq!(client[0].value, json!("tag-b"));
}

#[test]
fn test_partition_mixed_conditions() {
    let conditions = vec![
        EntityTriggerCondition {
            property_path: "status".to_string(),
            operator: "eq".to_string(),
            value: json!("In Progress"),
        },
        EntityTriggerCondition {
            property_path: "progress".to_string(),
            operator: "gt".to_string(),
            value: json!(50),
        },
        EntityTriggerCondition {
            property_path: "title".to_string(),
            operator: "neq".to_string(),
            value: json!("Draft"),
        },
    ];
    let map = AttributeMap::default();
    let (db, client) = partition_conditions(&conditions, &map);
    assert_eq!(db.len(), 2);
    assert_eq!(client.len(), 1);
}

#[test]
fn test_build_db_selector_simple() {
    let db_conditions = vec![EntityTriggerCondition {
        property_path: "status".to_string(),
        operator: "eq".to_string(),
        value: json!("In Progress"),
    }];
    let selector = build_db_selector(&db_conditions, "org_test", "work_test");
    let obj = selector.as_object().unwrap();
    assert_eq!(obj.get("status"), Some(&json!("In Progress")));
    assert_eq!(obj.get("orgId"), Some(&json!("org_test")));
    assert_eq!(obj.get("workspaceId"), Some(&json!("work_test")));
}

#[test]
fn test_build_db_selector_neq() {
    let db_conditions = vec![EntityTriggerCondition {
        property_path: "status".to_string(),
        operator: "neq".to_string(),
        value: json!("Complete"),
    }];
    let selector = build_db_selector(&db_conditions, "org_test", "work_test");
    let obj = selector.as_object().unwrap();
    assert_eq!(obj.get("status"), Some(&json!({"$ne": "Complete"})));
}

#[test]
fn test_build_db_selector_tag_ids_contains() {
    let db_conditions = vec![EntityTriggerCondition {
        property_path: "tagIds".to_string(),
        operator: "contains".to_string(),
        value: json!("attr_t1"),
    }];
    let selector = build_db_selector(&db_conditions, "org_test", "work_test");
    let obj = selector.as_object().unwrap();
    assert_eq!(
        obj.get("tagIds"),
        Some(&json!({"$elemMatch": {"$eq": "attr_t1"}}))
    );
}

#[test]
fn test_build_db_selector_empty_conditions() {
    let selector = build_db_selector(&[], "org_test", "work_test");
    let obj = selector.as_object().unwrap();
    assert_eq!(obj.get("orgId"), Some(&json!("org_test")));
    assert_eq!(obj.get("workspaceId"), Some(&json!("work_test")));
    assert_eq!(obj.len(), 2);
}

#[test]
fn test_evaluate_client_conditions_all_match() {
    let conditions = vec![EntityTriggerCondition {
        property_path: "progress".to_string(),
        operator: "gt".to_string(),
        value: json!(50),
    }];
    let entity = json!({"progress": 75});
    assert!(evaluate_client_conditions(&entity, &conditions));
}

#[test]
fn test_evaluate_client_conditions_one_fails() {
    let conditions = vec![
        EntityTriggerCondition {
            property_path: "progress".to_string(),
            operator: "gt".to_string(),
            value: json!(50),
        },
        EntityTriggerCondition {
            property_path: "status".to_string(),
            operator: "eq".to_string(),
            value: json!("Done"),
        },
    ];
    let entity = json!({"progress": 75, "status": "In Progress"});
    assert!(!evaluate_client_conditions(&entity, &conditions));
}

#[test]
fn test_evaluate_client_conditions_empty_returns_true() {
    let entity = json!({"anything": "value"});
    assert!(evaluate_client_conditions(&entity, &[]));
}

#[test]
fn test_evaluate_client_conditions_invalid_operator_returns_false() {
    let conditions = vec![EntityTriggerCondition {
        property_path: "status".to_string(),
        operator: "invalid_op".to_string(),
        value: json!("test"),
    }];
    let entity = json!({"status": "test"});
    assert!(!evaluate_client_conditions(&entity, &conditions));
}

#[test]
fn test_seed_variables_full_entity() {
    let attrs = vec![
        json!({"id": "attr_t1", "name": "pending", "type": "tag"}),
        json!({"id": "attr_t2", "name": "review", "type": "tag"}),
        json!({"id": "attr_c1", "name": "Sales", "type": "category"}),
    ];
    let map = AttributeMap::from_records(&attrs);
    let entity = json!({
        "id": "task_abc123",
        "title": "Review proposal",
        "description": "Review the sales proposal",
        "projectId": "proj_xyz",
        "milestoneId": "mile_def",
        "tagIds": ["attr_t1", "attr_t2"],
        "categoryId": "attr_c1",
        "metadata": {"assignedAgent": "agent_smith"}
    });
    let vars = seed_variables(&entity, "tasks", &map);
    assert_eq!(vars.get("entityId"), Some(&json!("task_abc123")));
    assert_eq!(vars.get("entityType"), Some(&json!("tasks")));
    // Type-specific alias: tasks collection seeds taskId
    assert_eq!(vars.get("taskId"), Some(&json!("task_abc123")));
    assert_eq!(vars.get("title"), Some(&json!("Review proposal")));
    assert_eq!(
        vars.get("description"),
        Some(&json!("Review the sales proposal"))
    );
    assert_eq!(vars.get("projectId"), Some(&json!("proj_xyz")));
    assert_eq!(vars.get("milestoneId"), Some(&json!("mile_def")));
    assert_eq!(vars.get("assignedAgent"), Some(&json!("agent_smith")));
    let tags = vars.get("tags").unwrap().as_array().unwrap();
    assert_eq!(tags.len(), 2);
    assert!(tags.contains(&json!("pending")));
    assert!(tags.contains(&json!("review")));
    assert_eq!(vars.get("category"), Some(&json!("Sales")));
}

#[test]
fn test_seed_variables_task_includes_context_and_alias() {
    let map = AttributeMap::default();
    let entity = json!({
        "id": "task_ctx1",
        "title": "Context test",
        "orgId": "org_abc",
        "workspaceId": "work_xyz"
    });
    let vars = seed_variables(&entity, "tasks", &map);
    assert_eq!(vars.get("entityId"), Some(&json!("task_ctx1")));
    assert_eq!(vars.get("taskId"), Some(&json!("task_ctx1")));
    assert_eq!(vars.get("orgId"), Some(&json!("org_abc")));
    assert_eq!(vars.get("workspaceId"), Some(&json!("work_xyz")));
}

#[test]
fn test_seed_variables_milestone_alias() {
    let map = AttributeMap::default();
    let entity = json!({
        "id": "mile_abc",
        "title": "Milestone test",
        "orgId": "org_abc",
        "workspaceId": "work_xyz"
    });
    let vars = seed_variables(&entity, "milestones", &map);
    assert_eq!(vars.get("entityId"), Some(&json!("mile_abc")));
    assert_eq!(vars.get("milestoneId"), Some(&json!("mile_abc")));
    assert!(vars.get("taskId").is_none());
}

#[test]
fn test_seed_variables_project_alias() {
    let map = AttributeMap::default();
    let entity = json!({
        "id": "proj_abc",
        "title": "Project test",
        "orgId": "org_abc"
    });
    let vars = seed_variables(&entity, "projects", &map);
    assert_eq!(vars.get("entityId"), Some(&json!("proj_abc")));
    assert_eq!(vars.get("projectId"), Some(&json!("proj_abc")));
    assert!(vars.get("taskId").is_none());
    assert!(vars.get("milestoneId").is_none());
    // workspaceId not present on entity — should not appear
    assert!(vars.get("workspaceId").is_none());
}

#[test]
fn test_seed_variables_minimal_entity() {
    let map = AttributeMap::default();
    let entity = json!({"id": "task_min", "title": "Minimal task"});
    let vars = seed_variables(&entity, "tasks", &map);
    assert_eq!(vars.get("entityId"), Some(&json!("task_min")));
    assert_eq!(vars.get("entityType"), Some(&json!("tasks")));
    assert_eq!(vars.get("title"), Some(&json!("Minimal task")));
    assert!(vars.get("description").is_none());
    assert!(vars.get("projectId").is_none());
    assert!(vars.get("assignedAgent").is_none());
    // No tagIds present → no tags key
    assert!(vars.get("tags").is_none());
}

#[test]
fn test_pluralize_standard() {
    assert_eq!(pluralize_entity_type("task"), "tasks");
    assert_eq!(pluralize_entity_type("project"), "projects");
    assert_eq!(pluralize_entity_type("milestone"), "milestones");
    assert_eq!(pluralize_entity_type("document"), "documents");
}

#[test]
fn test_pluralize_already_plural() {
    assert_eq!(pluralize_entity_type("tasks"), "tasks");
    assert_eq!(pluralize_entity_type("milestones"), "milestones");
}

#[test]
fn test_pluralize_entity_to_entities() {
    assert_eq!(pluralize_entity_type("entity"), "entities");
    assert_eq!(pluralize_entity_type("category"), "categories");
}

fn make_scan_context(base_url: &str) -> RunContext {
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
async fn test_scan_no_processes_returns_empty_report() {
    let mock = MockServer::start().await;
    // processes is VCA — query goes through MCP
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(json!({ "collection": "processes" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"documents": []})))
        .mount(&mock)
        .await;

    let ctx = make_scan_context(&mock.uri());
    let report = scan(&ctx).await.unwrap();
    assert!(report.created.is_empty());
    assert_eq!(report.skipped, 0);
    assert!(report.errors.is_empty());
}

#[tokio::test]
async fn test_scan_creates_execution_for_matching_entity() {
    let mock = MockServer::start().await;

    // Process with entity trigger (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(json!({ "collection": "processes" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{
                "id": "proc_scan1", "name": "test-process", "status": "active",
                "orgId": "org_test", "workspaceId": "work_test",
                "startStepId": "step_start",
                "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z",
                "trigger": {
                    "type": "entity",
                    "entityTrigger": {
                        "entityType": "task", "selector": {},
                        "conditions": [{"propertyPath": "status", "operator": "eq", "value": "In Progress"}]
                    }
                }
            }]
        }))).mount(&mock).await;

    // Matching entity (native collection — REST)
    Mock::given(method("POST"))
        .and(path_regex(r"tasks-rest/.*/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{"id": "task_match1", "title": "Matching Task", "status": "In Progress", "orgId": "org_test", "workspaceId": "work_test"}]
        }))).expect(1).mount(&mock).await;

    // No active executions (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"documents": []})))
        .mount(&mock)
        .await;

    // Accept creation (VCA — MCP create)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-create"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documentId": "exec_created1"
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let ctx = make_scan_context(&mock.uri());
    let report = scan(&ctx).await.unwrap();
    assert_eq!(report.created.len(), 1);
    assert_eq!(report.skipped, 0);
}

#[tokio::test]
async fn test_scan_skips_entity_with_active_execution() {
    let mock = MockServer::start().await;

    // Process (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(json!({ "collection": "processes" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{
                "id": "proc_scan2", "name": "test-process", "status": "active",
                "orgId": "org_test", "workspaceId": "work_test", "startStepId": "step_start",
                "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z",
                "trigger": {
                    "type": "entity",
                    "entityTrigger": {"entityType": "task", "selector": {},
                        "conditions": [{"propertyPath": "status", "operator": "eq", "value": "In Progress"}]}
                }
            }]
        }))).mount(&mock).await;

    // Entities (native — REST)
    Mock::given(method("POST"))
        .and(path_regex(r"tasks-rest/.*/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{"id": "task_dup1", "title": "Already Running", "status": "In Progress", "orgId": "org_test", "workspaceId": "work_test"}]
        }))).mount(&mock).await;

    // Active execution exists (VCA — MCP query).
    // externalId must be present so batch_active_check can populate the skip set.
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(json!({ "collection": "processexecutions" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{"id": "exec_existing", "processId": "proc_scan2", "externalId": "task_dup1", "status": "running"}]
        })))
        .mount(&mock)
        .await;

    let ctx = make_scan_context(&mock.uri());
    let report = scan(&ctx).await.unwrap();
    assert!(report.created.is_empty());
    assert_eq!(report.skipped, 1);
}

#[tokio::test]
async fn test_scan_filters_entities_client_side() {
    let mock = MockServer::start().await;

    // Process (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(json!({ "collection": "processes" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [{
                "id": "proc_scan3", "name": "gt-process", "status": "active",
                "orgId": "org_test", "workspaceId": "work_test", "startStepId": "step_start",
                "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z",
                "trigger": {
                    "type": "entity",
                    "entityTrigger": {"entityType": "task", "selector": {},
                        "conditions": [
                            {"propertyPath": "status", "operator": "eq", "value": "In Progress"},
                            {"propertyPath": "progress", "operator": "gt", "value": 50}
                        ]}
                }
            }]
        })))
        .mount(&mock)
        .await;

    // Entities (native — REST)
    Mock::given(method("POST"))
        .and(path_regex(r"tasks-rest/.*/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documents": [
                {"id": "task_high", "title": "High Progress", "status": "In Progress", "progress": 75, "orgId": "org_test", "workspaceId": "work_test"},
                {"id": "task_low", "title": "Low Progress", "status": "In Progress", "progress": 30, "orgId": "org_test", "workspaceId": "work_test"}
            ]
        }))).mount(&mock).await;

    // No active executions (VCA — MCP query)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-query"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"documents": []})))
        .mount(&mock)
        .await;

    // Accept creation (VCA — MCP create)
    Mock::given(method("POST"))
        .and(path_regex(r"tools/collection-create"))
        .and(body_partial_json(
            json!({ "collection": "processexecutions" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documentId": "exec_filtered1"
        })))
        .mount(&mock)
        .await;

    let ctx = make_scan_context(&mock.uri());
    let report = scan(&ctx).await.unwrap();
    assert_eq!(
        report.created.len(),
        1,
        "only high-progress entity should match"
    );
}
