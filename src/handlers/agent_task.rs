use crate::handlers::{Handler, RunContext};
use crate::models::agent::AgentConfig;
use crate::models::execution::{ExecutionState, PauseReason, ResolvedStep, StepOutcome};
use crate::template::interpolate_str;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

pub struct AgentTaskHandler;

// ---------------------------------------------------------------------------
// Pure helper functions — tested independently, no async needed
// ---------------------------------------------------------------------------

/// Build the prompt text for the agent by interpolating `action.prompt` with
/// the current execution variables. If the action also carries a
/// `systemContext` field, it is prepended as a "fenced system block" — a
/// block of background context text separated from the prompt by two newlines
/// — so the agent receives the full picture.
///
/// Returns an empty string when no `prompt` field is present; the caller
/// should treat that as a hard failure.
pub fn build_agent_prompt(action: &Value, variables: &Map<String, Value>) -> String {
    let system_context = action
        .get("systemContext")
        .and_then(Value::as_str)
        .unwrap_or("");

    let prompt_template = match action.get("prompt").and_then(Value::as_str) {
        Some(t) => t,
        None => return String::new(),
    };

    let interpolated = interpolate_str(prompt_template, variables);

    if system_context.is_empty() {
        interpolated.into_owned()
    } else {
        format!("{}\n\n{}", system_context, interpolated)
    }
}

/// Extract agent configuration fields from the action value.
///
/// All fields are optional at the JSON level; missing ones become `None`.
/// The `systemContext` field maps to `memory_context` in `AgentConfig`
/// (the memory / context blob the agent receives as background).
pub fn extract_agent_config(action: &Value) -> AgentConfig {
    AgentConfig {
        agent_name: action
            .get("agentName")
            .and_then(Value::as_str)
            .map(str::to_owned),
        provider: action
            .get("provider")
            .and_then(Value::as_str)
            .map(str::to_owned),
        model: action
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned),
        timeout: action.get("timeout").and_then(Value::as_u64),
        // systemContext is also used as the agent's memory / background context
        memory_context: action
            .get("systemContext")
            .and_then(Value::as_str)
            .map(str::to_owned),
        working_dir: action
            .get("workingDir")
            .and_then(Value::as_str)
            .map(str::to_owned),
        permission_mode: action
            .get("permissionMode")
            .and_then(Value::as_str)
            .map(str::to_owned),
        team_member_id: action
            .get("teamMemberId")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

/// Return `true` when all output files listed in `action.outputFiles` exist
/// on disk with non-empty content, meaning this agent invocation can be
/// skipped (already done in a previous run).
///
/// Returns `false` whenever `plan_dir` is `None`, the array is absent, or
/// any file is missing / empty. Errors reading files are treated as
/// "not done yet" to avoid silently skipping a file that should be written.
pub fn should_skip_agent(action: &Value, plan_dir: Option<&str>) -> bool {
    let dir = match plan_dir {
        Some(d) => d,
        None => return false,
    };

    let files = match action.get("outputFiles").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => arr,
        // No outputFiles spec → nothing to check, do not skip
        _ => return false,
    };

    files.iter().all(|f| {
        let name = match f.as_str() {
            Some(s) => s,
            None => return false,
        };
        let path = Path::new(dir).join(name);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > 0 => true,
            // File missing or empty → cannot skip
            _ => false,
        }
    })
}

/// Read output files listed in `action.outputFiles` and return them as a
/// variable map keyed by filename stem (e.g. `"design.md"` → key `"design"`).
///
/// Files that are valid JSON are stored as `Value::Object`/`Value::Array`/etc.
/// All other files are stored as `Value::String` with their raw content.
/// Missing files are silently omitted rather than causing a hard failure;
/// the agent already ran successfully so partial output is still useful.
pub fn collect_output_files(action: &Value, plan_dir: Option<&str>) -> HashMap<String, Value> {
    let mut out = HashMap::new();

    let dir = match plan_dir {
        Some(d) => d,
        None => return out,
    };

    let files = match action.get("outputFiles").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return out,
    };

    for f in files {
        let name = match f.as_str() {
            Some(s) => s,
            None => continue,
        };

        let path = Path::new(dir).join(name);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Derive variable name from the filename stem (part before first '.').
        let stem = Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name);

        // Attempt JSON parse first; fall back to raw string.
        // unwrap_or_else is needed here: `content` is borrowed by from_str,
        // so we can only move it into Value::String on the error path.
        #[allow(clippy::unnecessary_lazy_evaluations)]
        let value =
            serde_json::from_str::<Value>(&content).unwrap_or_else(|_| Value::String(content));

        out.insert(stem.to_owned(), value);
    }

    out
}

// ---------------------------------------------------------------------------
// Handler implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Handler for AgentTaskHandler {
    /// Execute an agent-task step.
    ///
    /// Flow:
    /// 1. Guard: action must be present.
    /// 2. Skip if all output files already exist with content (resume safety).
    /// 3. Build prompt; fail if empty.
    /// 4. Write prompt.md for audit trail (best-effort).
    /// 5. Invoke the configured `AgentExecutor`.
    /// 6. On success: collect output files; attach metrics.
    /// 7. On failure: return `Failed` with the error + first 500 chars of output.
    async fn execute(
        &self,
        step: &ResolvedStep,
        state: &ExecutionState,
        ctx: &RunContext,
    ) -> Result<StepOutcome> {
        // 1. Require action
        let raw_action = match &step.action {
            Some(a) => a,
            None => {
                return Ok(StepOutcome::Failed {
                    error: "agent-task step missing action configuration".to_string(),
                });
            }
        };

        // Unwrap nested "agent" sub-object if present (e.g., { "type": "agent", "agent": { "prompt": ... } })
        let action = raw_action
            .get("agent")
            .filter(|a| a.is_object())
            .unwrap_or(raw_action);

        let plan_dir = state.plan_dir.as_deref();

        // 2. Skip if all output files already produced
        if should_skip_agent(action, plan_dir) {
            tracing::info!(
                step_id = %step.id,
                "Skipping agent-task: all output files already exist"
            );
            let mut outputs = collect_output_files(action, plan_dir);
            outputs.insert("_skipped".to_string(), Value::Bool(true));
            return Ok(StepOutcome::Completed { outputs });
        }

        // 3. Build prompt
        let prompt = build_agent_prompt(action, &state.variables);
        if prompt.is_empty() {
            return Ok(StepOutcome::Failed {
                error: "agent-task action missing 'prompt' field".to_string(),
            });
        }

        // 4. Write prompt.md for audit trail (best-effort — never fail the step)
        if let Some(dir) = plan_dir {
            let prompt_path = Path::new(dir).join("prompt.md");
            if let Ok(mut file) = std::fs::File::create(&prompt_path) {
                let _ = file.write_all(prompt.as_bytes());
            }
        }

        // 5. Invoke agent executor
        let config = extract_agent_config(action);
        let timeout = config.timeout;

        // The executor trait requires &Path, not Option<&str>.
        let working_dir = plan_dir.map(Path::new).unwrap_or_else(|| Path::new("."));

        // Collect streaming events for telemetry; log them at debug level.
        let on_event = |event: crate::agent::AgentEvent| {
            tracing::debug!(
                step_id = %step.id,
                event = ?event,
                "Agent event"
            );
        };

        let result = ctx
            .agent_executor
            .execute(&prompt, &config, working_dir, timeout, &on_event)
            .await?;

        // 6. Success path
        if result.success {
            let mut outputs = collect_output_files(action, plan_dir);

            // Metrics forwarded as structured variables for downstream steps.
            if let Ok(metrics_val) = serde_json::to_value(&result.metrics) {
                outputs.insert("_agentMetrics".to_string(), metrics_val);
            }

            outputs.insert(
                "_filesModified".to_string(),
                Value::Array(
                    result
                        .files_modified
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );

            outputs.insert(
                "_toolsUsed".to_string(),
                Value::Array(
                    result
                        .tools_used
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );

            return Ok(StepOutcome::Completed { outputs });
        }

        // 7. Failure path: include first 500 chars of output for context
        let snippet: String = result.output.chars().take(500).collect();
        let error = if snippet.is_empty() {
            "Agent task failed with no output".to_string()
        } else {
            format!("Agent task failed: {}", snippet)
        };

        Ok(StepOutcome::Failed { error })
    }

    /// Check whether a paused agent-task step can resume.
    ///
    /// An agent-task pauses with `PauseReason::AgentTask` when the agent
    /// posts a question and waits for a human reply in a discussion thread.
    ///
    /// Resume flow:
    /// 1. Query discussion replies with `parentId = discussion_id`.
    /// 2. Client-side filter: keep only replies created after `posted_at`
    ///    (RxDB REST does not support server-side date comparisons).
    /// 3. No new replies → still waiting; return `Ok(None)`.
    /// 4. Replies found → inject as `humanReply` variable, append to
    ///    `qa_history.md`, re-run agent with updated context.
    async fn check_resume(
        &self,
        step: &ResolvedStep,
        state: &ExecutionState,
        reason: &PauseReason,
        ctx: &RunContext,
    ) -> Result<Option<StepOutcome>> {
        let (discussion_id, posted_at) = match reason {
            PauseReason::AgentTask {
                discussion_id,
                posted_at,
            } => (discussion_id.as_str(), posted_at.as_str()),
            // We only handle our own pause reason.
            _ => return Ok(None),
        };

        // 1. Query replies to this discussion thread
        let replies: Vec<Value> = ctx
            .rest
            .query(
                "discussions",
                serde_json::json!({ "parentId": discussion_id }),
            )
            .await?;

        // 2. Client-side date filter (VCA REST does not support $gt on dates)
        let new_replies: Vec<&Value> = replies
            .iter()
            .filter(|r| {
                r.get("createdAt")
                    .and_then(Value::as_str)
                    .map(|ts| ts > posted_at)
                    .unwrap_or(false)
            })
            .collect();

        // 3. Nothing new yet
        if new_replies.is_empty() {
            return Ok(None);
        }

        // 4. Build combined reply text
        let human_reply = new_replies
            .iter()
            .filter_map(|r| r.get("content").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n\n");

        // Merge reply into variables so the re-run agent sees it
        let mut variables = state.variables.clone();
        variables.insert("humanReply".to_string(), Value::String(human_reply.clone()));

        // Append exchange to qa_history.md (best-effort)
        if let Some(dir) = state.plan_dir.as_deref() {
            let qa_path = Path::new(dir).join("qa_history.md");
            let entry = format!(
                "\n\n---\n**Human reply ({})**\n\n{}\n",
                chrono::Utc::now().to_rfc3339(),
                human_reply
            );
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&qa_path)
            {
                let _ = file.write_all(entry.as_bytes());
            }
        }

        // Re-run agent with updated context
        let raw_action = match &step.action {
            Some(a) => a,
            None => {
                return Ok(Some(StepOutcome::Failed {
                    error: "agent-task step missing action on resume".to_string(),
                }));
            }
        };

        // Unwrap nested "agent" sub-object if present
        let action = raw_action
            .get("agent")
            .filter(|a| a.is_object())
            .unwrap_or(raw_action);

        let prompt = build_agent_prompt(action, &variables);
        if prompt.is_empty() {
            return Ok(Some(StepOutcome::Failed {
                error: "agent-task action missing 'prompt' field on resume".to_string(),
            }));
        }

        let config = extract_agent_config(action);
        let timeout = config.timeout;
        let working_dir = state
            .plan_dir
            .as_deref()
            .map(Path::new)
            .unwrap_or_else(|| Path::new("."));

        let on_event = |event: crate::agent::AgentEvent| {
            tracing::debug!(step_id = %step.id, event = ?event, "Agent resume event");
        };

        let result = ctx
            .agent_executor
            .execute(&prompt, &config, working_dir, timeout, &on_event)
            .await?;

        if result.success {
            let plan_dir = state.plan_dir.as_deref();
            let mut outputs = collect_output_files(action, plan_dir);

            if let Ok(metrics_val) = serde_json::to_value(&result.metrics) {
                outputs.insert("_agentMetrics".to_string(), metrics_val);
            }
            outputs.insert(
                "_filesModified".to_string(),
                Value::Array(
                    result
                        .files_modified
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );
            outputs.insert(
                "_toolsUsed".to_string(),
                Value::Array(
                    result
                        .tools_used
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );

            return Ok(Some(StepOutcome::Completed { outputs }));
        }

        let snippet: String = result.output.chars().take(500).collect();
        let error = if snippet.is_empty() {
            "Agent task failed on resume with no output".to_string()
        } else {
            format!("Agent task failed on resume: {}", snippet)
        };

        Ok(Some(StepOutcome::Failed { error }))
    }
}
