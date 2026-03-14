use crate::attributes::AttributeMap;
use crate::auth::TokenExchanger;
use crate::cache::TtlCache;
use crate::clients::mcp::McpClient;
use crate::clients::rest::FlowstateRestClient;
use crate::config::Config;
use crate::handlers::{create_agent_executor, RunContext};
use crate::models::process::StepTemplate;
use anyhow::{Context, Result};
#[allow(unused_imports)]
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;

/// Load all step templates from FlowState via MCP.
///
/// `steptemplates` is a VCA-backed virtual collection, so it must be
/// queried through MCP rather than the native REST endpoints.
///
/// The REST client transparently handles the `steptemplates` virtual
/// collection by routing through `records-rest` with schema filtering.
///
/// Returns a `HashMap` keyed by template ID for efficient lookup during
/// step resolution. An empty result is valid — it means no templates have
/// been defined yet, not that the operation failed.
pub async fn load_templates(mcp: &McpClient) -> Result<HashMap<String, StepTemplate>> {
    let templates: Vec<StepTemplate> = mcp
        .query("steptemplates", json!({}), None)
        .await
        .context("Failed to load step templates")?;

    tracing::info!(count = templates.len(), "Loaded step templates");

    Ok(templates.into_iter().map(|t| (t.id.clone(), t)).collect())
}

/// Initialize MCP client with retry backoff.
///
/// The MCP server may be lazy-initializing when the runner starts.
/// Retries `max_retries` times with exponential backoff starting at
/// `initial_delay_ms` milliseconds.
pub async fn init_mcp_with_retry(
    mcp: &mut McpClient,
    max_retries: u32,
    initial_delay_ms: u64,
) -> Result<()> {
    // When max_retries is 0, try exactly once with no retry loop
    if max_retries == 0 {
        return mcp
            .set_context()
            .await
            .context("MCP set-context failed (no retries configured)");
    }

    let mut delay = initial_delay_ms;
    for attempt in 1..=max_retries {
        match mcp.set_context().await {
            Ok(()) => {
                tracing::info!(attempt, "MCP set-context succeeded");
                return Ok(());
            }
            Err(e) => {
                if attempt == max_retries {
                    return Err(e).context(format!(
                        "MCP set-context failed after {} attempts",
                        max_retries
                    ));
                }
                tracing::warn!(
                    attempt,
                    max_retries,
                    delay_ms = delay,
                    error = %e,
                    "MCP set-context failed, retrying"
                );
                tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                delay = std::cmp::min(delay * 2, 5000); // cap at 5s
            }
        }
    }
    anyhow::bail!(
        "MCP set-context failed after {} attempts (unexpected loop exit)",
        max_retries
    )
}

/// Build a fully initialized `RunContext` from configuration.
///
/// Steps:
/// 1. Create REST client
/// 2. Create and initialize MCP client (with retry backoff)
/// 3. Load virtual collection schemas into REST client
/// 4. Load step templates (via MCP — steptemplates is a VCA collection)
/// 5. Load attribute map (tag/category name-to-ID resolution)
///
/// Resolve the auth token and optional token exchanger.
///
/// Priority:
/// 1. If `FLOWSTATE_API_TOKEN` and `FLOWSTATE_AUTH_URL` are set, exchange
///    the API token for a short-lived JWT via the auth server.
///    The `TokenExchanger` is returned so it can be stored for later refresh.
/// 2. Otherwise, fall back to the static `FLOWSTATE_AUTH_TOKEN` env var.
/// 3. If neither is set, return `None` (unauthenticated, internal Docker use).
async fn resolve_auth_token(config: &Config) -> Result<(Option<String>, Option<TokenExchanger>)> {
    if let (Some(api_token), Some(auth_url)) = (&config.api_token, &config.auth_url) {
        tracing::info!("Exchanging API token for JWT via {}", auth_url);
        let exchanger = TokenExchanger::new(api_token.clone(), auth_url.clone())
            .context("Failed to create TokenExchanger")?;
        let jwt = exchanger
            .get_token()
            .await
            .context("Failed to exchange API token for JWT")?;
        return Ok((Some(jwt), Some(exchanger)));
    }

    if config.auth_token.is_some() {
        tracing::debug!("Using static FLOWSTATE_AUTH_TOKEN");
    }

    Ok((config.auth_token.clone(), None))
}

/// 6. Create agent executor
///
/// Returns `(RunContext, templates)` so the caller can pass templates
/// to the executor or resumer. Virtual collections (VCA) are routed
/// through MCP for CRUD operations. The REST client also loads schemas
/// for transparent VCA query routing through `records-rest`.
///
/// # Retry behavior
///
/// Uses 3 retries with 100ms initial delay (capped at 5s). This is enough to
/// handle a briefly-unavailable MCP server without blocking startup for tens of
/// seconds in misconfigured environments.
pub async fn build_run_context(
    config: Config,
) -> Result<(RunContext, HashMap<String, StepTemplate>)> {
    // 1. Resolve auth token — prefer API token exchange, fall back to static token
    let (auth_token, token_exchanger) = resolve_auth_token(&config).await?;

    // 2. REST client (with resolved auth token)
    let mut rest = FlowstateRestClient::with_options(
        &config.rest_base_url,
        crate::clients::rest::default_schema_versions(),
        auth_token.clone(),
    );

    // 3. MCP client with retry — used for set-context initialization
    let mut mcp = McpClient::with_auth(
        &config.mcp_base_url,
        &config.org_id,
        &config.workspace_id,
        auth_token,
    );
    init_mcp_with_retry(&mut mcp, 3, 100).await?;

    // 3. Load virtual collection schemas into REST client
    // This populates the schema_map so the REST client can transparently
    // route virtual collection queries through records-rest.
    rest.load_schemas(&config.org_id)
        .await
        .context("Failed to load virtual collection schemas")?;

    // 4. Load templates (via MCP — steptemplates is a VCA collection)
    // Templates are optional — if the collection doesn't exist or loading
    // fails, continue with an empty map. Steps resolve without templates.
    let templates = match load_templates(&mcp).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(
                error = format!("{:#}", e),
                "Template loading failed — continuing without templates"
            );
            HashMap::new()
        }
    };

    // 5. Load attribute map (scoped to org/workspace)
    let attribute_map = AttributeMap::load(&rest, &config.org_id, &config.workspace_id).await?;

    // 6. Agent executor
    let agent_executor = create_agent_executor(&config.agent_executor);

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(format!("flowstate-runner/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Failed to build HTTP client")?;

    let ctx = RunContext {
        config,
        rest,
        http,
        mcp,
        agent_executor,
        attribute_map,
        process_cache: std::sync::Mutex::new(TtlCache::new(Duration::from_secs(60))),
        step_cache: std::sync::Mutex::new(TtlCache::new(Duration::from_secs(60))),
        token_exchanger,
    };

    Ok((ctx, templates))
}
