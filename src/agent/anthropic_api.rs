//! AnthropicApiExecutor — placeholder for direct Anthropic API integration.
//!
//! Currently returns bail!() — all agent tasks use ClaudeCliExecutor.
//! This module will implement direct HTTP calls to the Anthropic API
//! (models.anthropic.com) for future integration.

use super::{AgentEvent, AgentExecutor, AgentResult};
use crate::models::agent::AgentConfig;

use anyhow::bail;
use async_trait::async_trait;
use std::path::Path;

/// Placeholder executor for direct Anthropic API integration.
pub struct AnthropicApiExecutor;

#[async_trait]
impl AgentExecutor for AnthropicApiExecutor {
    async fn execute(
        &self,
        _prompt: &str,
        _config: &AgentConfig,
        _working_dir: &Path,
        _timeout_secs: Option<u64>,
        _on_event: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> anyhow::Result<AgentResult> {
        bail!("AnthropicApiExecutor not yet implemented — use ClaudeCliExecutor")
    }
}
