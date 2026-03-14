use flowstate_runner::attributes::AttributeMap;
use serde_json::json;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn test_from_records_builds_tag_lookups() {
    let records = vec![
        json!({"id": "attr_tag1", "name": "pending", "type": "tag"}),
        json!({"id": "attr_tag2", "name": "approved", "type": "tag"}),
    ];
    let map = AttributeMap::from_records(&records);
    assert_eq!(map.tag_name_to_id("pending"), Some("attr_tag1"));
    assert_eq!(map.tag_name_to_id("approved"), Some("attr_tag2"));
    assert_eq!(map.tag_id_to_name("attr_tag1"), Some("pending"));
    assert_eq!(map.tag_id_to_name("attr_tag2"), Some("approved"));
}

#[test]
fn test_from_records_builds_category_lookups() {
    let records = vec![json!({"id": "attr_cat1", "name": "Sales", "type": "category"})];
    let map = AttributeMap::from_records(&records);
    assert_eq!(map.category_name_to_id("Sales"), Some("attr_cat1"));
    assert_eq!(map.category_id_to_name("attr_cat1"), Some("Sales"));
}

#[test]
fn test_missing_lookups_return_none() {
    let map = AttributeMap::from_records(&[]);
    assert_eq!(map.tag_name_to_id("nonexistent"), None);
    assert_eq!(map.tag_id_to_name("attr_nope"), None);
    assert_eq!(map.category_name_to_id("nonexistent"), None);
    assert_eq!(map.category_id_to_name("attr_nope"), None);
}

#[test]
fn test_resolve_tag_names_to_ids() {
    let records = vec![
        json!({"id": "attr_t1", "name": "pending", "type": "tag"}),
        json!({"id": "attr_t2", "name": "review", "type": "tag"}),
    ];
    let map = AttributeMap::from_records(&records);
    let ids = map.resolve_tag_names(&["pending", "review", "unknown"]);
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"attr_t1".to_string()));
    assert!(ids.contains(&"attr_t2".to_string()));
}

#[test]
fn test_resolve_tag_ids_to_names() {
    let records = vec![
        json!({"id": "attr_t1", "name": "pending", "type": "tag"}),
        json!({"id": "attr_t2", "name": "review", "type": "tag"}),
    ];
    let map = AttributeMap::from_records(&records);
    let names = map.resolve_tag_ids(&["attr_t1", "attr_t2", "attr_unknown"]);
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"pending".to_string()));
    assert!(names.contains(&"review".to_string()));
}

#[test]
fn test_from_records_ignores_malformed_entries() {
    let records = vec![
        json!({"id": "attr_ok", "name": "valid", "type": "tag"}),
        json!({"name": "no_id", "type": "tag"}),
        json!({"id": "attr_notype", "name": "notype"}),
        json!("not_an_object"),
    ];
    let map = AttributeMap::from_records(&records);
    assert_eq!(map.tag_name_to_id("valid"), Some("attr_ok"));
    assert_eq!(map.tag_name_to_id("no_id"), None);
    assert_eq!(map.tag_name_to_id("notype"), None);
}

#[tokio::test]
async fn test_load_from_rest() {
    use flowstate_runner::clients::rest::FlowstateRestClient;

    let mock = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"attributes-rest/.*/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "documents": [
                { "id": "attr_t1", "name": "pending", "type": "tag" },
                { "id": "attr_c1", "name": "Sales", "type": "category" }
            ]
        })))
        .mount(&mock)
        .await;

    let rest = FlowstateRestClient::new(&mock.uri());
    let map = AttributeMap::load(&rest, "org_test", "work_test").await.unwrap();

    assert_eq!(map.tag_name_to_id("pending"), Some("attr_t1"));
    assert_eq!(map.category_name_to_id("Sales"), Some("attr_c1"));
}
