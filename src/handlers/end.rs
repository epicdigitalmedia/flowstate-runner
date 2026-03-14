use crate::handlers::action::execute_mcp_tool_from_action;
use crate::handlers::{Handler, RunContext};
use crate::models::execution::{ExecutionState, ResolvedStep, StepOutcome};
use anyhow::Result;
use async_trait::async_trait;
use nanoid::nanoid;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Handler for the `end` step type. Aggregates execution metrics, posts a
/// completion summary discussion on the source entity, and updates the entity
/// status to "Complete". All REST side-effects are fire-and-forget — failures
/// are logged as warnings and do not fail the step.
pub struct EndHandler;

#[async_trait]
impl Handler for EndHandler {
    async fn execute(
        &self,
        step: &ResolvedStep,
        state: &ExecutionState,
        ctx: &RunContext,
    ) -> Result<StepOutcome> {
        let total_steps = state.step_history.len();
        let completed_steps = state
            .step_history
            .iter()
            .filter(|h| h.status == "completed")
            .count();
        let failed_steps = state
            .step_history
            .iter()
            .filter(|h| h.status == "failed")
            .count();

        // Compute elapsed time from started_at to now. started_at is recorded
        // when the execution begins; we measure against wall clock here since
        // completed_at has not been written yet at this point.
        let elapsed_ms = state.started_at.as_deref().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s).ok().map(|started| {
                let now = chrono::Utc::now();
                (now - started.with_timezone(&chrono::Utc)).num_milliseconds()
            })
        });

        let mut outputs: HashMap<String, Value> = HashMap::new();
        outputs.insert("_total_steps".to_string(), json!(total_steps));
        outputs.insert("_completed_steps".to_string(), json!(completed_steps));
        outputs.insert("_failed_steps".to_string(), json!(failed_steps));
        if let Some(ms) = elapsed_ms {
            outputs.insert("_elapsed_ms".to_string(), json!(ms));
        }

        // Post completion summary via MCP tool path — fire-and-forget.
        let elapsed_str = elapsed_ms
            .map(|ms| format!(", elapsed: {}ms", ms))
            .unwrap_or_default();
        let summary = format!(
            "Process '{}' completed.\n\n**Steps:** {} total, {} completed, {} failed{}",
            state.process_name, total_steps, completed_steps, failed_steps, elapsed_str
        );

        if let Some(context) = &state.context {
            if let Some(user_id) = context.user_id.as_deref() {
                let now_str = chrono::Utc::now().to_rfc3339();
                let disc_id = format!("disc_{}", nanoid!(10));
                let disc_action = json!({
                    "type": "mcp-tool",
                    "tool": "collection-create",
                    "args": {
                        "collection": "discussions",
                        "data": {
                            "id": disc_id,
                            "orgId": state.org_id,
                            "workspaceId": state.workspace_id,
                            "entityType": context.entity_type,
                            "entityId": context.entity_id,
                            "userId": user_id,
                            "userName": "FlowState Runner",
                            "content": summary,
                            "threadDepth": 0,
                            "isEdited": false,
                            "isDeleted": false,
                            "attachments": [],
                            "archived": false,
                            "createdAt": now_str,
                            "updatedAt": now_str
                        }
                    }
                });
                match execute_mcp_tool_from_action(&disc_action, step, state, ctx).await {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(
                            execution_id = %state.id,
                            error = %e,
                            "Failed to post completion summary discussion (non-fatal)"
                        );
                    }
                }
            }
        }

        // Update source entity status to Complete — fire-and-forget.
        if let Some(context) = &state.context {
            let collection = crate::scanner::pluralize_entity_type(&context.entity_type);
            update_entity_status(ctx, &collection, &context.entity_id).await;
        }

        tracing::info!(
            step_id = %step.id,
            total_steps = total_steps,
            completed_steps = completed_steps,
            failed_steps = failed_steps,
            "End handler completed"
        );

        Ok(StepOutcome::Completed { outputs })
    }
}

/// Fetch the entity, patch status to "Complete", and write it back via the
/// RunContext proxy (routes through MCP for VCA collections, REST for native).
/// Non-fatal: failures are logged as warnings.
async fn update_entity_status(ctx: &RunContext, collection: &str, entity_id: &str) {
    let entity: Result<Value, _> = ctx.get(collection, entity_id).await;
    match entity {
        Ok(mut doc) => {
            doc["status"] = json!("Complete");
            doc["completed"] = json!(true);
            doc["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
            if let Err(e) = ctx.set(collection, &doc).await {
                tracing::warn!(
                    collection = collection,
                    entity_id = entity_id,
                    error = %e,
                    "Failed to update entity status to Complete (non-fatal)"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                collection = collection,
                entity_id = entity_id,
                error = %e,
                "Failed to fetch entity for status update (non-fatal)"
            );
        }
    }
}
