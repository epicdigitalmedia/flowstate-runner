use flowstate_runner::conditions::{evaluate_condition, resolve_comparison_value, resolve_dotpath};
use flowstate_runner::models::trigger::Op;
use serde_json::{json, Map, Value};

fn vars(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

// --- resolve_dotpath ---

#[test]
fn test_dotpath_single_level() {
    let entity = json!({"name": "Alice"});
    assert_eq!(resolve_dotpath(&entity, "name"), Some(&json!("Alice")));
}

#[test]
fn test_dotpath_nested() {
    let entity = json!({"metadata": {"agent": "claude"}});
    assert_eq!(
        resolve_dotpath(&entity, "metadata.agent"),
        Some(&json!("claude"))
    );
}

#[test]
fn test_dotpath_deeply_nested() {
    let entity = json!({"a": {"b": {"c": 42}}});
    assert_eq!(resolve_dotpath(&entity, "a.b.c"), Some(&json!(42)));
}

#[test]
fn test_dotpath_missing() {
    let entity = json!({"name": "Alice"});
    assert_eq!(resolve_dotpath(&entity, "missing"), None);
}

#[test]
fn test_dotpath_partial_missing() {
    let entity = json!({"a": {"b": 1}});
    assert_eq!(resolve_dotpath(&entity, "a.c"), None);
}

// --- resolve_comparison_value ---

#[test]
fn test_comparison_value_literal() {
    let result = resolve_comparison_value(&json!("expected"), None, None);
    assert_eq!(result, json!("expected"));
}

#[test]
fn test_comparison_value_from_variable() {
    let v = vars(&[("status", json!("approved"))]);
    let result = resolve_comparison_value(&json!("unused"), Some("status"), Some(&v));
    assert_eq!(result, json!("approved"));
}

#[test]
fn test_comparison_value_from_missing_falls_back() {
    let v = vars(&[]);
    let result = resolve_comparison_value(&json!("fallback"), Some("missing"), Some(&v));
    assert_eq!(result, json!("fallback"));
}

// --- evaluate_condition with each operator ---

#[test]
fn test_equals_match() {
    let entity = json!({"status": "approved"});
    assert!(evaluate_condition(
        &entity,
        "status",
        &Op::Equals,
        &json!("approved"),
        None,
        None
    ));
}

#[test]
fn test_equals_no_match() {
    let entity = json!({"status": "pending"});
    assert!(!evaluate_condition(
        &entity,
        "status",
        &Op::Equals,
        &json!("approved"),
        None,
        None
    ));
}

#[test]
fn test_not_equals() {
    let entity = json!({"status": "pending"});
    assert!(evaluate_condition(
        &entity,
        "status",
        &Op::NotEquals,
        &json!("approved"),
        None,
        None
    ));
    assert!(
        !evaluate_condition(
            &entity,
            "status",
            &Op::NotEquals,
            &json!("pending"),
            None,
            None
        ),
        "same value should not satisfy not-equals"
    );
}

#[test]
fn test_gt_numbers() {
    let entity = json!({"count": 10});
    assert!(evaluate_condition(
        &entity,
        "count",
        &Op::Gt,
        &json!(5),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "count",
        &Op::Gt,
        &json!(10),
        None,
        None
    ));
}

#[test]
fn test_gte_numbers() {
    let entity = json!({"count": 10});
    assert!(evaluate_condition(
        &entity,
        "count",
        &Op::Gte,
        &json!(10),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "count",
        &Op::Gte,
        &json!(11),
        None,
        None
    ));
}

#[test]
fn test_lt_numbers() {
    let entity = json!({"count": 5});
    assert!(evaluate_condition(
        &entity,
        "count",
        &Op::Lt,
        &json!(10),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "count",
        &Op::Lt,
        &json!(5),
        None,
        None
    ));
}

#[test]
fn test_lte_numbers() {
    let entity = json!({"count": 5});
    assert!(evaluate_condition(
        &entity,
        "count",
        &Op::Lte,
        &json!(5),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "count",
        &Op::Lte,
        &json!(4),
        None,
        None
    ));
}

#[test]
fn test_contains_string_in_string() {
    let entity = json!({"desc": "hello world"});
    assert!(evaluate_condition(
        &entity,
        "desc",
        &Op::Contains,
        &json!("world"),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "desc",
        &Op::Contains,
        &json!("missing"),
        None,
        None
    ));
}

#[test]
fn test_contains_element_in_array() {
    let entity = json!({"tags": ["a", "b", "c"]});
    assert!(evaluate_condition(
        &entity,
        "tags",
        &Op::Contains,
        &json!("b"),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "tags",
        &Op::Contains,
        &json!("d"),
        None,
        None
    ));
}

#[test]
fn test_in_value_in_array() {
    let entity = json!({"status": "approved"});
    assert!(evaluate_condition(
        &entity,
        "status",
        &Op::In,
        &json!(["approved", "rejected"]),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "status",
        &Op::In,
        &json!(["pending", "draft"]),
        None,
        None
    ));
}

#[test]
fn test_not_in() {
    let entity = json!({"status": "draft"});
    assert!(evaluate_condition(
        &entity,
        "status",
        &Op::NotIn,
        &json!(["approved", "rejected"]),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "status",
        &Op::NotIn,
        &json!(["draft", "pending"]),
        None,
        None
    ));
}

#[test]
fn test_exists() {
    let entity = json!({"name": "Alice"});
    assert!(evaluate_condition(
        &entity,
        "name",
        &Op::Exists,
        &json!(null),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "missing",
        &Op::Exists,
        &json!(null),
        None,
        None
    ));
}

#[test]
fn test_not_exists() {
    let entity = json!({"name": "Alice"});
    assert!(!evaluate_condition(
        &entity,
        "name",
        &Op::NotExists,
        &json!(null),
        None,
        None
    ));
    assert!(evaluate_condition(
        &entity,
        "missing",
        &Op::NotExists,
        &json!(null),
        None,
        None
    ));
}

#[test]
fn test_regex_match() {
    let entity = json!({"email": "alice@example.com"});
    assert!(evaluate_condition(
        &entity,
        "email",
        &Op::Regex,
        &json!("^[^@]+@example\\.com$"),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "email",
        &Op::Regex,
        &json!("^admin@"),
        None,
        None
    ));
}

#[test]
fn test_changes_to_same_as_equals() {
    let entity = json!({"status": "approved"});
    assert!(evaluate_condition(
        &entity,
        "status",
        &Op::ChangesTo,
        &json!("approved"),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "status",
        &Op::ChangesTo,
        &json!("rejected"),
        None,
        None
    ));
}

#[test]
fn test_changes_from_same_as_not_equals() {
    let entity = json!({"status": "approved"});
    assert!(evaluate_condition(
        &entity,
        "status",
        &Op::ChangesFrom,
        &json!("pending"),
        None,
        None
    ));
    assert!(!evaluate_condition(
        &entity,
        "status",
        &Op::ChangesFrom,
        &json!("approved"),
        None,
        None
    ));
}

#[test]
fn test_evaluate_with_value_from() {
    let entity = json!({"approvalStatus": "approved"});
    let v = vars(&[("expectedStatus", json!("approved"))]);
    assert!(evaluate_condition(
        &entity,
        "approvalStatus",
        &Op::Equals,
        &json!("unused"),
        Some("expectedStatus"),
        Some(&v)
    ));
}

#[test]
fn test_evaluate_dotpath_in_condition() {
    let entity = json!({"metadata": {"assignedAgent": "claude"}});
    assert!(evaluate_condition(
        &entity,
        "metadata.assignedAgent",
        &Op::Equals,
        &json!("claude"),
        None,
        None
    ));
}

#[test]
fn test_evaluate_missing_field_returns_false_for_comparison() {
    let entity = json!({"name": "Alice"});
    assert!(!evaluate_condition(
        &entity,
        "missing",
        &Op::Equals,
        &json!("anything"),
        None,
        None
    ));
}

#[test]
fn test_gt_with_string_numbers() {
    // JSON numbers stored as strings should still compare numerically if possible
    let entity = json!({"count": "10"});
    assert!(
        evaluate_condition(&entity, "count", &Op::Gt, &json!(5), None, None),
        "should coerce '10' string to numeric 10 for comparison"
    );
    assert!(
        !evaluate_condition(&entity, "count", &Op::Gt, &json!(15), None, None),
        "numeric comparison: 10 > 15 is false"
    );
}

#[test]
fn test_null_field_with_equals() {
    let entity = json!({"value": null});
    assert!(
        !evaluate_condition(
            &entity,
            "value",
            &Op::Equals,
            &json!("expected"),
            None,
            None
        ),
        "null should not equal a string"
    );
    assert!(
        evaluate_condition(&entity, "value", &Op::Equals, &json!(null), None, None),
        "null should equal null"
    );
}

#[test]
fn test_null_field_exists() {
    let entity = json!({"value": null});
    // null IS a value that exists in the JSON — Exists checks path resolution, not null-ness
    assert!(
        evaluate_condition(&entity, "value", &Op::Exists, &json!(null), None, None),
        "null field exists in the object"
    );
}

#[test]
fn test_regex_invalid_pattern() {
    let entity = json!({"name": "Alice"});
    // Invalid regex should return false (fail-safe), not crash
    assert!(
        !evaluate_condition(
            &entity,
            "name",
            &Op::Regex,
            &json!("[invalid(regex"),
            None,
            None
        ),
        "invalid regex should return false, not panic"
    );
}
