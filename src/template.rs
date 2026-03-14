use serde_json::{Map, Value};
use std::borrow::Cow;

use crate::models::execution::ResolvedStep;
use crate::models::process::{ProcessStep, StepTemplate};

/// Interpolate `${varName}` references in a string template.
/// Returns `Cow::Borrowed` when no substitutions are needed (zero-alloc fast path).
/// Non-string values are converted via Display (numbers, bools) or serde_json::to_string (objects/arrays).
/// Unresolved variables are preserved as-is.
pub fn interpolate_str<'a>(template: &'a str, vars: &Map<String, Value>) -> Cow<'a, str> {
    if !template.contains("${") {
        return Cow::Borrowed(template);
    }

    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_name = String::new();
            let mut found_close = false;
            for c in chars.by_ref() {
                if c == '}' {
                    found_close = true;
                    break;
                }
                var_name.push(c);
            }
            if found_close {
                if let Some(val) = vars.get(&var_name) {
                    result.push_str(&value_to_string(val));
                } else {
                    // Unresolved — preserve original
                    result.push_str("${");
                    result.push_str(&var_name);
                    result.push('}');
                }
            } else {
                // Unclosed — preserve
                result.push_str("${");
                result.push_str(&var_name);
            }
        } else {
            result.push(ch);
        }
    }

    Cow::Owned(result)
}

/// Recursively interpolate `${varName}` references in a JSON value.
///
/// **Full-value pattern:** If a string value is exactly `"${varName}"` (nothing else),
/// the variable's typed value is returned directly (preserving numbers, bools, objects).
///
/// **Mixed pattern:** If a string contains `${varName}` among other text, the result
/// is always a string with the variable stringified.
///
/// Non-string values (numbers, bools, null) pass through unchanged.
/// Objects and arrays are walked recursively.
pub fn interpolate_json(value: &Value, vars: &Map<String, Value>) -> Value {
    match value {
        Value::String(s) => {
            // Check for full-value pattern: exactly "${varName}"
            if let Some(var_name) = parse_full_value_ref(s) {
                if let Some(val) = vars.get(var_name) {
                    return val.clone();
                }
                // Unresolved full-value — return original string
                return value.clone();
            }
            // Mixed text — interpolate as string
            let interpolated = interpolate_str(s, vars);
            Value::String(interpolated.into_owned())
        }
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), interpolate_json(v, vars));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| interpolate_json(v, vars)).collect()),
        // Numbers, bools, null pass through
        other => other.clone(),
    }
}

/// Check if a string is exactly `${varName}` with no surrounding text.
/// Returns the variable name if so.
///
/// Note: Leading and trailing whitespace is trimmed before matching, which
/// means `"  ${var}  "` resolves to the typed value of `var` without the
/// surrounding spaces. This matches shell variable resolution behavior.
fn parse_full_value_ref(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if trimmed.starts_with("${") && trimmed.ends_with('}') && trimmed.matches("${").count() == 1 {
        Some(&trimmed[2..trimmed.len() - 1])
    } else {
        None
    }
}

/// Convert a JSON Value to its string representation for interpolation.
fn value_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        // Objects and arrays get JSON-serialized
        other => serde_json::to_string(other).unwrap_or_else(|_| String::new()),
    }
}

/// Deep-merge two JSON Values. `override_val` keys win on collision.
/// Both must be objects; non-object values return `override_val`.
pub fn deep_merge_json(base: &Value, override_val: &Value) -> Value {
    match (base, override_val) {
        (Value::Object(b), Value::Object(o)) => {
            let mut merged = b.clone();
            for (k, v) in o {
                let existing = merged.get(k);
                let new_val = match existing {
                    Some(existing_val) => deep_merge_json(existing_val, v),
                    None => v.clone(),
                };
                merged.insert(k.clone(), new_val);
            }
            Value::Object(merged)
        }
        _ => override_val.clone(),
    }
}

/// Resolve a ProcessStep by merging with its StepTemplate (if provided).
///
/// Merge semantics:
/// - action: deep merge (template base, step overrides win)
/// - inputs: shallow merge (step wins on key collision)
/// - outputs: keyed merge on "name" (step replaces matching, template remainder kept)
/// - output_extraction: step overrides template entirely
/// - required_variables: union of both
pub fn resolve_template(step: &ProcessStep, template: Option<&StepTemplate>) -> ResolvedStep {
    let (action, inputs, outputs, output_extraction, required_variables) = match template {
        Some(tmpl) => {
            // action: deep merge
            let action = match (&tmpl.action, &step.action) {
                (Some(t), Some(s)) => Some(deep_merge_json(t, s)),
                (None, s) => s.clone(),
                (t, None) => t.clone(),
            };

            // inputs: shallow merge (template base, step wins)
            let inputs = match (&tmpl.inputs, &step.inputs) {
                (Some(Value::Object(t)), Some(Value::Object(s))) => {
                    let mut merged = t.clone();
                    for (k, v) in s {
                        merged.insert(k.clone(), v.clone());
                    }
                    Some(Value::Object(merged))
                }
                (None, s) => s.clone(),
                (t, None) => t.clone(),
                _ => step.inputs.clone(),
            };

            // outputs: keyed merge on "name"
            let outputs = merge_outputs_by_name(&tmpl.outputs, &step.outputs);

            // output_extraction: step overrides entirely
            let output_extraction = if step.output_extraction.is_some() {
                step.output_extraction.clone()
            } else {
                tmpl.output_extraction.clone()
            };

            // required_variables: union
            let mut req_vars: Vec<String> = tmpl.required_variables.clone();
            for v in &step.required_variables {
                if !req_vars.contains(v) {
                    req_vars.push(v.clone());
                }
            }

            (action, inputs, outputs, output_extraction, req_vars)
        }
        None => (
            step.action.clone(),
            step.inputs.clone(),
            step.outputs.clone(),
            step.output_extraction.clone(),
            step.required_variables.clone(),
        ),
    };

    ResolvedStep {
        id: step.id.clone(),
        process_id: step.process_id.clone(),
        name: step.name.clone(),
        step_type: step.step_type.clone(),
        action,
        inputs,
        outputs,
        output_extraction,
        conditions: step.conditions.clone(),
        next_step_id: step.next_step_id.clone(),
        required_variables,
        estimated_duration_minutes: step.estimated_duration_minutes,
        metadata: step.metadata.as_object().cloned().unwrap_or_default(),
    }
}

/// Merge output specs by "name" key. Step entries replace template entries
/// with the same name. Template entries not overridden are kept.
fn merge_outputs_by_name(template_outputs: &[Value], step_outputs: &[Value]) -> Vec<Value> {
    let step_names: Vec<Option<&str>> = step_outputs
        .iter()
        .map(|o| o.get("name").and_then(Value::as_str))
        .collect();

    let mut result: Vec<Value> = template_outputs
        .iter()
        .filter(|t| {
            let name = t.get("name").and_then(Value::as_str);
            match name {
                Some(n) => !step_names.contains(&Some(n)),
                None => true,
            }
        })
        .cloned()
        .collect();

    result.extend(step_outputs.iter().cloned());
    result
}
