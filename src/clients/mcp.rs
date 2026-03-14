use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::RwLock;

/// Client for the FlowState MCP server's REST transport.
/// Used for orchestration tools: proposal-create, mission-status,
/// step-claim, step-complete.
pub struct McpClient {
    http: Client,
    base_url: String,
    org_id: String,
    workspace_id: String,
    session_id: Option<String>,
    /// Bearer token for authenticated requests. Uses `RwLock` for interior
    /// mutability so daemon mode can refresh the JWT without requiring `&mut self`.
    auth_token: RwLock<Option<String>>,
}

impl McpClient {
    /// Create a new MCP client. Call `set_context()` before using `call_tool()`.
    pub fn new(base_url: &str, org_id: &str, workspace_id: &str) -> Self {
        Self::with_auth(base_url, org_id, workspace_id, None)
    }

    /// Create a new MCP client with optional auth token.
    pub fn with_auth(
        base_url: &str,
        org_id: &str,
        workspace_id: &str,
        auth_token: Option<String>,
    ) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");
        McpClient {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            org_id: org_id.to_string(),
            workspace_id: workspace_id.to_string(),
            session_id: None,
            auth_token: RwLock::new(auth_token),
        }
    }

    /// The current session ID, if set.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Apply auth header to a request builder if a token is configured.
    fn apply_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let token = self.auth_token.read().unwrap().clone();
        match token {
            Some(t) => request.bearer_auth(t),
            None => request,
        }
    }

    /// Update the auth token. Used by daemon mode to refresh expired JWTs
    /// without rebuilding the client.
    pub fn set_auth_token(&self, token: Option<String>) {
        *self.auth_token.write().unwrap() = token;
    }

    /// Set the org/workspace context on the MCP server.
    /// Must be called before using orchestration tools.
    /// Captures the `mcp-session-id` header from the response.
    pub async fn set_context(&mut self) -> Result<()> {
        let url = format!("{}/tools/set-context", self.base_url);
        let body = serde_json::json!({
            "orgId": self.org_id,
            "workspaceId": self.workspace_id,
        });

        let resp = self
            .apply_auth(self.http.post(&url))
            .json(&body)
            .send()
            .await
            .with_context(|| "MCP set-context failed")?
            .error_for_status()
            .with_context(|| "MCP set-context returned error")?;

        // Capture session ID from response header (skip if not valid UTF-8)
        if let Some(session_id) = resp.headers().get("mcp-session-id") {
            if let Ok(session_str) = session_id.to_str() {
                self.session_id = Some(session_str.to_string());
            }
        }

        Ok(())
    }

    /// Call an MCP tool by name with the given arguments.
    /// Returns the inner tool result, unwrapping the HTTP server's
    /// `{ success, toolName, result }` envelope.
    pub async fn call_tool(&self, tool_name: &str, args: Value) -> Result<Value> {
        let url = format!("{}/tools/{}", self.base_url, tool_name);

        let mut request = self.apply_auth(self.http.post(&url)).json(&args);

        if let Some(ref session_id) = self.session_id {
            request = request.header("mcp-session-id", session_id);
        }

        let resp = request
            .send()
            .await
            .with_context(|| format!("MCP call_tool failed: {tool_name}"))?
            .error_for_status()
            .with_context(|| format!("MCP call_tool returned error: {tool_name}"))?;

        let body: Value = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse MCP response: {tool_name}"))?;

        // The MCP HTTP server wraps tool results as { success, toolName, result }.
        // Unwrap to return the inner `result` directly. If there's no `result`
        // field (e.g. the server format changed), fall back to the raw body.
        Ok(body.get("result").cloned().unwrap_or(body))
    }

    /// Query a collection and deserialize the results.
    ///
    /// Calls `collection-query` and unwraps the `{ documents: [...] }` envelope.
    /// Returns an empty `Vec` when the `documents` field is absent.
    pub async fn query<T: DeserializeOwned>(
        &self,
        collection: &str,
        selector: Value,
        limit: Option<u32>,
    ) -> Result<Vec<T>> {
        let mut args = serde_json::json!({ "collection": collection, "selector": selector });
        if let Some(l) = limit {
            args["limit"] = serde_json::json!(l);
        }
        let resp = self.call_tool("collection-query", args).await?;
        let docs = resp
            .get("documents")
            .cloned()
            .unwrap_or(Value::Array(vec![]));
        serde_json::from_value(docs).context("Failed to deserialize MCP query response")
    }

    /// Fetch a single document from a collection and deserialize it.
    ///
    /// Calls `collection-get` and unwraps the `{ document: {...} }` envelope.
    pub async fn get<T: DeserializeOwned>(&self, collection: &str, id: &str) -> Result<T> {
        let resp = self
            .call_tool(
                "collection-get",
                serde_json::json!({ "collection": collection, "id": id }),
            )
            .await?;
        let doc = resp
            .get("document")
            .cloned()
            .ok_or_else(|| anyhow!("MCP get response missing 'document' field"))?;
        serde_json::from_value(doc).context("Failed to deserialize MCP get response")
    }

    /// Create a new document in a collection.
    ///
    /// Calls `collection-create` and returns the new document's ID from the
    /// `{ documentId: "..." }` envelope.
    pub async fn create(&self, collection: &str, data: &Value) -> Result<String> {
        let resp = self
            .call_tool(
                "collection-create",
                serde_json::json!({ "collection": collection, "data": data }),
            )
            .await?;
        resp.get("documentId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("MCP create response missing 'documentId'"))
    }

    /// Update an existing document in a collection.
    ///
    /// Calls `collection-update` with a partial data patch. Returns `()` on success.
    pub async fn update(&self, collection: &str, id: &str, data: &Value) -> Result<()> {
        self.call_tool(
            "collection-update",
            serde_json::json!({ "collection": collection, "id": id, "data": data }),
        )
        .await?;
        Ok(())
    }
}
