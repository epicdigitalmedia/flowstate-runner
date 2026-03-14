use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

/// Fire-and-forget observability client.
/// Sends log entries to the obs-server. Errors are traced, never propagated.
/// Disabled when `obs_url` is None.
pub struct ObsClient {
    http: Client,
    base_url: Option<String>,
}

/// A log entry to send to the obs-server.
#[derive(Debug, Clone, Serialize)]
pub struct ObsEntry {
    pub level: String,
    pub message: String,
    pub context: Value,
}

impl ObsClient {
    /// Create a new obs client. Pass `None` to disable.
    pub fn new(obs_url: Option<String>) -> Self {
        ObsClient {
            http: Client::new(),
            base_url: obs_url.map(|u| u.trim_end_matches('/').to_string()),
        }
    }

    /// Send a log entry. Fire-and-forget: errors are traced, never returned.
    pub fn send(&self, entry: ObsEntry) {
        let Some(ref base_url) = self.base_url else {
            return;
        };

        let http = self.http.clone();
        let url = format!("{}/api/logs", base_url);

        tokio::spawn(async move {
            if let Err(e) = http.post(&url).json(&entry).send().await {
                tracing::debug!("obs send failed: {e}");
            }
        });
    }
}
