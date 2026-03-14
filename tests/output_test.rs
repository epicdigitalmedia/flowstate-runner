use flowstate_runner::models::agent::{ExtractionMode, OutputExtraction};
use flowstate_runner::output::{extract_output, map_outputs, resolve_json_path};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

// --- resolve_json_path ---

#[test]
fn test_json_path_simple_field() {
    let val = json!({"name": "Alice"});
    assert_eq!(resolve_json_path(&val, "name"), Some(&json!("Alice")));
}

#[test]
fn test_json_path_nested() {
    let val = json!({"data": {"id": "abc"}});
    assert_eq!(resolve_json_path(&val, "data.id"), Some(&json!("abc")));
}

#[test]
fn test_json_path_array_index() {
    let val = json!({"items": [{"id": "first"}, {"id": "second"}]});
    assert_eq!(
        resolve_json_path(&val, "items[0].id"),
        Some(&json!("first"))
    );
    assert_eq!(
        resolve_json_path(&val, "items[1].id"),
        Some(&json!("second"))
    );
}

#[test]
fn test_json_path_missing() {
    let val = json!({"name": "Alice"});
    assert_eq!(resolve_json_path(&val, "missing"), None);
}

#[test]
fn test_json_path_array_out_of_bounds() {
    let val = json!({"items": [1, 2]});
    assert_eq!(resolve_json_path(&val, "items[5]"), None);
}

// --- map_outputs ---

#[test]
fn test_map_outputs_flat_source() {
    let output_specs = vec![json!({
        "name": "taskId",
        "source": "entityId"
    })];
    let handler_output: HashMap<String, Value> =
        [("entityId".to_string(), json!("task_123"))].into();
    let mut vars = Map::new();
    map_outputs(&output_specs, &handler_output, &mut vars).unwrap();
    assert_eq!(vars["taskId"], json!("task_123"));
}

#[test]
fn test_map_outputs_with_target_variable() {
    let output_specs = vec![json!({
        "name": "result",
        "source": "output",
        "targetVariable": "finalResult"
    })];
    let handler_output: HashMap<String, Value> = [("output".to_string(), json!("success"))].into();
    let mut vars = Map::new();
    map_outputs(&output_specs, &handler_output, &mut vars).unwrap();
    assert_eq!(vars["finalResult"], json!("success"));
}

#[test]
fn test_map_outputs_no_source_uses_name() {
    let output_specs = vec![json!({
        "name": "commandOutput"
    })];
    let handler_output: HashMap<String, Value> =
        [("commandOutput".to_string(), json!("output data"))].into();
    let mut vars = Map::new();
    map_outputs(&output_specs, &handler_output, &mut vars).unwrap();
    assert_eq!(vars["commandOutput"], json!("output data"));
}

#[test]
fn test_map_outputs_with_json_path() {
    let output_specs = vec![json!({
        "name": "docId",
        "source": "apiResponse",
        "jsonPath": "documents[0].id"
    })];
    let handler_output: HashMap<String, Value> = [(
        "apiResponse".to_string(),
        json!({"documents": [{"id": "doc_abc"}]}),
    )]
    .into();
    let mut vars = Map::new();
    map_outputs(&output_specs, &handler_output, &mut vars).unwrap();
    assert_eq!(vars["docId"], json!("doc_abc"));
}

#[test]
fn test_map_outputs_preserves_existing_vars() {
    let output_specs = vec![json!({
        "name": "newVar",
        "source": "output"
    })];
    let handler_output: HashMap<String, Value> = [("output".to_string(), json!("new"))].into();
    let mut vars = Map::new();
    vars.insert("existingVar".to_string(), json!("preserved"));
    map_outputs(&output_specs, &handler_output, &mut vars).unwrap();
    assert_eq!(vars["existingVar"], json!("preserved"));
    assert_eq!(vars["newVar"], json!("new"));
}

#[test]
fn test_map_outputs_missing_source_skipped() {
    let output_specs = vec![json!({
        "name": "result",
        "source": "nonexistent"
    })];
    let handler_output: HashMap<String, Value> = HashMap::new();
    let mut vars = Map::new();
    // Should not error — missing source is skipped
    map_outputs(&output_specs, &handler_output, &mut vars).unwrap();
    // result should not be set when source is missing and no default
    assert!(
        !vars.contains_key("result"),
        "missing source with no default should not set variable"
    );
}

#[test]
fn test_map_outputs_with_default_value() {
    let output_specs = vec![json!({
        "name": "result",
        "source": "nonexistent",
        "defaultValue": "fallback"
    })];
    let handler_output: HashMap<String, Value> = HashMap::new();
    let mut vars = Map::new();
    map_outputs(&output_specs, &handler_output, &mut vars).unwrap();
    assert_eq!(vars["result"], json!("fallback"));
}

// --- extract_output ---

#[test]
fn test_extract_regex_capture() {
    let extraction = OutputExtraction {
        mode: ExtractionMode::Regex,
        source: Some("raw".to_string()),
        expression: Some(r"ID:\s*(\S+)".to_string()),
        merge_result: false,
    };
    let raw = json!({"raw": "Created document ID: doc_abc123 successfully"});
    let result = extract_output(&raw, &extraction).unwrap();
    assert_eq!(result, json!("doc_abc123"));
}

#[test]
fn test_extract_regex_no_match() {
    let extraction = OutputExtraction {
        mode: ExtractionMode::Regex,
        source: Some("raw".to_string()),
        expression: Some(r"NOTFOUND:\s*(\S+)".to_string()),
        merge_result: false,
    };
    let raw = json!({"raw": "no match here"});
    let result = extract_output(&raw, &extraction);
    assert!(result.is_err() || result.unwrap().is_null());
}

#[test]
fn test_extract_regex_full_match_no_group() {
    let extraction = OutputExtraction {
        mode: ExtractionMode::Regex,
        source: Some("raw".to_string()),
        expression: Some(r"\d+".to_string()),
        merge_result: false,
    };
    let raw = json!({"raw": "count is 42 items"});
    let result = extract_output(&raw, &extraction).unwrap();
    assert_eq!(result, json!("42"));
}
