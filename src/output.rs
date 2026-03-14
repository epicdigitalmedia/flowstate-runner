use crate::models::agent::{ExtractionMode, OutputExtraction};
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Resolve a simple JSON path (dot-notation with array indices).
/// Supports: `field`, `field.nested`, `field[0].nested`.
/// Not a full jq implementation — covers the output mapping use case.
pub fn resolve_json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in split_json_path(path) {
        match &segment {
            PathSegment::Field(name) => {
                current = current.get(name.as_str())?;
            }
            PathSegment::Index(name, idx) => {
                current = current.get(name.as_str())?;
                current = current.get(*idx)?;
            }
        }
    }
    Some(current)
}

enum PathSegment {
    Field(String),
    Index(String, usize),
}

/// Split a path like "items[0].id" into segments.
///
/// Malformed bracket expressions (e.g., `items[`, `items[]`, `items[abc]`)
/// are treated as plain field names rather than panicking.
fn split_json_path(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    for part in path.split('.') {
        if let Some(bracket_pos) = part.find('[') {
            if let Some(close_pos) = part.find(']') {
                if close_pos > bracket_pos + 1 {
                    let field = &part[..bracket_pos];
                    let idx_str = &part[bracket_pos + 1..close_pos];
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        segments.push(PathSegment::Index(field.to_string(), idx));
                        continue;
                    }
                }
            }
            // Malformed bracket — treat as plain field
            segments.push(PathSegment::Field(part.to_string()));
        } else {
            segments.push(PathSegment::Field(part.to_string()));
        }
    }
    segments
}

/// Map handler outputs to execution variables using output spec definitions.
///
/// Each output spec in `output_specs` is a JSON Value with these fields:
/// - `name`: target variable name (or fallback source key)
/// - `source`: key in handler_output to read from (defaults to `name`)
/// - `jsonPath`: optional path to extract from the source value
/// - `targetVariable`: optional override for the target variable name
/// - `defaultValue`: optional fallback when source is missing
pub fn map_outputs(
    output_specs: &[Value],
    handler_output: &HashMap<String, Value>,
    variables: &mut Map<String, Value>,
) -> Result<()> {
    for spec in output_specs {
        let name = match spec.get("name").and_then(Value::as_str) {
            Some(n) if !n.is_empty() => n,
            _ => {
                tracing::warn!("Output spec missing 'name' field — skipping");
                continue;
            }
        };
        let source_key = spec.get("source").and_then(Value::as_str).unwrap_or(name);
        let target = spec
            .get("targetVariable")
            .and_then(Value::as_str)
            .unwrap_or(name);
        let json_path = spec.get("jsonPath").and_then(Value::as_str);
        let default_value = spec.get("defaultValue");

        let source_value = handler_output.get(source_key);

        let resolved = match (source_value, json_path) {
            (Some(val), Some(path)) => resolve_json_path(val, path).cloned(),
            (Some(val), None) => Some(val.clone()),
            (None, _) => None,
        };

        let final_value = resolved
            .or_else(|| default_value.cloned())
            .unwrap_or(Value::Null);

        if !final_value.is_null() || default_value.is_some() {
            variables.insert(target.to_string(), final_value);
        }
    }
    Ok(())
}

/// Apply output extraction to raw handler output.
///
/// Supports:
/// - `Regex`: Apply regex to source field, return first capture group (or full match)
/// - `Jq`: Not implemented in Phase 2 (returns error)
/// - `Script`: Not implemented in Phase 2 (returns error)
pub fn extract_output(raw: &Value, extraction: &OutputExtraction) -> Result<Value> {
    let source_key = extraction.source.as_deref().unwrap_or("raw");
    let source_value = raw
        .get(source_key)
        .and_then(Value::as_str)
        .context(format!(
            "Extraction source '{}' not found or not a string",
            source_key
        ))?;

    let expression = extraction
        .expression
        .as_deref()
        .context("Extraction expression is required")?;

    match extraction.mode {
        ExtractionMode::Regex => extract_regex(source_value, expression),
        ExtractionMode::Jq => bail!("Jq extraction not implemented in Phase 2"),
        ExtractionMode::Script => bail!("Script extraction not implemented in Phase 2"),
    }
}

/// Apply a regex to a string. Returns the first capture group, or the full match.
fn extract_regex(input: &str, pattern: &str) -> Result<Value> {
    let re = Regex::new(pattern).context("Invalid regex pattern")?;
    match re.captures(input) {
        Some(caps) => {
            // Return first capture group if it exists, else full match
            let matched = caps
                .get(1)
                .or_else(|| caps.get(0))
                .map(|m| m.as_str())
                .unwrap_or("");
            Ok(Value::String(matched.to_string()))
        }
        None => bail!("Regex did not match: {}", pattern),
    }
}
