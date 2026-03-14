pub mod anthropic_api;
pub mod claude_cli;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::models::agent::{AgentConfig, AgentMetrics};

/// Streaming events emitted by an agent executor during a run.
///
/// Consumers receive these via the `on_event` callback in `AgentExecutor::execute`.
/// The `Unknown` variant captures any event the executor does not recognise, so
/// callers are never surprised by a parse failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The agent has started; `model` names the model being invoked.
    Start { model: String },
    /// The agent is calling a tool; `input` is the raw JSON argument payload.
    ToolUse {
        tool: String,
        input: serde_json::Value,
    },
    /// The agent produced a text fragment.
    Text { content: String },
    /// The agent finished; `cost` is the USD cost of the run (when available).
    Complete { cost: Option<f64> },
    /// The agent reported an error mid-stream.
    Error { message: String },
    /// An event type the executor did not recognise. `raw` preserves the
    /// original JSON so callers can inspect it without losing information.
    Unknown { raw: serde_json::Value },
}

/// Final outcome of an agent run.
#[derive(Debug, Clone)]
pub struct AgentResult {
    /// Whether the agent completed without error.
    pub success: bool,
    /// Combined text output produced by the agent.
    pub output: String,
    /// Token usage and timing metrics.
    pub metrics: AgentMetrics,
    /// Paths of files the agent created or modified, relative to `working_dir`.
    pub files_modified: Vec<String>,
    /// Names of tools the agent invoked during the run.
    pub tools_used: Vec<String>,
    /// Process exit code, when the executor launches a subprocess.
    pub exit_code: Option<i32>,
}

/// Abstraction over any agent back-end that can execute a prompt.
///
/// Implementations must be `Send + Sync` so they can be stored in `RunContext`
/// and shared across async task boundaries.
#[async_trait]
pub trait AgentExecutor: Send + Sync {
    /// Execute the given `prompt` and return a fully-resolved `AgentResult`.
    ///
    /// # Arguments
    /// * `prompt`       — The instruction text sent to the agent.
    /// * `config`       — Per-agent configuration (model, permission mode, …).
    /// * `working_dir`  — Working directory for any subprocesses launched.
    /// * `timeout_secs` — Wall-clock timeout; `None` means no limit.
    /// * `on_event`     — Callback invoked for each streaming `AgentEvent`.
    ///                    Implementations MUST call it in the order events are
    ///                    received and MUST NOT hold locks while calling it.
    async fn execute(
        &self,
        prompt: &str,
        config: &AgentConfig,
        working_dir: &Path,
        timeout_secs: Option<u64>,
        on_event: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> Result<AgentResult>;
}

// ---------------------------------------------------------------------------
// No-op executor — used in tests so RunContext can be constructed without a
// real agent back-end.  Returns success immediately with empty outputs.
// ---------------------------------------------------------------------------

/// A do-nothing `AgentExecutor` for unit-test helpers.
///
/// Returns `AgentResult { success: true, … }` with all other fields empty or
/// zero.  Never invokes `on_event`.  Should not be used in production code.
pub struct NoopAgentExecutor;

#[async_trait]
impl AgentExecutor for NoopAgentExecutor {
    async fn execute(
        &self,
        _prompt: &str,
        _config: &AgentConfig,
        _working_dir: &Path,
        _timeout_secs: Option<u64>,
        _on_event: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> Result<AgentResult> {
        Ok(AgentResult {
            success: true,
            output: String::new(),
            metrics: AgentMetrics {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                model: None,
                duration_ms: None,
                cost: None,
            },
            files_modified: vec![],
            tools_used: vec![],
            exit_code: None,
        })
    }
}
