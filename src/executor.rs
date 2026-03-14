use crate::handlers::{Handler, RunContext};
use crate::models::execution::{ExecutionState, ResolvedStep, StepHistoryEntry, StepOutcome};
use crate::output::map_outputs;
use crate::template::interpolate_json;
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;

/// Status string constants — used instead of string literals for consistency
/// and to avoid typos. A full migration to the `ExecutionStatus` enum would
/// require changes to DB serialization and is deferred to a later phase.
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_PAUSED: &str = "paused";
pub const STATUS_FAILED: &str = "failed";

/// Calculate progress as percentage (0-99). Capped at 99 until the end handler
/// explicitly marks completion (which sets progress to 100).
pub fn calculate_progress(completed_steps: usize, total_steps: usize) -> u32 {
    if total_steps == 0 {
        return 0;
    }
    let pct = (completed_steps * 100) / total_steps;
    std::cmp::min(pct, 99) as u32
}

/// Determine the next step ID. `_next_step_override` in handler outputs
/// takes precedence over the step's `next_step_id`. Returns None if
/// execution should end.
pub fn resolve_next_step(
    step: &ResolvedStep,
    handler_outputs: &HashMap<String, Value>,
) -> Option<String> {
    // Check for handler override (set by decision handler, etc.)
    if let Some(Value::String(override_id)) = handler_outputs.get("_next_step_override") {
        if !override_id.is_empty() {
            return Some(override_id.clone());
        }
    }
    step.next_step_id.clone()
}

/// Record a step execution in the history.
pub fn record_step_history(
    state: &mut ExecutionState,
    step: &ResolvedStep,
    status: &str,
    started_at: &str,
) {
    let completed_at = if status == "completed" || status == "failed" {
        Some(chrono::Utc::now().to_rfc3339())
    } else {
        None
    };

    state.step_history.push(StepHistoryEntry {
        step_id: step.id.clone(),
        step_name: Some(step.name.clone()),
        step_type: Some(step.step_type.clone()),
        status: status.to_string(),
        started_at: Some(started_at.to_string()),
        completed_at,
        duration_ms: None,
        retry_count: 0,
        executed_by: None,
        notes: None,
        inputs: None,
        outputs: None,
    });
}

/// Core execution loop. Runs through steps sequentially until completion,
/// pause, or failure.
///
/// - `state`: mutable execution state (modified in place)
/// - `steps`: pre-loaded step definitions keyed by step ID
/// - `dispatch`: function that maps step_type -> Handler (parameterized for testing)
/// - `ctx`: shared runtime context
/// - `persist`: whether to persist state to DB after each step (false for unit tests)
pub async fn execute<D>(
    state: &mut ExecutionState,
    steps: &HashMap<String, ResolvedStep>,
    dispatch: &D,
    ctx: &RunContext,
    persist: bool,
) -> Result<()>
where
    D: Fn(&str) -> Result<Box<dyn Handler>>,
{
    let total_steps = steps.len();
    let mut steps_since_persist: u32 = 0;

    // Mark as running and clear any stale pause reason from a previous pause
    state.status = STATUS_RUNNING.to_string();
    state.pause_reason = None;
    if state.started_at.is_none() {
        state.started_at = Some(chrono::Utc::now().to_rfc3339());
    }

    loop {
        // Get current step
        let step_id = state
            .current_step_id
            .as_ref()
            .context("No current step ID — cannot continue execution")?
            .clone();

        let step = steps
            .get(&step_id)
            .with_context(|| format!("Step '{}' not found in loaded steps", step_id))?;

        let started_at = chrono::Utc::now().to_rfc3339();

        // Resolve step inputs into execution variables.
        // Each input entry maps a step-local variable name to an expression
        // (e.g., {"dirPath": "${planDir}"}). Interpolate the expression against
        // current variables, then inject the result so the handler can use it.
        if let Some(Value::Object(inputs)) = &step.inputs {
            for (key, value) in inputs {
                let resolved = interpolate_json(value, &state.variables);
                state.variables.insert(key.clone(), resolved);
            }
        }

        // Dispatch handler
        let handler = dispatch(&step.step_type).with_context(|| {
            format!(
                "Failed to dispatch handler for step type '{}'",
                step.step_type
            )
        })?;

        // Execute
        let outcome = handler
            .execute(step, state, ctx)
            .await
            .with_context(|| format!("Handler failed for step '{}'", step.name))?;

        // Process outcome
        match outcome {
            StepOutcome::Completed { outputs } => {
                // Map outputs to variables via output specs
                map_outputs(&step.outputs, &outputs, &mut state.variables)?;

                // Direct merge fallback: when no output specs are defined,
                // merge handler outputs directly into variables.
                // StartHandler relies on this to pass inputs to subsequent steps.
                if step.outputs.is_empty() {
                    for (key, value) in &outputs {
                        if !key.starts_with('_') {
                            state.variables.insert(key.clone(), value.clone());
                        }
                    }
                }

                // Record history
                record_step_history(state, step, "completed", &started_at);

                // Update progress — count only completed steps to avoid
                // inflating progress with paused/failed entries
                let completed_count = state
                    .step_history
                    .iter()
                    .filter(|h| h.status == STATUS_COMPLETED)
                    .count();
                state.progress = Some(calculate_progress(completed_count, total_steps));

                // Reset retry count on success
                state.retry_count = 0;

                // Determine next step
                match resolve_next_step(step, &outputs) {
                    Some(next_id) => {
                        state.current_step_id = Some(next_id);
                    }
                    None => {
                        // No next step — execution complete
                        state.status = STATUS_COMPLETED.to_string();
                        state.progress = Some(100);
                        state.completed_at = Some(chrono::Utc::now().to_rfc3339());

                        // Persist final state
                        if persist {
                            persist_state(state, ctx).await?;
                        }
                        break;
                    }
                }

                // Persist at configured interval (default: every step)
                steps_since_persist += 1;
                if persist && steps_since_persist >= ctx.config.persist_interval {
                    persist_state(state, ctx).await?;
                    steps_since_persist = 0;
                }
            }
            StepOutcome::Paused(reason) => {
                record_step_history(state, step, STATUS_PAUSED, &started_at);
                state.status = STATUS_PAUSED.to_string();
                state.pause_reason = Some(reason);

                // Persist paused state
                if persist {
                    persist_state(state, ctx).await?;
                }
                break;
            }
            StepOutcome::Failed { error } => {
                if state.retry_count < state.max_retries {
                    // Retry: increment counter and re-run the same step
                    state.retry_count += 1;
                    tracing::warn!(
                        step_id = %step.id,
                        retry = state.retry_count,
                        max = state.max_retries,
                        error = %error,
                        "Step failed, retrying"
                    );
                    // Don't record history for retried steps — continue loop
                    continue;
                } else {
                    // Max retries exceeded — fail the execution
                    record_step_history(state, step, STATUS_FAILED, &started_at);
                    state.status = STATUS_FAILED.to_string();
                    state.error = Some(Value::String(error));

                    if persist {
                        persist_state(state, ctx).await?;
                    }
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Persist execution state to DB via REST.
pub(crate) async fn persist_state(state: &ExecutionState, ctx: &RunContext) -> Result<()> {
    let record = state.to_record();
    let data = serde_json::to_value(&record).context("Failed to serialize execution state")?;
    ctx.update("processexecutions", &state.id, &data)
        .await
        .context("Failed to persist execution state")?;
    Ok(())
}
