pub mod action;
pub mod agent_task;
pub mod approval;
pub mod decision;
pub mod delay;
pub mod end;
pub mod human_task;
pub mod notification;
pub mod start;
pub mod subprocess;

use crate::agent::AgentExecutor;
use crate::attributes::AttributeMap;
use crate::auth::TokenExchanger;
use crate::cache::TtlCache;
use crate::clients::mcp::McpClient;
use crate::clients::rest::FlowstateRestClient;
use crate::config::Config;
use crate::models::execution::{ExecutionState, PauseReason, ResolvedStep, StepOutcome};
use crate::models::process::{Process, ProcessStep};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use std::collections::HashMap;

/// Collections backed by the VCA (Virtual Collection Adapter) in the MCP server.
/// These are stored in the `records` collection and are not directly queryable
/// via the native RxDB REST endpoints — they must go through MCP tools.
const VCA_COLLECTIONS: &[&str] = &[
    "processexecutions",
    "processes",
    "processsteps",
    "steptemplates",
    "teammembers",
    "schemas",
    "ops_policy",
    "trigger_rules",
];

/// Returns `true` if `collection` is a VCA-backed virtual collection that
/// must be accessed through MCP convenience methods rather than REST.
fn is_vca(collection: &str) -> bool {
    VCA_COLLECTIONS.contains(&collection)
}

/// Shared runtime context for handlers and the executor.
/// Phase 2: config + REST client. Phase 3 adds http client for ActionHandler
/// HTTP sub-type. Phase 4 adds agent_executor for AgentTask handler.
pub struct RunContext {
    pub config: Config,
    pub rest: FlowstateRestClient,
    pub http: reqwest::Client,
    /// MCP client for querying virtual collections (processes, processsteps, etc.)
    /// that are backed by the `records` collection and not directly accessible via REST.
    pub mcp: McpClient,
    /// Executor implementation for agent-task steps.
    /// Use `NoopAgentExecutor` in tests; `ClaudeCliExecutor` in production
    /// (added in Task 3).
    pub agent_executor: Box<dyn AgentExecutor>,
    /// Pre-loaded attribute lookup table for tag/category name-to-ID resolution.
    /// Use `AttributeMap::default()` in tests; loaded from REST in production.
    pub attribute_map: AttributeMap,
    /// TTL cache for process definitions (avoids re-querying in daemon mode).
    pub process_cache: std::sync::Mutex<TtlCache<Process>>,
    /// TTL cache for process step lists (avoids re-querying in daemon mode).
    pub step_cache: std::sync::Mutex<TtlCache<Vec<ProcessStep>>>,
    /// Optional token exchanger for refreshing JWTs from API tokens.
    /// Stored here so long-running daemon mode can re-exchange tokens on expiry.
    pub token_exchanger: Option<TokenExchanger>,
}

impl RunContext {
    /// Query documents from a collection.
    /// VCA collections route through MCP `collection-query`; native collections
    /// use REST directly.
    pub async fn query<T: DeserializeOwned>(
        &self,
        collection: &str,
        selector: serde_json::Value,
    ) -> Result<Vec<T>> {
        if is_vca(collection) {
            self.mcp.query(collection, selector, None).await
        } else {
            self.rest.query(collection, selector).await
        }
    }

    /// Get a single document by ID.
    /// VCA collections route through MCP `collection-get`; native collections
    /// use REST directly.
    pub async fn get<T: DeserializeOwned>(&self, collection: &str, id: &str) -> Result<T> {
        if is_vca(collection) {
            self.mcp.get(collection, id).await
        } else {
            self.rest.get(collection, id).await
        }
    }

    /// Write (upsert) a document.
    /// VCA collections route through MCP `collection-create` (the returned ID
    /// is ignored); native collections use REST `/set` directly.
    pub async fn set(&self, collection: &str, doc: &serde_json::Value) -> Result<()> {
        if is_vca(collection) {
            self.mcp.create(collection, doc).await?;
            Ok(())
        } else {
            self.rest.set(collection, &[doc]).await
        }
    }

    /// Update an existing document.
    /// VCA collections route through MCP `collection-update` for merge
    /// semantics (partial update); native collections use REST upsert.
    pub async fn update(&self, collection: &str, id: &str, data: &serde_json::Value) -> Result<()> {
        if is_vca(collection) {
            self.mcp.update(collection, id, data).await
        } else {
            // For native collections, use REST set (upsert)
            let mut doc = match data {
                serde_json::Value::Object(_) => data.clone(),
                _ => {
                    bail!("RunContext::update expects a JSON object, got {}", data);
                }
            };
            doc["id"] = serde_json::json!(id);
            self.rest.set(collection, &[doc]).await
        }
    }

    /// Get a process definition, using the cache if available.
    ///
    /// On cache hit the database is not queried. On miss, loads from the
    /// `processes` virtual collection and stores the result for future calls.
    pub async fn get_process_cached(&self, process_id: &str) -> Result<Process> {
        // Check cache first
        if let Ok(cache) = self.process_cache.lock() {
            if let Some(process) = cache.get(process_id) {
                return Ok(process.clone());
            }
        }

        // Cache miss — load from database
        let process: Process = self.get("processes", process_id).await?;

        if let Ok(mut cache) = self.process_cache.lock() {
            cache.insert(process_id.to_string(), process.clone());
        }

        Ok(process)
    }

    /// Get process steps, using the cache if available.
    ///
    /// Cache key is the process ID. On miss, queries the `processsteps`
    /// virtual collection scoped to the configured org and workspace.
    pub async fn get_steps_cached(&self, process_id: &str) -> Result<Vec<ProcessStep>> {
        let cache_key = process_id.to_string();

        // Check cache first
        if let Ok(cache) = self.step_cache.lock() {
            if let Some(steps) = cache.get(&cache_key) {
                return Ok(steps.clone());
            }
        }

        // Cache miss — load from database
        let steps: Vec<ProcessStep> = self
            .query(
                "processsteps",
                serde_json::json!({
                    "processId": process_id,
                    "orgId": self.config.org_id,
                    "workspaceId": self.config.workspace_id,
                }),
            )
            .await?;

        if let Ok(mut cache) = self.step_cache.lock() {
            cache.insert(cache_key, steps.clone());
        }

        Ok(steps)
    }

    /// Refresh the JWT on both clients if a `TokenExchanger` is configured.
    ///
    /// The exchanger internally caches the token and only calls the auth
    /// server when the current JWT is close to expiry, so this is cheap
    /// to call on every daemon loop iteration.
    pub async fn refresh_auth_if_needed(&self) -> Result<()> {
        if let Some(ref exchanger) = self.token_exchanger {
            let jwt = exchanger.get_token().await?;
            self.rest.set_auth_token(Some(jwt.clone()));
            self.mcp.set_auth_token(Some(jwt));
        }
        Ok(())
    }

    /// Evict expired entries from all caches. Call periodically in daemon mode
    /// to prevent unbounded memory growth.
    pub fn evict_caches(&self) {
        if let Ok(mut cache) = self.process_cache.lock() {
            cache.evict_expired();
        }
        if let Ok(mut cache) = self.step_cache.lock() {
            cache.evict_expired();
        }
    }
}

/// Trait that all step handlers must implement.
#[async_trait]
pub trait Handler: Send + Sync {
    /// Execute the step. Returns the outcome (completed, paused, or failed).
    async fn execute(
        &self,
        step: &ResolvedStep,
        state: &ExecutionState,
        ctx: &RunContext,
    ) -> Result<StepOutcome>;

    /// Check if a paused step can resume. Returns Some(outcome) if ready,
    /// None if still waiting. Default: not resumable.
    async fn check_resume(
        &self,
        _step: &ResolvedStep,
        _state: &ExecutionState,
        _reason: &PauseReason,
        _ctx: &RunContext,
    ) -> Result<Option<StepOutcome>> {
        Ok(None)
    }
}

/// Select executor implementation based on config string.
/// Returns `ClaudeCliExecutor` by default if executor name is not recognised.
pub fn create_agent_executor(executor_name: &str) -> Box<dyn AgentExecutor> {
    match executor_name {
        "claude-cli" => Box::new(crate::agent::claude_cli::ClaudeCliExecutor),
        "anthropic-api" => Box::new(crate::agent::anthropic_api::AnthropicApiExecutor),
        other => {
            tracing::warn!(
                executor = other,
                "Unknown agent executor, defaulting to claude-cli"
            );
            Box::new(crate::agent::claude_cli::ClaudeCliExecutor)
        }
    }
}

/// Map a step type string to its handler implementation.
/// Phase 2: all handlers are stubs that return Completed with empty outputs.
pub fn dispatch_handler(step_type: &str) -> Result<Box<dyn Handler>> {
    match step_type {
        "start" => Ok(Box::new(start::StartHandler)),
        "parallel-gateway" => Ok(Box::new(StubHandler("parallel-gateway"))),
        "join-gateway" => Ok(Box::new(StubHandler("join-gateway"))),
        "end" => Ok(Box::new(end::EndHandler)),
        "action" | "script" | "api-call" => Ok(Box::new(action::ActionHandler)),
        "decision" => Ok(Box::new(decision::DecisionHandler)),
        "delay" => Ok(Box::new(delay::DelayHandler)),
        "notification" => Ok(Box::new(notification::NotificationHandler)),
        "agent-task" => Ok(Box::new(agent_task::AgentTaskHandler)),
        "approval" => Ok(Box::new(approval::ApprovalHandler)),
        "human-task" => Ok(Box::new(human_task::HumanTaskHandler)),
        "subprocess" => Ok(Box::new(subprocess::SubprocessHandler)),
        _ => bail!("Unknown step type: {}", step_type),
    }
}

/// Stub handler for Phase 2. Returns Completed with empty outputs.
/// Replaced by real implementations in Phases 3 and 4.
struct StubHandler(&'static str);

#[async_trait]
impl Handler for StubHandler {
    async fn execute(
        &self,
        _step: &ResolvedStep,
        _state: &ExecutionState,
        _ctx: &RunContext,
    ) -> Result<StepOutcome> {
        tracing::debug!(
            step_type = self.0,
            "Stub handler executed — no-op for unimplemented step type"
        );
        Ok(StepOutcome::Completed {
            outputs: HashMap::new(),
        })
    }
}
