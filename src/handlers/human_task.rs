use crate::handlers::{Handler, RunContext};
use crate::models::execution::{ExecutionState, PauseReason, ResolvedStep, StepOutcome};
use crate::template::interpolate_str;
use anyhow::{Context, Result};
use async_trait::async_trait;
use nanoid::nanoid;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub struct HumanTaskHandler;

// ---------------------------------------------------------------------------
// Pure helper functions — testable without async or HTTP
// ---------------------------------------------------------------------------

/// Build the discussion content by interpolating `action.content` with the
/// current execution variables.
///
/// Returns an empty string when no `content` field is present or when the
/// field is an empty string. The caller should treat an empty result as a
/// hard failure — there is nothing useful to post to a human.
pub fn build_discussion_content(action: &Value, variables: &Map<String, Value>) -> String {
    // Accept both "content" (Rust convention) and "message" (bash runner convention)
    let template = match action
        .get("content")
        .or_else(|| action.get("message"))
        .and_then(Value::as_str)
    {
        Some(t) => t,
        None => return String::new(),
    };

    interpolate_str(template, variables).into_owned()
}

/// Extract the entity reference (type and ID) that this discussion should be
/// attached to.
///
/// Defaults to `("task", "")` so callers always have a non-null pair to work
/// with even when the action config is sparse.
fn get_entity_ref<'a>(action: &'a Value, variables: &'a Map<String, Value>) -> (&'a str, &'a str) {
    let entity_type = action
        .get("entityType")
        .and_then(Value::as_str)
        .or_else(|| variables.get("entityType").and_then(Value::as_str))
        .unwrap_or("task");
    let entity_id = action
        .get("entityId")
        .and_then(Value::as_str)
        .or_else(|| variables.get("entityId").and_then(Value::as_str))
        .unwrap_or("");
    (entity_type, entity_id)
}

// ---------------------------------------------------------------------------
// Handler implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Handler for HumanTaskHandler {
    /// Execute a human-task step.
    ///
    /// Flow:
    /// 1. Require action; fail if absent.
    /// 2. Build discussion content; fail if empty.
    /// 3. Post discussion to the configured entity via REST.
    /// 4. Return `Paused(HumanTask { discussion_id, posted_at })`.
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
                    error: "human-task step missing action configuration".to_string(),
                });
            }
        };

        let content = build_discussion_content(action, &state.variables);
        if content.is_empty() {
            return Ok(StepOutcome::Failed {
                error: "human-task action 'content' is empty — nothing to post".to_string(),
            });
        }

        let (entity_type, entity_id) = get_entity_ref(action, &state.variables);
        let discussion_id = format!("disc_{}", nanoid!(10));
        let now = chrono::Utc::now().to_rfc3339();

        let user_id: Value = match &state.user_id {
            Some(uid) => Value::String(uid.clone()),
            None => Value::Null,
        };

        // Use agent identity from variables when available (set by process
        // config or team-member seeding), falling back to a generic label.
        let user_name = state
            .variables
            .get("agentUserName")
            .and_then(Value::as_str)
            .unwrap_or("Flowstate Agent");

        let discussion = json!({
            "id": discussion_id,
            "entityType": entity_type,
            "entityId": entity_id,
            "orgId": state.org_id,
            "workspaceId": state.workspace_id,
            "userName": user_name,
            "userId": user_id,
            "content": content,
            "threadDepth": 0,
            "isEdited": false,
            "isDeleted": false,
            "attachments": [],
            "createdAt": now,
            "updatedAt": now
        });

        ctx.set("discussions", &discussion)
            .await
            .context("Failed to create human-task discussion")?;

        tracing::info!(
            step_id = %step.id,
            discussion_id = %discussion_id,
            "Discussion posted — pausing for human reply"
        );

        Ok(StepOutcome::Paused(PauseReason::HumanTask {
            discussion_id,
            posted_at: now,
        }))
    }

    /// Check whether a paused human-task step can resume.
    ///
    /// Resume logic:
    /// 1. Query discussions with `parentId == discussion_id`.
    /// 2. Client-side filter: keep replies where `createdAt > posted_at`.
    ///    (RxDB REST does not support server-side `$gt` date comparisons.)
    /// 3. No qualifying replies → still waiting; return `Ok(None)`.
    /// 4. Replies found → merge content; return `Completed` with
    ///    `humanReply` and `replyCount` outputs.
    async fn check_resume(
        &self,
        _step: &ResolvedStep,
        _state: &ExecutionState,
        reason: &PauseReason,
        ctx: &RunContext,
    ) -> Result<Option<StepOutcome>> {
        let (discussion_id, posted_at) = match reason {
            PauseReason::HumanTask {
                discussion_id,
                posted_at,
            } => (discussion_id.as_str(), posted_at.as_str()),
            _ => return Ok(None),
        };

        // Query all replies to this thread
        let replies: Vec<Value> = ctx
            .query("discussions", json!({ "parentId": discussion_id }))
            .await?;

        // Client-side date filter — RFC3339 strings compare lexicographically
        let new_replies: Vec<&Value> = replies
            .iter()
            .filter(|r| {
                r.get("createdAt")
                    .and_then(Value::as_str)
                    .map(|ts| ts > posted_at)
                    .unwrap_or(false)
            })
            .collect();

        if new_replies.is_empty() {
            return Ok(None);
        }

        let reply_count = new_replies.len();
        let human_reply = new_replies
            .iter()
            .filter_map(|r| r.get("content").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n\n");

        let mut outputs: HashMap<String, Value> = HashMap::new();
        outputs.insert("humanReply".to_string(), Value::String(human_reply));
        outputs.insert(
            "replyCount".to_string(),
            Value::Number(serde_json::Number::from(reply_count)),
        );

        Ok(Some(StepOutcome::Completed { outputs }))
    }
}
