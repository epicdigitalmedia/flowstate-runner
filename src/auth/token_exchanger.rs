use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::Instant;

/// Response from the auth server `POST /auth/token` endpoint
/// when using `grant_type=api_token`.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    /// Lifetime of the JWT in seconds.
    expires_in: u64,
}

/// A cached JWT along with its expiry instant.
#[derive(Debug, Clone)]
struct CachedJwt {
    token: String,
    expires_at: Instant,
}

/// Exchanges a long-lived `epic_*` API token for a short-lived JWT
/// via the auth server's `grant_type=api_token` flow.
///
/// Caches the JWT and automatically refreshes it 5 minutes before expiry
/// to avoid request failures. Thread-safe via `Arc<RwLock<...>>`.
pub struct TokenExchanger {
    api_token: String,
    auth_url: String,
    cached: Arc<RwLock<Option<CachedJwt>>>,
    client: Client,
}

/// How many seconds before expiry to trigger a proactive refresh.
const REFRESH_BUFFER_SECS: u64 = 300; // 5 minutes

/// Minimum effective lifetime in seconds after subtracting the refresh buffer.
/// Prevents tight refresh loops when the server returns a very short `expires_in`.
const MIN_EFFECTIVE_LIFETIME_SECS: u64 = 30;

impl TokenExchanger {
    /// Create a new `TokenExchanger`.
    ///
    /// - `api_token` — the `epic_*` API token stored in the runner's config
    /// - `auth_url` — the auth server token endpoint, e.g.
    ///   `http://auth-server:3001/auth/token`
    ///
    /// Returns an error if the HTTP client cannot be constructed.
    pub fn new(api_token: String, auth_url: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .context("Failed to build HTTP client for TokenExchanger")?;

        Ok(TokenExchanger {
            api_token,
            auth_url,
            cached: Arc::new(RwLock::new(None)),
            client,
        })
    }

    /// Get a valid JWT, returning a cached one if it has not expired.
    /// Performs a token exchange if no cached JWT exists or if the
    /// cached JWT is within `REFRESH_BUFFER_SECS` of expiry.
    pub async fn get_token(&self) -> Result<String> {
        // Fast path: check read lock
        {
            let guard = self.cached.read().await;
            if let Some(ref cached) = *guard {
                if Instant::now() < cached.expires_at {
                    return Ok(cached.token.clone());
                }
            }
        }

        // Slow path: acquire write lock and exchange
        let mut guard = self.cached.write().await;

        // Double-check after acquiring write lock (another task may have refreshed)
        if let Some(ref cached) = *guard {
            if Instant::now() < cached.expires_at {
                return Ok(cached.token.clone());
            }
        }

        let new_cached = self.exchange().await?;
        let token = new_cached.token.clone();
        *guard = Some(new_cached);
        Ok(token)
    }

    /// Exchange the API token for a JWT via the auth server.
    async fn exchange(&self) -> Result<CachedJwt> {
        let body = [
            ("grant_type", "api_token"),
            ("api_token", &self.api_token),
        ];

        let resp = self
            .client
            .post(&self.auth_url)
            .form(&body)
            .send()
            .await
            .with_context(|| format!("Token exchange request failed: POST {}", self.auth_url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Token exchange failed (HTTP {}): {}",
                status.as_u16(),
                body_text
            );
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .with_context(|| "Failed to parse token exchange response")?;

        // Apply refresh buffer so we refresh before the token actually expires.
        // Clamp to MIN_EFFECTIVE_LIFETIME_SECS to prevent tight refresh loops
        // when the server returns expires_in <= REFRESH_BUFFER_SECS.
        let effective_lifetime = token_resp
            .expires_in
            .saturating_sub(REFRESH_BUFFER_SECS)
            .max(MIN_EFFECTIVE_LIFETIME_SECS);

        Ok(CachedJwt {
            token: token_resp.access_token,
            expires_at: Instant::now() + std::time::Duration::from_secs(effective_lifetime),
        })
    }
}
