use crate::handlers::{Handler, RunContext};
use crate::models::execution::{ExecutionState, ResolvedStep, StepOutcome};
use crate::template::{interpolate_json, interpolate_str};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub struct ActionHandler;

#[async_trait]
impl Handler for ActionHandler {
    async fn execute(
        &self,
        step: &ResolvedStep,
        state: &ExecutionState,
        ctx: &RunContext,
    ) -> Result<StepOutcome> {
        let action = match &step.action {
            Some(a) => a,
            None => {
                return Ok(StepOutcome::Failed {
                    error: "Action step missing action configuration".to_string(),
                });
            }
        };

        let action_type = match action.get("type").and_then(Value::as_str) {
            Some(t) => t,
            None => {
                return Ok(StepOutcome::Failed {
                    error: "Action step missing 'type' field in action".to_string(),
                });
            }
        };

        match action_type {
            "command" => execute_command(action, step, state).await,
            "script" => execute_script(action, step, state).await,
            "http" => execute_http(action, step, state, ctx).await,
            "mcp-tool" => execute_mcp_tool(action, step, state, ctx).await,
            other => Ok(StepOutcome::Failed {
                error: format!("Unknown action type: {}", other),
            }),
        }
    }
}

/// Spawn an external process, capture stdout/stderr/exit_code.
/// The `command` and each element of `args` are interpolated with execution variables.
///
/// Accepts two action formats:
/// - Rust format: `{"type":"command","command":"echo","args":["hello"]}`
/// - Bash runner format: `{"type":"command","command":{"command":"mkdir -p ${dir}"}}`
///   In the bash format, `action.command` is an object with its own `command` and
///   optional `args` fields.
async fn execute_command(
    action: &Value,
    step: &ResolvedStep,
    state: &ExecutionState,
) -> Result<StepOutcome> {
    // Normalize the two command formats
    let (command_str, args_value) = match action.get("command") {
        Some(Value::String(c)) => {
            // Rust format: command is a string, args at top level
            (c.as_str(), action.get("args"))
        }
        Some(Value::Object(obj)) => {
            // Bash runner format: command is an object with inner command/args
            let inner_cmd = obj
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("");
            let inner_args = obj.get("args");
            (inner_cmd, inner_args)
        }
        _ => {
            return Ok(StepOutcome::Failed {
                error: "Command action missing 'command' field".to_string(),
            });
        }
    };

    if command_str.is_empty() {
        return Ok(StepOutcome::Failed {
            error: "Command action has empty 'command' string".to_string(),
        });
    }

    let command = interpolate_str(command_str, &state.variables).into_owned();

    // Build interpolated args list
    let raw_args: Vec<String> = match args_value {
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| match v {
                Value::String(s) => interpolate_str(s, &state.variables).into_owned(),
                other => {
                    // Non-string args: interpolate as JSON then stringify
                    let interpolated = interpolate_json(other, &state.variables);
                    match interpolated {
                        Value::String(s) => s,
                        v => v.to_string(),
                    }
                }
            })
            .collect(),
        Some(_) => {
            return Ok(StepOutcome::Failed {
                error: "Command action 'args' must be an array".to_string(),
            });
        }
        None => vec![],
    };

    tracing::info!(
        step_id = %step.id,
        command = %command,
        args = ?raw_args,
        "Executing command"
    );

    // When args are empty but the command contains spaces (e.g., "mkdir -p /path"),
    // run through a shell so the string is parsed as command + arguments.
    // SAFETY: `command` originates from admin-authored process step definitions stored
    // in the database, not from user input or execution variables. The trust boundary
    // is the process definition itself.
    let output = if raw_args.is_empty() && command.contains(' ') {
        tokio::process::Command::new("sh")
            .args(["-c", &command])
            .output()
            .await
    } else {
        tokio::process::Command::new(&command)
            .args(&raw_args)
            .output()
            .await
    };

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
            let exit_code = out.status.code().unwrap_or(-1);

            tracing::debug!(
                step_id = %step.id,
                exit_code = exit_code,
                stdout_len = stdout.len(),
                stderr_len = stderr.len(),
                "Command completed"
            );

            if out.status.success() {
                let mut outputs = build_outputs(&stdout, &stderr, exit_code);
                apply_extraction(step, &stdout, &mut outputs);
                Ok(StepOutcome::Completed { outputs })
            } else {
                Ok(StepOutcome::Failed {
                    error: format!(
                        "Command exited with code {}: {}",
                        exit_code,
                        if stderr.is_empty() { &stdout } else { &stderr }
                    ),
                })
            }
        }
        Err(e) => Ok(StepOutcome::Failed {
            error: format!("Failed to spawn command '{}': {}", command, e),
        }),
    }
}

/// Write the inline script to a tempfile, make it executable, run via sh.
/// The script content is interpolated with execution variables before writing.
async fn execute_script(
    action: &Value,
    step: &ResolvedStep,
    state: &ExecutionState,
) -> Result<StepOutcome> {
    let script_content = match action.get("script").and_then(Value::as_str) {
        Some(s) => interpolate_str(s, &state.variables).into_owned(),
        None => {
            return Ok(StepOutcome::Failed {
                error: "Script action missing 'script' field".to_string(),
            });
        }
    };

    // Write to a named tempfile with .sh extension so shells can recognise it
    let mut tmp = match tempfile::Builder::new().suffix(".sh").tempfile() {
        Ok(f) => f,
        Err(e) => {
            return Ok(StepOutcome::Failed {
                error: format!("Failed to create temp script file: {}", e),
            });
        }
    };

    if let Err(e) = tmp.write_all(script_content.as_bytes()) {
        return Ok(StepOutcome::Failed {
            error: format!("Failed to write temp script: {}", e),
        });
    }

    // Flush to disk before setting permissions
    if let Err(e) = tmp.flush() {
        return Ok(StepOutcome::Failed {
            error: format!("Failed to flush temp script: {}", e),
        });
    }

    // Make executable (owner rwx) — Unix only
    let path = tmp.path().to_path_buf();
    #[cfg(unix)]
    {
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)) {
            return Ok(StepOutcome::Failed {
                error: format!("Failed to set script permissions: {}", e),
            });
        }
    }

    tracing::info!(
        step_id = %step.id,
        script_path = %path.display(),
        "Executing script"
    );

    let output = tokio::process::Command::new("sh").arg(&path).output().await;

    // Keep tempfile alive until after execution
    drop(tmp);

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
            let exit_code = out.status.code().unwrap_or(-1);

            tracing::debug!(
                step_id = %step.id,
                exit_code = exit_code,
                "Script completed"
            );

            if out.status.success() {
                let mut outputs = build_outputs(&stdout, &stderr, exit_code);
                apply_extraction(step, &stdout, &mut outputs);
                Ok(StepOutcome::Completed { outputs })
            } else {
                Ok(StepOutcome::Failed {
                    error: format!(
                        "Script exited with code {}: {}",
                        exit_code,
                        if stderr.is_empty() { &stdout } else { &stderr }
                    ),
                })
            }
        }
        Err(e) => Ok(StepOutcome::Failed {
            error: format!("Failed to execute script: {}", e),
        }),
    }
}

/// HTTP sub-type — build and execute a reqwest request.
/// Supports: method, url, headers, body fields in action.
/// Full test coverage added in Task 9.
async fn execute_http(
    action: &Value,
    step: &ResolvedStep,
    state: &ExecutionState,
    ctx: &RunContext,
) -> Result<StepOutcome> {
    let method = action
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_uppercase();

    let url = match action.get("url").and_then(Value::as_str) {
        Some(u) => interpolate_str(u, &state.variables).into_owned(),
        None => {
            return Ok(StepOutcome::Failed {
                error: "HTTP action missing 'url' field".to_string(),
            });
        }
    };

    tracing::info!(
        step_id = %step.id,
        method = %method,
        url = %url,
        "Executing HTTP request"
    );

    let mut req = match method.as_str() {
        "GET" => ctx.http.get(&url),
        "POST" => ctx.http.post(&url),
        "PUT" => ctx.http.put(&url),
        "PATCH" => ctx.http.patch(&url),
        "DELETE" => ctx.http.delete(&url),
        other => {
            return Ok(StepOutcome::Failed {
                error: format!("Unsupported HTTP method: {}", other),
            });
        }
    };

    // Apply optional headers
    if let Some(Value::Object(headers)) = action.get("headers") {
        for (key, val) in headers {
            if let Some(v) = val.as_str() {
                let v_interpolated = interpolate_str(v, &state.variables).into_owned();
                req = req.header(key.as_str(), v_interpolated);
            }
        }
    }

    // Apply optional body
    if let Some(body) = action.get("body") {
        let interpolated_body = interpolate_json(body, &state.variables);
        req = req.json(&interpolated_body);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let response_body = resp.text().await.unwrap_or_default();

            let mut outputs = HashMap::new();
            outputs.insert("status".to_string(), Value::Number(status.into()));
            outputs.insert("body".to_string(), Value::String(response_body));

            if status < 400 {
                Ok(StepOutcome::Completed { outputs })
            } else {
                Ok(StepOutcome::Failed {
                    error: format!("HTTP request returned status {}", status),
                })
            }
        }
        Err(e) => Ok(StepOutcome::Failed {
            error: format!("HTTP request failed: {}", e),
        }),
    }
}

/// MCP-tool sub-type — call MCP server tool with interpolated arguments.
/// Full test coverage added in Task 9.
async fn execute_mcp_tool(
    action: &Value,
    step: &ResolvedStep,
    state: &ExecutionState,
    ctx: &RunContext,
) -> Result<StepOutcome> {
    // Accept both field name conventions:
    // - Rust runner: { "tool": "...", "args": {...} }
    // - Bash runner: { "mcpTool": "...", "mcpParams": {...} }
    let tool_name = match action
        .get("tool")
        .or_else(|| action.get("mcpTool"))
        .and_then(Value::as_str)
    {
        Some(t) => t.to_string(),
        None => {
            return Ok(StepOutcome::Failed {
                error: "MCP-tool action missing 'tool'/'mcpTool' field".to_string(),
            });
        }
    };

    let args = match action.get("args").or_else(|| action.get("mcpParams")) {
        Some(a) => interpolate_json(a, &state.variables),
        None => Value::Object(serde_json::Map::new()),
    };

    tracing::info!(
        step_id = %step.id,
        tool = %tool_name,
        "Executing MCP tool"
    );

    // Route to external MCP server if mcpUrl is specified, otherwise use the
    // runner's MCP client which already has auth and session. The external URL
    // is interpolated so callers can embed variable references (e.g. ${serviceUrl}).
    //
    // Security: Do NOT forward the runner's auth token to external servers.
    // External MCP calls use the optional `mcpAuthToken` field from the action
    // config (also interpolated), or no auth at all.
    let call_result = match action.get("mcpUrl").and_then(Value::as_str) {
        Some(url) => {
            let resolved_url = interpolate_str(url, &state.variables);
            let external_token = action
                .get("mcpAuthToken")
                .and_then(Value::as_str)
                .map(|t| interpolate_str(t, &state.variables).into_owned());
            let adhoc = crate::clients::mcp::McpClient::with_auth(
                &resolved_url,
                "",
                "", // no org/workspace context for external servers
                external_token,
            );
            adhoc.call_tool(&tool_name, args).await
        }
        None => ctx.mcp.call_tool(&tool_name, args).await,
    };

    match call_result {
        Ok(response) => {
            let mut outputs = HashMap::new();
            // Apply responseMapping if present: extract named fields from the response
            // using simple dot-notation paths (with optional "$."-prefix for JSONPath compat).
            if let Some(mapping) = action.get("responseMapping").and_then(Value::as_object) {
                for (var_name, path_val) in mapping {
                    if let Some(path) = path_val.as_str() {
                        // Support both "$.field.nested" (JSONPath-like) and "field.nested" (plain)
                        let clean_path = path.strip_prefix("$.").unwrap_or(path);
                        if let Some(value) =
                            crate::output::resolve_json_path(&response, clean_path)
                        {
                            outputs.insert(var_name.clone(), value.clone());
                        }
                        // Missing paths are silently skipped — not every response has every field
                    }
                }
            }
            // Always include the raw response for backward compatibility
            outputs.insert("result".to_string(), response);
            Ok(StepOutcome::Completed { outputs })
        }
        Err(e) => Ok(StepOutcome::Failed {
            error: format!("MCP tool '{}' failed: {}", tool_name, e),
        }),
    }
}

/// Public entry point for other handlers to delegate to the MCP tool execution path.
/// Builds a resolved step internally since the caller only has an action object.
pub async fn execute_mcp_tool_from_action(
    action: &Value,
    step: &ResolvedStep,
    state: &ExecutionState,
    ctx: &RunContext,
) -> Result<StepOutcome> {
    execute_mcp_tool(action, step, state, ctx).await
}

/// Build the standard output map from stdout/stderr/exit_code.
fn build_outputs(stdout: &str, stderr: &str, exit_code: i32) -> HashMap<String, Value> {
    let mut outputs = HashMap::new();
    outputs.insert("stdout".to_string(), Value::String(stdout.to_string()));
    outputs.insert("stderr".to_string(), Value::String(stderr.to_string()));
    outputs.insert("exitCode".to_string(), Value::Number(exit_code.into()));
    outputs
}

/// Apply output extraction from the step config to stdout, inserting results
/// into the outputs map. Failures are logged but do not fail the step —
/// extraction is best-effort when data is missing.
fn apply_extraction(step: &ResolvedStep, stdout: &str, outputs: &mut HashMap<String, Value>) {
    if let Some(extraction_value) = &step.output_extraction {
        // Deserialize the extraction config from the stored JSON value
        if let Ok(extraction) = serde_json::from_value::<crate::models::agent::OutputExtraction>(
            extraction_value.clone(),
        ) {
            let raw_value = serde_json::json!({"raw": stdout});
            match crate::output::extract_output(&raw_value, &extraction) {
                Ok(extracted) => {
                    // When merge_result is true, merge object fields into outputs.
                    // Otherwise store under the "extracted" key.
                    if extraction.merge_result {
                        if let Value::Object(map) = extracted {
                            for (k, v) in map {
                                outputs.insert(k, v);
                            }
                        } else {
                            outputs.insert("extracted".to_string(), extracted);
                        }
                    } else {
                        outputs.insert("extracted".to_string(), extracted);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        step_id = %step.id,
                        error = %e,
                        "Output extraction failed (non-fatal)"
                    );
                }
            }
        }
    }
}
