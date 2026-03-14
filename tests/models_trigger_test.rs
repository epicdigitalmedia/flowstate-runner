use flowstate_runner::models::trigger::*;
use serde_json::json;

#[test]
fn test_op_parse_all_variants() {
    assert_eq!(Op::parse("equals").unwrap(), Op::Equals);
    assert_eq!(Op::parse("not-equals").unwrap(), Op::NotEquals);
    assert_eq!(Op::parse("changes-to").unwrap(), Op::ChangesTo);
    assert_eq!(Op::parse("changes-from").unwrap(), Op::ChangesFrom);
    assert_eq!(Op::parse("gt").unwrap(), Op::Gt);
    assert_eq!(Op::parse("gte").unwrap(), Op::Gte);
    assert_eq!(Op::parse("lt").unwrap(), Op::Lt);
    assert_eq!(Op::parse("lte").unwrap(), Op::Lte);
    assert_eq!(Op::parse("exists").unwrap(), Op::Exists);
    assert_eq!(Op::parse("not-exists").unwrap(), Op::NotExists);
    assert_eq!(Op::parse("contains").unwrap(), Op::Contains);
    assert_eq!(Op::parse("in").unwrap(), Op::In);
    assert_eq!(Op::parse("not-in").unwrap(), Op::NotIn);
    assert_eq!(Op::parse("regex").unwrap(), Op::Regex);
}

#[test]
fn test_op_parse_unknown_returns_error() {
    assert!(Op::parse("invalid-op").is_err());
}

#[test]
fn test_trigger_condition_deserialize() {
    let json = json!({
        "field": "status",
        "operator": "equals",
        "value": "in-progress"
    });

    let cond: TriggerCondition = serde_json::from_value(json).unwrap();
    assert_eq!(cond.field, "status");
    assert_eq!(cond.operator, Op::Equals);
    assert!(cond.value_from.is_none());
}

#[test]
fn test_trigger_condition_with_value_from() {
    let json = json!({
        "field": "assignedAgent",
        "operator": "equals",
        "value": "",
        "valueFrom": "expectedAgent"
    });

    let cond: TriggerCondition = serde_json::from_value(json).unwrap();
    assert_eq!(cond.value_from, Some("expectedAgent".to_string()));
}

#[test]
fn test_op_serializes_as_kebab_case() {
    let op = Op::ChangesTo;
    let serialized = serde_json::to_value(&op).unwrap();
    assert_eq!(serialized, "changes-to");

    let op = Op::NotExists;
    let serialized = serde_json::to_value(&op).unwrap();
    assert_eq!(serialized, "not-exists");
}
