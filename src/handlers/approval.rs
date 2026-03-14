use crate::handlers::{Handler, RunContext};
use crate::models::execution::{ExecutionState, PauseReason, ResolvedStep, StepOutcome};
use anyhow::Result;
use async_trait::async_trait;
use nanoid::nanoid;
use serde_json::{json, Value};
use std::collections::HashMap;

pub struct ApprovalHandler;

// ---------------------------------------------------------------------------
// Pure helper functions — testable without async or HTTP
// ---------------------------------------------------------------------------

/// Build the approval record to persist to the `approvals` collection.
///
/// Always uses strategy `"human"` unless overridden via `action.strategy`.
/// Category defaults to `"spec"` — a stable, schema-valid enum value.
///
/// The ID is generated with a `"appr_"` prefix to match the global entity
/// ID convention (`{4-char-prefix}_{10-char-nanoid}`).
pub fn build_approval_record(
    action: &Value,
    execution_id: &str,
    step_id: &str,
    org_id: &str,
    workspace_id: &str,
) -> Value {
    let approval_id = format!("appr_{}", nanoid!(10));
    let strategy = action
        .get("strategy")
        .and_then(Value::as_str)
        .unwrap_or("human");
    let category = action
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("spec");
    let title = action
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Approval Required");
    let description = action
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");

    let now = chrono::Utc::now().to_rfc3339();

    json!({
        "id": approval_id,
        "orgId": org_id,
        "workspaceId": workspace_id,
        "executionId": execution_id,
        "stepId": step_id,
        "status": "pending",
        "strategy": strategy,
        "category": category,
        "title": title,
        "description": description,
        "feedback": null,
        "annotations": null,
        "archived": false,
        "createdAt": now,
        "updatedAt": now
    })
}

/// Scan the step's conditions for an entry matching `outcome` and return its
/// `targetStepId` if found.
///
/// Conditions are JSON objects of the form:
/// `{"value": "rejected", "targetStepId": "step_revise"}`.
/// Returns `None` when the slice is empty or no condition matches.
pub fn find_condition_target(conditions: &[Value], outcome: &str) -> Option<String> {
    conditions.iter().find_map(|c| {
        let matches = c.get("value").and_then(Value::as_str)? == outcome;
        if matches {
            c.get("targetStepId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Handler implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Handler for ApprovalHandler {
    /// Execute an approval step.
    ///
    /// Flow:
    /// 1. Require action; fail if absent.
    /// 2. Read strategy (default: "human").
    /// 3. Build approval record and write to `approvals` collection via REST.
    /// 4. If strategy == "agent_approve": create a bridge task for the
    ///    designated approver agent.
    /// 5. Return `Paused(Approval { approval_id })`.
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
                    error: "approval step missing action configuration".to_string(),
                });
            }
        };

        let record = build_approval_record(
            action,
            &state.id,
            &step.id,
            &state.org_id,
            &state.workspace_id,
        );

        let approval_id = record["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("build_approval_record produced a record without string id"))?
            .to_owned();

        ctx.set("approvals", &record).await?;

        tracing::info!(
            step_id = %step.id,
            approval_id = %approval_id,
            "Approval record created — pausing execution"
        );

        let strategy = action
            .get("strategy")
            .and_then(Value::as_str)
            .unwrap_or("human");

        if strategy == "agent_approve" {
            self.create_bridge_task(action, &approval_id, step, state, ctx)
                .await?;
        }

        Ok(StepOutcome::Paused(PauseReason::Approval { approval_id }))
    }

    /// Check whether a paused approval step can resume.
    ///
    /// Resume logic:
    /// - "approved"        → Completed; forward approvalStatus, approvalFeedback,
    ///                       and approvalAnnotations as outputs.
    /// - "rejected"        → Completed with approvalStatus + optional
    ///                       `_next_step_override` when a matching condition exists.
    /// - "needs-revision"  → Same as rejected but status is "needs-revision".
    /// - "pending"         → Still waiting; return `None`.
    /// - other             → Unexpected; log a warning and return `None`.
    async fn check_resume(
        &self,
        step: &ResolvedStep,
        _state: &ExecutionState,
        reason: &PauseReason,
        ctx: &RunContext,
    ) -> Result<Option<StepOutcome>> {
        let approval_id = match reason {
            PauseReason::Approval { approval_id } => approval_id.as_str(),
            _ => return Ok(None),
        };

        let record: Value = ctx.get("approvals", approval_id).await?;

        let status = match record.get("status").and_then(Value::as_str) {
            Some(s) => s.to_owned(),
            None => {
                tracing::warn!(approval_id, "Approval record missing status field");
                return Ok(None);
            }
        };

        match status.as_str() {
            "approved" => {
                let mut outputs = HashMap::new();
                outputs.insert("approvalStatus".to_string(), json!("approved"));
                if let Some(fb) = record.get("feedback") {
                    outputs.insert("approvalFeedback".to_string(), fb.clone());
                }
                if let Some(ann) = record.get("annotations") {
                    outputs.insert("approvalAnnotations".to_string(), ann.clone());
                }
                Ok(Some(StepOutcome::Completed { outputs }))
            }

            "rejected" | "needs-revision" => {
                let mut outputs = HashMap::new();
                outputs.insert("approvalStatus".to_string(), json!(status));
                if let Some(fb) = record.get("feedback") {
                    outputs.insert("approvalFeedback".to_string(), fb.clone());
                }
                if let Some(ann) = record.get("annotations") {
                    outputs.insert("approvalAnnotations".to_string(), ann.clone());
                }

                // Route to a different step when a matching condition is found.
                if let Some(target) = find_condition_target(&step.conditions, &status) {
                    outputs.insert("_next_step_override".to_string(), Value::String(target));
                }

                Ok(Some(StepOutcome::Completed { outputs }))
            }

            "pending" => Ok(None),

            other => {
                tracing::warn!(
                    approval_id,
                    status = other,
                    "Unrecognised approval status — still waiting"
                );
                Ok(None)
            }
        }
    }
}

impl ApprovalHandler {
    /// Create a bridge task that notifies an approver agent about the pending
    /// approval. Used when `strategy == "agent_approve"`.
    ///
    /// The task is tagged with `["approval", "pending"]` so the approver
    /// agent's trigger rule can detect it. The `approvalId` is embedded in
    /// `metadata` for correlation.
    async fn create_bridge_task(
        &self,
        action: &Value,
        approval_id: &str,
        step: &ResolvedStep,
        state: &ExecutionState,
        ctx: &RunContext,
    ) -> Result<()> {
        let task_id = format!("task_{}", nanoid!(10));
        let title = action
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Review and approve");
        let now = chrono::Utc::now().to_rfc3339();

        let task = json!({
            "id": task_id,
            "orgId": state.org_id,
            "workspaceId": state.workspace_id,
            "title": format!("[Approval] {}", title),
            "status": "pending",
            "tags": ["approval", "pending"],
            "metadata": {
                "approvalId": approval_id,
                "executionId": state.id,
                "stepId": step.id
            },
            "archived": false,
            "createdAt": now,
            "updatedAt": now
        });

        ctx.set("tasks", &task).await?;

        tracing::info!(
            step_id = %step.id,
            approval_id = %approval_id,
            task_id = %task_id,
            "Bridge task created for agent_approve strategy"
        );

        Ok(())
    }
}
