use crate::handlers::action::execute_mcp_tool_from_action;
use crate::handlers::{Handler, RunContext};
use crate::models::execution::{ExecutionState, ResolvedStep, StepOutcome};
use crate::template::interpolate_str;
use anyhow::Result;
use async_trait::async_trait;
use nanoid::nanoid;
use serde_json::{json, Value};
use std::collections::HashMap;

pub struct NotificationHandler;

#[async_trait]
impl Handler for NotificationHandler {
    async fn execute(
        &self,
        step: &ResolvedStep,
        state: &ExecutionState,
        ctx: &RunContext,
    ) -> Result<StepOutcome> {
        let content = match &step.inputs {
            Some(Value::Object(inputs)) => match inputs.get("content").and_then(Value::as_str) {
                Some(template) => interpolate_str(template, &state.variables).into_owned(),
                None => format!("Notification from step '{}'", step.name),
            },
            _ => format!("Notification from step '{}'", step.name),
        };

        // Derive entity reference from execution context
        let (entity_type, entity_id, user_id) = match &state.context {
            Some(ctx) => (
                ctx.entity_type.as_str(),
                ctx.entity_id.as_str(),
                ctx.user_id.as_deref().unwrap_or("system"),
            ),
            None => {
                tracing::debug!(
                    execution_id = %state.id,
                    "No execution context — skipping notification"
                );
                return Ok(StepOutcome::Completed {
                    outputs: HashMap::new(),
                });
            }
        };

        let now = chrono::Utc::now().to_rfc3339();
        let discussion_id = format!("disc_{}", nanoid!(10));

        let action = json!({
            "type": "mcp-tool",
            "tool": "collection-create",
            "args": {
                "collection": "discussions",
                "data": {
                    "id": discussion_id,
                    "orgId": state.org_id,
                    "workspaceId": state.workspace_id,
                    "entityType": entity_type,
                    "entityId": entity_id,
                    "userId": user_id,
                    "userName": "FlowState Runner",
                    "content": content,
                    "threadDepth": 0,
                    "isEdited": false,
                    "isDeleted": false,
                    "attachments": [],
                    "archived": false,
                    "createdAt": now,
                    "updatedAt": now
                }
            }
        });

        // Delegate to the generic MCP tool execution path.
        // Failures are non-fatal — notification is best-effort.
        match execute_mcp_tool_from_action(&action, step, state, ctx).await {
            Ok(StepOutcome::Completed { .. }) => Ok(StepOutcome::Completed {
                outputs: HashMap::new(),
            }),
            Ok(StepOutcome::Failed { error }) => {
                tracing::warn!(
                    step_id = %step.id,
                    error = %error,
                    "Notification discussion creation failed (non-fatal)"
                );
                Ok(StepOutcome::Completed {
                    outputs: HashMap::new(),
                })
            }
            Ok(other) => Ok(other),
            Err(e) => {
                tracing::warn!(
                    step_id = %step.id,
                    error = %e,
                    "Notification discussion creation failed (non-fatal)"
                );
                Ok(StepOutcome::Completed {
                    outputs: HashMap::new(),
                })
            }
        }
    }
}
