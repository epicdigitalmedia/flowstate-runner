use crate::models::trigger::Op;
use regex::Regex;
use serde_json::{Map, Value};

/// Resolve a dot-notation path (e.g., "metadata.agent") against a JSON Value.
/// Returns a reference to the nested value, or None if the path doesn't exist.
pub fn resolve_dotpath<'a>(entity: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = entity;
    for segment in path.split('.') {
        match current.get(segment) {
            Some(next) => current = next,
            None => return None,
        }
    }
    Some(current)
}

/// Resolve the comparison value. If `value_from` is set and the named variable
/// exists in `variables`, use that. Otherwise, fall back to `literal_value`.
pub fn resolve_comparison_value(
    literal_value: &Value,
    value_from: Option<&str>,
    variables: Option<&Map<String, Value>>,
) -> Value {
    if let Some(var_name) = value_from {
        if let Some(vars) = variables {
            if let Some(val) = vars.get(var_name) {
                return val.clone();
            }
            tracing::debug!(
                variable = var_name,
                "valueFrom variable not found, falling back to literal"
            );
        }
    }
    literal_value.clone()
}

/// Evaluate a single condition against an entity.
///
/// - `entity`: the JSON object to evaluate against (variables map, entity record, etc.)
/// - `field`: dot-notation path to the field in `entity`
/// - `op`: the comparison operator
/// - `expected`: the literal comparison value
/// - `value_from`: if set, resolve comparison value from this variable name
/// - `variables`: execution variables for `value_from` resolution
pub fn evaluate_condition(
    entity: &Value,
    field: &str,
    op: &Op,
    expected: &Value,
    value_from: Option<&str>,
    variables: Option<&Map<String, Value>>,
) -> bool {
    let comparison_value = resolve_comparison_value(expected, value_from, variables);

    // Exists/NotExists don't need the actual field value
    match op {
        Op::Exists => return resolve_dotpath(entity, field).is_some(),
        Op::NotExists => return resolve_dotpath(entity, field).is_none(),
        _ => {}
    }

    let actual = match resolve_dotpath(entity, field) {
        Some(v) => v,
        None => return false,
    };

    match op {
        // ChangesTo and ChangesFrom are evaluated as point-in-time equality checks
        // because the runner is stateless — it has no access to the previous entity
        // value. Full change detection would require the scanner to provide (old, new)
        // pairs, which the REST API does not support. This is intentional: ChangesTo
        // degrades to "field currently equals value" and ChangesFrom degrades to
        // "field currently does not equal value", which is a safe approximation.
        Op::Equals | Op::ChangesTo => values_equal(actual, &comparison_value),
        Op::NotEquals | Op::ChangesFrom => !values_equal(actual, &comparison_value),
        Op::Gt => compare_numeric(actual, &comparison_value).is_some_and(|o| o > 0),
        Op::Gte => compare_numeric(actual, &comparison_value).is_some_and(|o| o >= 0),
        Op::Lt => compare_numeric(actual, &comparison_value).is_some_and(|o| o < 0),
        Op::Lte => compare_numeric(actual, &comparison_value).is_some_and(|o| o <= 0),
        Op::Contains => eval_contains(actual, &comparison_value),
        Op::In => eval_in(actual, &comparison_value),
        Op::NotIn => !eval_in(actual, &comparison_value),
        Op::Regex => eval_regex(actual, &comparison_value),
        Op::Exists | Op::NotExists => unreachable!(),
    }
}

/// Compare two Values for equality. Handles cross-type comparisons
/// (e.g., string "10" vs number 10).
fn values_equal(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    // Cross-type: try numeric comparison
    if let (Some(an), Some(bn)) = (to_f64(a), to_f64(b)) {
        return (an - bn).abs() < f64::EPSILON;
    }
    // Cross-type: try string comparison
    match (a.as_str(), b.as_str()) {
        (Some(sa), _) if sa == value_as_string(b) => true,
        (_, Some(sb)) if value_as_string(a) == sb => true,
        _ => false,
    }
}

/// Numeric comparison. Returns ordering: -1, 0, or 1.
fn compare_numeric(a: &Value, b: &Value) -> Option<i8> {
    let an = to_f64(a)?;
    let bn = to_f64(b)?;
    if (an - bn).abs() < f64::EPSILON {
        Some(0)
    } else if an > bn {
        Some(1)
    } else {
        Some(-1)
    }
}

/// Extract a numeric value from a JSON Value.
/// Handles Number, String-encoded numbers, and booleans (true=1, false=0).
fn to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// String representation of a value for cross-type comparison.
fn value_as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Contains: string-in-string or element-in-array.
fn eval_contains(actual: &Value, expected: &Value) -> bool {
    match actual {
        Value::String(s) => {
            if let Some(substr) = expected.as_str() {
                s.contains(substr)
            } else {
                s.contains(&value_as_string(expected))
            }
        }
        Value::Array(arr) => arr.iter().any(|item| values_equal(item, expected)),
        _ => false,
    }
}

/// In: check if `actual` value is in the `expected` array.
fn eval_in(actual: &Value, expected: &Value) -> bool {
    match expected {
        Value::Array(arr) => arr.iter().any(|item| values_equal(actual, item)),
        _ => false,
    }
}

/// Regex: match `actual` string against `expected` regex pattern.
/// Returns `false` on invalid regex patterns (fail-safe behavior — an invalid
/// pattern is logged but never panics or propagates an error).
fn eval_regex(actual: &Value, pattern: &Value) -> bool {
    let actual_str = match actual {
        Value::String(s) => s.as_str(),
        _ => return false,
    };
    let pattern_str = match pattern {
        Value::String(s) => s.as_str(),
        _ => return false,
    };
    match Regex::new(pattern_str) {
        Ok(re) => re.is_match(actual_str),
        Err(e) => {
            tracing::debug!(pattern = pattern_str, error = %e, "Invalid regex pattern, returning false");
            false
        }
    }
}
