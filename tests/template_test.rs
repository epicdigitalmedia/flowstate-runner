use flowstate_runner::models::process::{ProcessStep, StepTemplate};
use flowstate_runner::template::{interpolate_json, interpolate_str, resolve_template};
use serde_json::{json, Map, Value};
use std::borrow::Cow;

fn vars(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

// --- interpolate_str tests ---

#[test]
fn test_interpolate_str_no_vars() {
    let v = Map::new();
    let result = interpolate_str("hello world", &v);
    assert!(
        matches!(result, Cow::Borrowed(_)),
        "should borrow when no variables"
    );
    assert_eq!(result, "hello world");
}

#[test]
fn test_interpolate_str_single_var() {
    let v = vars(&[("name", json!("Alice"))]);
    let result = interpolate_str("hello ${name}", &v);
    assert_eq!(result, "hello Alice");
}

#[test]
fn test_interpolate_str_multiple_vars() {
    let v = vars(&[("first", json!("Alice")), ("last", json!("Smith"))]);
    let result = interpolate_str("${first} ${last}", &v);
    assert_eq!(result, "Alice Smith");
}

#[test]
fn test_interpolate_str_unresolved_preserved() {
    let v = vars(&[("name", json!("Alice"))]);
    let result = interpolate_str("hello ${name}, ${unknown}", &v);
    assert_eq!(result, "hello Alice, ${unknown}");
}

#[test]
fn test_interpolate_str_no_dollar_sign() {
    let v = vars(&[("x", json!("y"))]);
    let result = interpolate_str("no variables here", &v);
    assert!(matches!(result, Cow::Borrowed(_)));
    assert_eq!(result, "no variables here");
}

#[test]
fn test_interpolate_str_number_to_string() {
    let v = vars(&[("count", json!(42))]);
    let result = interpolate_str("total: ${count}", &v);
    assert_eq!(result, "total: 42");
}

#[test]
fn test_interpolate_str_bool_to_string() {
    let v = vars(&[("flag", json!(true))]);
    let result = interpolate_str("is: ${flag}", &v);
    assert_eq!(result, "is: true");
}

// --- interpolate_json tests ---

#[test]
fn test_interpolate_json_string_value() {
    let v = vars(&[("name", json!("Alice"))]);
    let input = json!("hello ${name}");
    let result = interpolate_json(&input, &v);
    assert_eq!(result, json!("hello Alice"));
}

#[test]
fn test_interpolate_json_full_value_returns_typed() {
    let v = vars(&[("count", json!(42))]);
    let input = json!("${count}");
    let result = interpolate_json(&input, &v);
    assert_eq!(
        result,
        json!(42),
        "full-value pattern should return typed value"
    );
}

#[test]
fn test_interpolate_json_full_value_object() {
    let obj = json!({"nested": "data"});
    let v = vars(&[("config", obj.clone())]);
    let input = json!("${config}");
    let result = interpolate_json(&input, &v);
    assert_eq!(result, obj);
}

#[test]
fn test_interpolate_json_full_value_bool() {
    let v = vars(&[("flag", json!(true))]);
    let input = json!("${flag}");
    let result = interpolate_json(&input, &v);
    assert_eq!(result, json!(true));
}

#[test]
fn test_interpolate_json_mixed_text_returns_string() {
    let v = vars(&[("count", json!(42))]);
    let input = json!("total: ${count} items");
    let result = interpolate_json(&input, &v);
    assert_eq!(result, json!("total: 42 items"));
}

#[test]
fn test_interpolate_json_nested_object() {
    let v = vars(&[("name", json!("Alice"))]);
    let input = json!({
        "greeting": "hello ${name}",
        "raw": "no vars"
    });
    let result = interpolate_json(&input, &v);
    assert_eq!(result["greeting"], "hello Alice");
    assert_eq!(result["raw"], "no vars");
}

#[test]
fn test_interpolate_json_array() {
    let v = vars(&[("x", json!("A"))]);
    let input = json!(["${x}", "literal", "${x} and more"]);
    let result = interpolate_json(&input, &v);
    assert_eq!(result[0], "A");
    assert_eq!(result[1], "literal");
    assert_eq!(result[2], "A and more");
}

#[test]
fn test_interpolate_json_non_string_passthrough() {
    let v = vars(&[("x", json!("unused"))]);
    let input = json!(42);
    let result = interpolate_json(&input, &v);
    assert_eq!(result, json!(42));
}

#[test]
fn test_interpolate_json_null_passthrough() {
    let v = Map::new();
    let input = json!(null);
    let result = interpolate_json(&input, &v);
    assert_eq!(result, json!(null));
}

#[test]
fn test_interpolate_json_deeply_nested() {
    let v = vars(&[("val", json!("deep"))]);
    let input = json!({
        "l1": {
            "l2": {
                "l3": "${val}"
            }
        }
    });
    let result = interpolate_json(&input, &v);
    assert_eq!(result["l1"]["l2"]["l3"], "deep");
}

// --- resolve_template tests ---

fn make_step(overrides: serde_json::Value) -> ProcessStep {
    let mut base = json!({
        "id": "step_test00001",
        "processId": "proc_test00001",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "name": "test-step",
        "stepType": "action",
        "order": 1,
        "optional": false,
        "enabled": true,
        "conditions": [],
        "outputs": [],
        "requiredVariables": [],
        "archived": false,
        "metadata": {},
        "extended": {},
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z"
    });
    if let (Value::Object(b), Value::Object(o)) = (&mut base, &overrides) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(base).unwrap()
}

fn make_template(overrides: serde_json::Value) -> StepTemplate {
    let mut base = json!({
        "id": "stpl_test0001",
        "name": "test-template",
        "stepType": "action",
        "orgId": "org_test",
        "workspaceId": "work_test",
        "outputs": [],
        "requiredVariables": [],
        "archived": false,
        "metadata": {},
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z"
    });
    if let (Value::Object(b), Value::Object(o)) = (&mut base, &overrides) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(base).unwrap()
}

#[test]
fn test_resolve_no_template() {
    let step = make_step(json!({
        "action": {"type": "command", "command": {"executable": "echo"}},
        "nextStepId": "step_next"
    }));
    let resolved = resolve_template(&step, None);
    assert_eq!(resolved.id, "step_test00001");
    assert_eq!(resolved.step_type, "action");
    assert_eq!(resolved.next_step_id, Some("step_next".to_string()));
    assert!(resolved.action.is_some());
}

#[test]
fn test_resolve_action_deep_merge() {
    let step = make_step(json!({
        "templateId": "stpl_test0001",
        "action": {"command": {"args": ["hello"]}}
    }));
    let template = make_template(json!({
        "action": {"type": "command", "command": {"executable": "/bin/echo"}}
    }));
    let resolved = resolve_template(&step, Some(&template));
    let action = resolved.action.unwrap();
    // Template provides base, step overrides
    assert_eq!(action["type"], "command");
    assert_eq!(action["command"]["executable"], "/bin/echo");
    assert_eq!(action["command"]["args"][0], "hello");
}

#[test]
fn test_resolve_inputs_step_wins() {
    let step = make_step(json!({
        "templateId": "stpl_test0001",
        "inputs": {"prompt": "step-value", "extra": "step-only"}
    }));
    let template = make_template(json!({
        "inputs": {"prompt": "template-value", "default": "template-only"}
    }));
    let resolved = resolve_template(&step, Some(&template));
    let inputs = resolved.inputs.unwrap();
    assert_eq!(inputs["prompt"], "step-value", "step wins on collision");
    assert_eq!(
        inputs["default"], "template-only",
        "template-only preserved"
    );
    assert_eq!(inputs["extra"], "step-only", "step-only preserved");
}

#[test]
fn test_resolve_outputs_keyed_merge() {
    let step = make_step(json!({
        "templateId": "stpl_test0001",
        "outputs": [{"name": "result", "source": "stepSource"}]
    }));
    let template = make_template(json!({
        "outputs": [
            {"name": "result", "source": "templateSource"},
            {"name": "extra", "source": "extraSource"}
        ]
    }));
    let resolved = resolve_template(&step, Some(&template));
    assert_eq!(resolved.outputs.len(), 2);
    // "result" replaced by step version
    let result_out = resolved
        .outputs
        .iter()
        .find(|o| o["name"] == "result")
        .unwrap();
    assert_eq!(result_out["source"], "stepSource");
    // "extra" kept from template
    let extra_out = resolved
        .outputs
        .iter()
        .find(|o| o["name"] == "extra")
        .unwrap();
    assert_eq!(extra_out["source"], "extraSource");
}

#[test]
fn test_resolve_output_extraction_step_overrides() {
    let step = make_step(json!({
        "templateId": "stpl_test0001",
        "outputExtraction": {"mode": "regex", "expression": "step-regex"}
    }));
    let template = make_template(json!({
        "outputExtraction": {"mode": "jq", "expression": "template-jq"}
    }));
    let resolved = resolve_template(&step, Some(&template));
    let extraction = resolved.output_extraction.unwrap();
    assert_eq!(
        extraction["mode"], "regex",
        "step overrides template entirely"
    );
}

#[test]
fn test_resolve_required_variables_union() {
    let step = make_step(json!({
        "templateId": "stpl_test0001",
        "requiredVariables": ["varA", "varC"]
    }));
    let template = make_template(json!({
        "requiredVariables": ["varA", "varB"]
    }));
    let resolved = resolve_template(&step, Some(&template));
    assert!(resolved.required_variables.contains(&"varA".to_string()));
    assert!(resolved.required_variables.contains(&"varB".to_string()));
    assert!(resolved.required_variables.contains(&"varC".to_string()));
}

#[test]
fn test_resolve_action_deep_merge_three_levels() {
    // Verify deep merge works at 3+ levels of nesting
    let step = make_step(json!({
        "templateId": "stpl_test0001",
        "action": {"command": {"options": {"verbose": true}}}
    }));
    let template = make_template(json!({
        "action": {
            "type": "command",
            "command": {"executable": "/bin/echo", "options": {"color": false}}
        }
    }));
    let resolved = resolve_template(&step, Some(&template));
    let action = resolved.action.unwrap();
    assert_eq!(action["type"], "command", "template top-level preserved");
    assert_eq!(
        action["command"]["executable"], "/bin/echo",
        "template nested preserved"
    );
    assert_eq!(
        action["command"]["options"]["verbose"], true,
        "step 3rd-level wins"
    );
    assert_eq!(
        action["command"]["options"]["color"], false,
        "template 3rd-level preserved"
    );
}

#[test]
fn test_resolve_empty_template_fields() {
    // Template with all None/empty fields — step provides everything
    let step = make_step(json!({
        "templateId": "stpl_test0001",
        "action": {"type": "command", "command": {"executable": "echo"}},
        "inputs": {"prompt": "hello"},
        "outputs": [{"name": "result", "source": "stdout"}]
    }));
    let template = make_template(json!({})); // Empty template
    let resolved = resolve_template(&step, Some(&template));
    assert!(
        resolved.action.is_some(),
        "step action preserved when template empty"
    );
    assert!(
        resolved.inputs.is_some(),
        "step inputs preserved when template empty"
    );
    assert_eq!(
        resolved.outputs.len(),
        1,
        "step outputs preserved when template empty"
    );
}
