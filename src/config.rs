use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Runtime configuration loaded from `.flowstate/config.json` with env overrides.
#[derive(Debug, Clone)]
pub struct Config {
    pub org_id: String,
    pub workspace_id: String,
    pub rest_base_url: String,
    pub mcp_base_url: String,
    pub obs_url: Option<String>,
    pub plan_base_dir: PathBuf,
    pub worker_mode: bool,
    pub health_port: u16,
    pub max_subprocess_depth: u32,
    pub agent_executor: String,
    /// Optional static auth token for REST and MCP clients (legacy).
    /// Prefer `api_token` + `auth_url` for automatic JWT exchange.
    /// When set, requests include a `Bearer` authorization header.
    /// Not required when running inside Docker on the internal network.
    pub auth_token: Option<String>,
    /// Long-lived `epic_*` API token for token-to-JWT exchange.
    /// When set (along with `auth_url`), the runner uses a `TokenExchanger`
    /// to obtain short-lived JWTs automatically.
    pub api_token: Option<String>,
    /// Auth server token endpoint URL for API token exchange.
    /// e.g. `http://auth-server:3001/auth/token`
    pub auth_url: Option<String>,
    /// How often to persist state during step execution.
    /// Persist every N completed steps. Default 1 (every step).
    /// Also always persists on pause, fail, and completion regardless.
    pub persist_interval: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileConfig {
    #[serde(default)]
    org_id: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    max_subprocess_depth: Option<u32>,
    #[serde(default)]
    agent_executor: Option<String>,
    #[serde(default)]
    persist_interval: Option<u32>,
}

impl Config {
    pub fn load(project_root: &Path) -> Result<Self> {
        let config_path = project_root.join(".flowstate/config.json");
        let file_content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;

        let file_config: FileConfig = serde_json::from_str(&file_content)
            .with_context(|| format!("Failed to parse {}", config_path.display()))?;

        let org_id = file_config
            .org_id
            .filter(|s| !s.is_empty())
            .context("orgId is required in .flowstate/config.json")?;

        let workspace_id = file_config.workspace_id.unwrap_or_default();

        Ok(Config {
            org_id,
            workspace_id,
            rest_base_url: std::env::var("FLOWSTATE_REST_URL")
                .unwrap_or_else(|_| "http://localhost:7080".to_string()),
            mcp_base_url: std::env::var("FLOWSTATE_MCP_URL")
                .unwrap_or_else(|_| "http://localhost:7080/mcp".to_string()),
            obs_url: std::env::var("OBS_SERVER_URL").ok(),
            plan_base_dir: project_root.join(".flowstate/plans"),
            worker_mode: std::env::var("WORKER_MODE")
                .map(|v| v == "true")
                .unwrap_or(false),
            health_port: std::env::var("HEALTH_PORT")
                .ok()
                .and_then(|v| match v.parse::<u16>() {
                    Ok(p) => Some(p),
                    Err(_) => {
                        eprintln!("WARNING: Invalid HEALTH_PORT '{}', using default 9090", v);
                        None
                    }
                })
                .unwrap_or(9090),
            max_subprocess_depth: std::env::var("MAX_SUBPROCESS_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(file_config.max_subprocess_depth)
                .unwrap_or(5),
            agent_executor: std::env::var("AGENT_EXECUTOR")
                .ok()
                .or(file_config.agent_executor)
                .unwrap_or_else(|| "claude-cli".to_string()),
            auth_token: std::env::var("FLOWSTATE_AUTH_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
            api_token: std::env::var("FLOWSTATE_API_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
            auth_url: std::env::var("FLOWSTATE_AUTH_URL")
                .ok()
                .filter(|s| !s.is_empty()),
            persist_interval: std::env::var("PERSIST_INTERVAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(file_config.persist_interval)
                .unwrap_or(1)
                .max(1), // Minimum 1 to prevent infinite loops
        })
    }
}
