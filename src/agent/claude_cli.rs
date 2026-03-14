//! ClaudeCliExecutor — spawns the `claude` CLI as a subprocess, streams its
//! JSONL output, and converts it into the `AgentEvent`/`AgentResult` types
//! defined in the agent module.
//!
//! The core parse/extract functions are intentionally pure so they can be
//! exercised in unit tests without touching the process table.

use super::{AgentEvent, AgentExecutor, AgentResult};
use crate::models::agent::{AgentConfig, AgentMetrics};

use anyhow::{bail, Result};
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

// ---------------------------------------------------------------------------
// JSONL parsing — pure function, no I/O
// ---------------------------------------------------------------------------

/// Parse a single JSONL line emitted by the Claude CLI.
///
/// Rules:
/// - Blank / whitespace-only lines → `None`
/// - Lines that are not valid JSON → `None` (logged at trace level)
/// - Valid JSON with an unrecognised `type` field → `AgentEvent::Unknown`
/// - Valid JSON with no `type` field → `AgentEvent::Unknown`
pub fn parse_jsonl_line(line: &str) -> Option<AgentEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let value: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            tracing::trace!(line = trimmed, error = %e, "Ignoring non-JSON JSONL line");
            return None;
        }
    };

    let event_type = match value.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => {
            // No type field — treat as unknown so callers can inspect if needed
            return Some(AgentEvent::Unknown { raw: value });
        }
    };

    match event_type {
        "system" => {
            // Claude CLI emits {"type":"system","subtype":"init","model":"..."} at start
            let model = value
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(AgentEvent::Start { model })
        }
        "assistant" => {
            // Content blocks arrive as
            //   {"type":"assistant","message":{"content":[{"type":"text","text":"..."}]}}
            // or tool_use blocks:
            //   {"type":"assistant","message":{"content":[{"type":"tool_use","name":"...","input":{...}}]}}
            if let Some(blocks) = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                // Return the first recognisable block; multiple blocks per line
                // are unusual in practice and the caller can iterate calls.
                for block in blocks {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match block_type {
                        "text" => {
                            let content = block
                                .get("text")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();
                            return Some(AgentEvent::Text { content });
                        }
                        "tool_use" => {
                            let tool = block
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();
                            let input =
                                block.get("input").cloned().unwrap_or(serde_json::json!({}));
                            return Some(AgentEvent::ToolUse { tool, input });
                        }
                        _ => {}
                    }
                }
            }
            Some(AgentEvent::Unknown { raw: value })
        }
        "result" => {
            // Final result line: {"type":"result","subtype":"success","cost_usd":0.42,...}
            let cost = value.get("cost_usd").and_then(|v| v.as_f64());
            Some(AgentEvent::Complete { cost })
        }
        "error" => {
            let message = value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .or_else(|| value.get("message").and_then(|m| m.as_str()))
                .unwrap_or("unknown error")
                .to_string();
            Some(AgentEvent::Error { message })
        }
        _ => Some(AgentEvent::Unknown { raw: value }),
    }
}

// ---------------------------------------------------------------------------
// Metrics extraction — pure function
// ---------------------------------------------------------------------------

/// Extract `AgentMetrics` from a completed event slice.
///
/// - `model` comes from the first `Start` event
/// - `cost` comes from the first `Complete` event
/// - token counts are summed from `Unknown` events that contain a
///   `usage` sub-object (as emitted by some Claude CLI versions)
pub fn extract_metrics(events: &[AgentEvent]) -> AgentMetrics {
    let mut metrics = AgentMetrics {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        model: None,
        duration_ms: None,
        cost: None,
    };

    for event in events {
        match event {
            AgentEvent::Start { model } if !model.is_empty() => {
                if metrics.model.is_none() {
                    metrics.model = Some(model.clone());
                }
            }
            AgentEvent::Complete { cost } => {
                if metrics.cost.is_none() {
                    metrics.cost = *cost;
                }
            }
            AgentEvent::Unknown { raw } => {
                // Some Claude CLI builds embed usage data in a top-level
                // "usage" object, e.g.:
                //   {"type":"...","usage":{"input_tokens":100,"output_tokens":50,...}}
                if let Some(usage) = raw.get("usage") {
                    if let Some(n) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                        metrics.input_tokens += n;
                    }
                    if let Some(n) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                        metrics.output_tokens += n;
                    }
                    if let Some(n) = usage
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_u64())
                    {
                        metrics.cache_read_tokens += n;
                    }
                }
                // Model name can also appear in system/result unknown events
                if metrics.model.is_none() {
                    if let Some(m) = raw.get("model").and_then(|v| v.as_str()) {
                        metrics.model = Some(m.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    metrics
}

// ---------------------------------------------------------------------------
// Facts extraction — pure function
// ---------------------------------------------------------------------------

/// Extract files modified and tools used from a completed event slice.
///
/// Returns `(files_modified, tools_used)`.
///
/// Only `Write` and `Edit` tool invocations contribute to `files_modified`.
/// All tool invocations contribute to `tools_used`.
/// Both lists are deduplicated and sorted for deterministic output.
pub fn extract_facts(events: &[AgentEvent]) -> (Vec<String>, Vec<String>) {
    let mut files: HashSet<String> = HashSet::new();
    let mut tools: HashSet<String> = HashSet::new();

    for event in events {
        if let AgentEvent::ToolUse { tool, input } = event {
            tools.insert(tool.clone());

            // Only Write / Edit count as file modifications
            if tool == "Write" || tool == "Edit" {
                // The file path lives at input.file_path for both Write and Edit
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    files.insert(path.to_string());
                }
            }
        }
    }

    let mut files_sorted: Vec<String> = files.into_iter().collect();
    files_sorted.sort();

    let mut tools_sorted: Vec<String> = tools.into_iter().collect();
    tools_sorted.sort();

    (files_sorted, tools_sorted)
}

// ---------------------------------------------------------------------------
// Command construction — pure function for testability
// ---------------------------------------------------------------------------

/// Build the argument list for the `claude` CLI invocation.
///
/// Extracted as a pure function so tests can verify flag construction
/// without spawning a process.
pub fn build_command_args(prompt: &str, config: &AgentConfig) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // Output format must be stream-json for JSONL streaming
    args.push("--output-format".to_string());
    args.push("stream-json".to_string());

    // Verbose flag ensures we get all events
    args.push("--verbose".to_string());

    // Model selection
    if let Some(model) = &config.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }

    // Permission mode — defaults to bypassPermissions for automated runs
    let permission_mode = config
        .permission_mode
        .as_deref()
        .unwrap_or("bypassPermissions");
    args.push("--permission-mode".to_string());
    args.push(permission_mode.to_string());

    // Agent name / persona
    if let Some(agent) = &config.agent_name {
        args.push("--agent".to_string());
        args.push(agent.clone());
    }

    // The prompt is passed as the final positional argument via --print flag
    // (non-interactive mode)
    args.push("--print".to_string());
    args.push(prompt.to_string());

    args
}

/// Create a `tokio::process::Command` ready to spawn.
///
/// - Sets `working_dir` as the process CWD
/// - Overrides with `config.working_dir` if provided
/// - Removes `CLAUDECODE` so nested invocations do not detect a parent session
/// - Pipes stdout; inherits stderr so logs reach the parent's terminal
pub fn build_command(prompt: &str, config: &AgentConfig, working_dir: &Path) -> Command {
    let mut cmd = Command::new("claude");

    // working_dir parameter takes precedence; config.working_dir overrides if set
    let effective_dir = config
        .working_dir
        .as_deref()
        .map(Path::new)
        .unwrap_or(working_dir);

    cmd.current_dir(effective_dir);

    // Remove CLAUDECODE to prevent claude from detecting it's running inside
    // another claude session, which would change its behaviour
    cmd.env_remove("CLAUDECODE");

    let args = build_command_args(prompt, config);
    cmd.args(&args);

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::inherit());

    cmd
}

// ---------------------------------------------------------------------------
// ClaudeCliExecutor
// ---------------------------------------------------------------------------

/// Concrete `AgentExecutor` that drives the `claude` CLI subprocess.
///
/// Spawns `claude --output-format stream-json …`, reads JSONL from stdout,
/// fires `on_event` callbacks for each recognised event, then collects metrics
/// and facts for the returned `AgentResult`.
pub struct ClaudeCliExecutor;

#[async_trait]
impl AgentExecutor for ClaudeCliExecutor {
    async fn execute(
        &self,
        prompt: &str,
        config: &AgentConfig,
        working_dir: &Path,
        timeout_secs: Option<u64>,
        on_event: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> Result<AgentResult> {
        let mut cmd = build_command(prompt, config, working_dir);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => bail!("Failed to spawn claude CLI: {}", e),
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => bail!("claude CLI stdout was not piped"),
        };

        let mut events: Vec<AgentEvent> = Vec::new();
        let mut output_text = String::new();
        let mut had_error = false;

        let stream_future = async {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(event) = parse_jsonl_line(&line) {
                    // Collect text for the output field
                    if let AgentEvent::Text { content } = &event {
                        if !output_text.is_empty() {
                            output_text.push('\n');
                        }
                        output_text.push_str(content);
                    }
                    if matches!(event, AgentEvent::Error { .. }) {
                        had_error = true;
                    }
                    on_event(event.clone());
                    events.push(event);
                }
            }
        };

        // Apply optional wall-clock timeout
        let timed_out = if let Some(secs) = timeout_secs {
            timeout(Duration::from_secs(secs), stream_future)
                .await
                .is_err()
        } else {
            stream_future.await;
            false
        };

        if timed_out {
            // Best-effort kill — ignore kill error since we're already
            // propagating the timeout
            let _ = child.kill().await;
            bail!(
                "claude CLI timed out after {} seconds",
                timeout_secs.unwrap_or(0)
            );
        }

        let exit_status = child.wait().await;
        let exit_code = exit_status.ok().and_then(|s| s.code());

        let success = !had_error && exit_code.map(|c| c == 0).unwrap_or(false);

        let metrics = extract_metrics(&events);
        let (files_modified, tools_used) = extract_facts(&events);

        Ok(AgentResult {
            success,
            output: output_text,
            metrics,
            files_modified,
            tools_used,
            exit_code,
        })
    }
}
