use crate::executor::{
    self, calculate_progress, persist_state, record_step_history, resolve_next_step,
    STATUS_COMPLETED, STATUS_FAILED, STATUS_RUNNING,
};
use crate::handlers::{dispatch_handler, RunContext};
use crate::models::execution::{ExecutionState, ProcessExecutionRecord, StepOutcome};
use crate::models::process::StepTemplate;
use crate::output::map_outputs;
use crate::state::compute_plan_dir;
use crate::template::resolve_template;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Report returned by `resume()` summarising what happened.
#[derive(Debug, Clone, Default)]
pub struct ResumeReport {
    /// IDs of executions that successfully resumed.
    pub resumed: Vec<String>,
    /// Count of paused executions still waiting for their condition.
    pub still_waiting: u32,
    /// Errors encountered (non-fatal — resume continues past failures).
    pub errors: Vec<String>,
}

/// Resume paused executions.
///
/// Flow:
/// 1. Query all paused executions.
/// 2. For each:
///    a. Convert to `ExecutionState`, extract `PauseReason`.
///    b. Load the process and its steps.
///    c. Resolve the paused step's template.
///    d. Dispatch to handler's `check_resume()`.
///    e. If `Some(outcome)`: apply outcome, then continue executor loop.
///    f. If `None`: skip (still waiting).
///
/// `templates` is the pre-loaded step template map, used for template
/// resolution when rebuilding steps from the database.
pub async fn resume(
    ctx: &RunContext,
    templates: &HashMap<String, StepTemplate>,
) -> Result<ResumeReport> {
    let mut report = ResumeReport::default();

    // 1. Query paused executions (virtual collection — goes through MCP)
    let records: Vec<ProcessExecutionRecord> = ctx
        .query(
            "processexecutions",
            json!({
                "status": "paused",
                "orgId": ctx.config.org_id,
                "workspaceId": ctx.config.workspace_id,
            }),
        )
        .await
        .context("Failed to query paused executions")?;

    tracing::info!(count = records.len(), "Found paused executions to check");

    for record in records {
        let exec_id = record.id.clone();
        let process_id = record.process_id.clone();

        match resume_one(record, ctx, templates).await {
            Ok(ResumeOneResult::Resumed) => {
                tracing::info!(execution_id = %exec_id, "Execution resumed");
                report.resumed.push(exec_id);
            }
            Ok(ResumeOneResult::StillWaiting) => {
                tracing::debug!(execution_id = %exec_id, "Still waiting");
                report.still_waiting += 1;
            }
            Err(err) => {
                let msg = format!(
                    "Failed to check resume for execution '{}' (process '{}'): {}",
                    exec_id, process_id, err
                );
                tracing::warn!("{}", msg);
                report.errors.push(msg);
            }
        }
    }

    tracing::info!(
        resumed = report.resumed.len(),
        still_waiting = report.still_waiting,
        errors = report.errors.len(),
        "Resume complete"
    );

    Ok(report)
}

enum ResumeOneResult {
    Resumed,
    StillWaiting,
}

/// Attempt to resume a single paused execution.
async fn resume_one(
    record: ProcessExecutionRecord,
    ctx: &RunContext,
    templates: &HashMap<String, StepTemplate>,
) -> Result<ResumeOneResult> {
    // Load process to get name (virtual collection — goes through MCP, cached)
    let process = ctx
        .get_process_cached(&record.process_id)
        .await
        .with_context(|| format!("Failed to load process '{}'", record.process_id))?;

    // Convert to in-memory state
    let mut state = ExecutionState::from_record(record, process.name.clone());
    state.plan_dir = compute_plan_dir(
        ctx.config.plan_base_dir.to_str().unwrap_or("."),
        state.external_id.as_deref(),
    );

    // Extract pause reason
    let reason = match &state.pause_reason {
        Some(r) => r.clone(),
        None => {
            tracing::warn!(
                execution_id = %state.id,
                "Paused execution has no pause_reason in metadata — skipping"
            );
            return Ok(ResumeOneResult::StillWaiting);
        }
    };

    // Get current step ID
    let step_id = match &state.current_step_id {
        Some(id) => id.clone(),
        None => {
            anyhow::bail!("Paused execution has no current_step_id");
        }
    };

    // Load process steps (virtual collection — goes through MCP, cached)
    let raw_steps = ctx
        .get_steps_cached(&process.id)
        .await
        .context("Failed to load process steps")?;

    // Build step map with template resolution
    let steps: HashMap<String, _> = raw_steps
        .iter()
        .map(|s| {
            let tmpl = s.template_id.as_ref().and_then(|tid| templates.get(tid));
            let resolved = resolve_template(s, tmpl);
            (s.id.clone(), resolved)
        })
        .collect();

    // Find the paused step
    let step = steps
        .get(&step_id)
        .with_context(|| format!("Paused step '{}' not found in process steps", step_id))?;

    // Dispatch handler and check resume
    let handler = dispatch_handler(&step.step_type)
        .with_context(|| format!("Failed to dispatch handler for '{}'", step.step_type))?;

    let outcome = handler
        .check_resume(step, &state, &reason, ctx)
        .await
        .with_context(|| format!("check_resume failed for step '{}'", step_id))?;

    match outcome {
        None => Ok(ResumeOneResult::StillWaiting),
        Some(step_outcome) => {
            // Apply outcome and continue execution
            apply_outcome_and_continue(&mut state, step, step_outcome, &steps, ctx).await?;
            Ok(ResumeOneResult::Resumed)
        }
    }
}

/// Apply a resume outcome to the state and continue the executor loop.
///
/// This mirrors the executor's outcome processing: map outputs, record history,
/// update progress, reset retry count, resolve next step, then call
/// `executor::execute()` if there are more steps.
async fn apply_outcome_and_continue(
    state: &mut ExecutionState,
    step: &crate::models::execution::ResolvedStep,
    outcome: StepOutcome,
    steps: &HashMap<String, crate::models::execution::ResolvedStep>,
    ctx: &RunContext,
) -> Result<()> {
    let started_at = chrono::Utc::now().to_rfc3339();
    let total_steps = steps.len();

    match outcome {
        StepOutcome::Completed { outputs } => {
            // Clear pause reason
            state.pause_reason = None;
            state.status = STATUS_RUNNING.to_string();

            // Map outputs
            map_outputs(&step.outputs, &outputs, &mut state.variables)?;

            // Direct merge fallback
            if step.outputs.is_empty() {
                for (key, value) in &outputs {
                    if !key.starts_with('_') {
                        state.variables.insert(key.clone(), value.clone());
                    }
                }
            }

            // Record history
            record_step_history(state, step, STATUS_COMPLETED, &started_at);

            // Update progress — count only completed steps
            let completed_count = state
                .step_history
                .iter()
                .filter(|h| h.status == STATUS_COMPLETED)
                .count();
            state.progress = Some(calculate_progress(completed_count, total_steps));

            // Reset retry count on success (mirrors executor behavior)
            state.retry_count = 0;

            // Resolve next step
            match resolve_next_step(step, &outputs) {
                Some(next_id) => {
                    state.current_step_id = Some(next_id);
                    // Persist intermediate state, then continue executor loop
                    persist_state(state, ctx).await?;
                    executor::execute(state, steps, &dispatch_handler, ctx, true).await?;
                }
                None => {
                    // No next step — completed
                    state.status = STATUS_COMPLETED.to_string();
                    state.progress = Some(100);
                    state.completed_at = Some(chrono::Utc::now().to_rfc3339());
                    persist_state(state, ctx).await?;
                }
            }
        }
        StepOutcome::Paused(reason) => {
            // Still paused with a new reason (unlikely but possible)
            state.pause_reason = Some(reason);
            persist_state(state, ctx).await?;
        }
        StepOutcome::Failed { error } => {
            // Check retry count before failing permanently
            if state.retry_count < state.max_retries {
                state.retry_count += 1;
                state.pause_reason = None;
                state.status = STATUS_RUNNING.to_string();
                persist_state(state, ctx).await?;
                executor::execute(state, steps, &dispatch_handler, ctx, true).await?;
            } else {
                state.pause_reason = None;
                state.status = STATUS_FAILED.to_string();
                state.error = Some(Value::String(error));
                record_step_history(state, step, STATUS_FAILED, &started_at);
                persist_state(state, ctx).await?;
            }
        }
    }

    Ok(())
}
