use crate::handlers::{Handler, RunContext};
use crate::models::execution::{
    ExecutionContext, ExecutionState, PauseReason, ResolvedStep, StepOutcome,
};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use nanoid::nanoid;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Pure helper functions — testable without async or HTTP
// ---------------------------------------------------------------------------

/// Resolve an input mapping from parent variables into a new variable map for
/// the child execution.
///
/// Mapping values of the form `"${varName}"` are template references: the
/// `${…}` wrapper is stripped and the value is looked up in `parent_vars`.
/// If the referenced variable is absent the entry is omitted silently.
///
/// Non-template string values and non-string values are passed through as-is.
///
/// Returns an empty map when `mapping` is not a JSON object.
pub fn resolve_input_mapping(
    mapping: &Value,
    parent_vars: &Map<String, Value>,
) -> Map<String, Value> {
    let obj = match mapping.as_object() {
        Some(o) => o,
        None => return Map::new(),
    };

    let mut result = Map::new();

    for (child_key, val) in obj {
        if let Some(tmpl) = val.as_str() {
            if tmpl.starts_with("${") && tmpl.ends_with('}') {
                // Strip the "${" prefix and "}" suffix to get the variable name.
                let var_name = &tmpl[2..tmpl.len() - 1];
                if let Some(parent_val) = parent_vars.get(var_name) {
                    result.insert(child_key.clone(), parent_val.clone());
                }
                // Missing variable → silently skip
            } else {
                // Literal string value — pass through
                result.insert(child_key.clone(), val.clone());
            }
        } else {
            // Non-string value (number, object, array, bool) — pass through
            result.insert(child_key.clone(), val.clone());
        }
    }

    result
}

/// Apply an output mapping from a completed child execution back into the
/// parent's variable map.
///
/// Mapping format: `{ "parentVarName": "childVarName" }`.
/// Missing child variables are silently skipped.
pub fn apply_output_mapping(
    mapping: &Value,
    child_vars: &Map<String, Value>,
    parent_vars: &mut Map<String, Value>,
) {
    let obj = match mapping.as_object() {
        Some(o) => o,
        None => return,
    };

    for (parent_key, child_key_val) in obj {
        if let Some(child_key) = child_key_val.as_str() {
            if let Some(val) = child_vars.get(child_key) {
                parent_vars.insert(parent_key.clone(), val.clone());
            }
        }
    }
}

/// Validate the subprocess depth limit and return the child's depth value.
///
/// Returns `Err` when `context.depth >= max_subprocess_depth`. The `context`
/// argument may be `None` (meaning top-level, depth 0).
pub fn check_depth_limit(
    context: Option<&ExecutionContext>,
    max_subprocess_depth: u32,
) -> Result<u32> {
    let current_depth = context.map(|c| c.depth).unwrap_or(0);
    if current_depth >= max_subprocess_depth {
        bail!(
            "Subprocess depth limit reached: current depth {} >= max {}",
            current_depth,
            max_subprocess_depth
        );
    }
    Ok(current_depth + 1)
}

// ---------------------------------------------------------------------------
// Handler implementation
// ---------------------------------------------------------------------------

pub struct SubprocessHandler;

#[async_trait]
impl Handler for SubprocessHandler {
    /// Execute a subprocess step.
    ///
    /// Flow:
    /// 1. Fail if `step.action` is absent.
    /// 2. Idempotency: if `childExecutionId` is already in parent variables,
    ///    fetch the child and return its current state instead of re-creating.
    ///    - completed → apply output mapping, return Completed
    ///    - running/paused/pending → return Paused(Subprocess{…})
    ///    - failed → warn and fall through to create a fresh child
    /// 3. Enforce depth limit.
    /// 4. Resolve input mapping from parent variables.
    /// 5. Create child execution with status "running" (to prevent worker
    ///    race) when `waitForCompletion` is true and we are not in worker mode,
    ///    otherwise "pending".
    /// 6. Return Paused(Subprocess{child_execution_id}).
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
                    error: "subprocess step missing action configuration".to_string(),
                });
            }
        };

        // Idempotency: check if we already created a child execution
        if let Some(Value::String(existing_id)) = state.variables.get("childExecutionId") {
            if !existing_id.is_empty() {
                let child_id = existing_id.clone();
                match self.fetch_child_status(&child_id, ctx).await {
                    Ok(child_doc) => {
                        let child_status = child_doc
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();

                        match child_status.as_str() {
                            "completed" => {
                                let mut outputs = HashMap::new();
                                outputs.insert(
                                    "childExecutionId".to_string(),
                                    Value::String(child_id.clone()),
                                );
                                outputs.insert("childStatus".to_string(), json!("completed"));
                                let child_vars = child_doc
                                    .get("variables")
                                    .and_then(Value::as_object)
                                    .cloned()
                                    .unwrap_or_default();
                                let output_mapping =
                                    action.get("outputMapping").cloned().unwrap_or(json!({}));
                                let mut parent_vars_out = Map::new();
                                apply_output_mapping(
                                    &output_mapping,
                                    &child_vars,
                                    &mut parent_vars_out,
                                );
                                for (k, v) in parent_vars_out {
                                    outputs.insert(k, v);
                                }
                                return Ok(StepOutcome::Completed { outputs });
                            }
                            "running" | "paused" | "pending" => {
                                return Ok(StepOutcome::Paused(PauseReason::Subprocess {
                                    child_execution_id: child_id,
                                }));
                            }
                            "failed" => {
                                tracing::warn!(
                                    child_execution_id = %child_id,
                                    "Previous child execution failed — creating new child"
                                );
                                // Fall through to create a new child execution
                            }
                            other => {
                                tracing::warn!(
                                    child_execution_id = %child_id,
                                    status = other,
                                    "Unexpected child execution status — treating as still running"
                                );
                                return Ok(StepOutcome::Paused(PauseReason::Subprocess {
                                    child_execution_id: child_id,
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            child_execution_id = %child_id,
                            error = %e,
                            "Failed to fetch existing child execution — creating new child"
                        );
                        // Fall through
                    }
                }
            }
        }

        // Depth enforcement
        let child_depth =
            check_depth_limit(state.context.as_ref(), ctx.config.max_subprocess_depth)?;

        // Required: processId from action
        let process_id = match action.get("processId").and_then(Value::as_str) {
            Some(id) => id.to_owned(),
            None => {
                return Ok(StepOutcome::Failed {
                    error: "subprocess step action missing required 'processId'".to_string(),
                });
            }
        };

        let wait_for_completion = action
            .get("waitForCompletion")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Resolve input mapping from parent variables
        let input_mapping = action.get("inputMapping").cloned().unwrap_or(json!({}));
        let child_variables = resolve_input_mapping(&input_mapping, &state.variables);

        // Worker race prevention: when waitForCompletion is true and we are not
        // in worker mode, create child as "running" so the worker daemon won't
        // also claim it. In worker mode we can't run it ourselves so keep it
        // "pending" and let a worker pick it up.
        let child_status = if wait_for_completion && !ctx.config.worker_mode {
            "running"
        } else {
            "pending"
        };

        let child_id = format!("exec_{}", nanoid!(10));
        let now = chrono::Utc::now().to_rfc3339();

        let child_record = json!({
            "id": child_id,
            "processId": process_id,
            "orgId": state.org_id,
            "workspaceId": state.workspace_id,
            "userId": state.user_id,
            "status": child_status,
            "parentExecutionId": state.id,
            "depth": child_depth,
            "currentStepId": null,
            "variables": child_variables,
            "stepHistory": [],
            "retryCount": 0,
            "maxRetries": 3,
            "archived": false,
            "metadata": {
                "parentStepId": step.id,
                "_pause_reason": null
            },
            "createdAt": now,
            "updatedAt": now
        });

        ctx.set("processexecutions", &child_record)
            .await
            .context("Failed to create child execution record")?;

        tracing::info!(
            step_id = %step.id,
            child_execution_id = %child_id,
            child_status = %child_status,
            "Child execution created — pausing parent"
        );

        Ok(StepOutcome::Paused(PauseReason::Subprocess {
            child_execution_id: child_id,
        }))
    }

    /// Poll the child execution and resolve if it has finished.
    ///
    /// - "completed" → apply output mapping and return Completed
    /// - "failed"    → return Failed with the child's error message
    /// - "running" / "paused" / "pending" → return None (still waiting)
    /// - other → warn and return None
    async fn check_resume(
        &self,
        step: &ResolvedStep,
        _state: &ExecutionState,
        reason: &PauseReason,
        ctx: &RunContext,
    ) -> Result<Option<StepOutcome>> {
        let child_id = match reason {
            PauseReason::Subprocess { child_execution_id } => child_execution_id.as_str(),
            _ => return Ok(None),
        };

        let action = step.action.as_ref().cloned().unwrap_or(json!({}));

        let child_doc: Value = ctx
            .get("processexecutions", child_id)
            .await
            .with_context(|| format!("Failed to fetch child execution {child_id}"))?;

        let child_status = child_doc
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();

        match child_status.as_str() {
            "completed" => {
                let child_vars = child_doc
                    .get("variables")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let output_mapping = action.get("outputMapping").cloned().unwrap_or(json!({}));

                let mut outputs = HashMap::new();
                outputs.insert(
                    "childExecutionId".to_string(),
                    Value::String(child_id.to_owned()),
                );
                outputs.insert("childStatus".to_string(), json!("completed"));

                let mut parent_vars_out = Map::new();
                apply_output_mapping(&output_mapping, &child_vars, &mut parent_vars_out);
                for (k, v) in parent_vars_out {
                    outputs.insert(k, v);
                }

                Ok(Some(StepOutcome::Completed { outputs }))
            }

            "failed" => {
                let error = child_doc
                    .get("error")
                    .and_then(|e| e.get("message").or(Some(e)))
                    .and_then(Value::as_str)
                    .unwrap_or("child execution failed")
                    .to_owned();

                Ok(Some(StepOutcome::Failed {
                    error: format!("Child execution {child_id} failed: {error}"),
                }))
            }

            "running" | "paused" | "pending" => Ok(None),

            other => {
                tracing::warn!(
                    child_execution_id = child_id,
                    status = other,
                    "Unrecognised child execution status — still waiting"
                );
                Ok(None)
            }
        }
    }
}

impl SubprocessHandler {
    /// Fetch a child execution by ID and return the raw document.
    async fn fetch_child_status(&self, child_id: &str, ctx: &RunContext) -> Result<Value> {
        ctx.get("processexecutions", child_id).await
    }
}
