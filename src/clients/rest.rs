use anyhow::{Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::RwLock;

/// HTTP client for the FlowState RxDB REST API.
///
/// Transparently handles both native collections (direct REST endpoints)
/// and virtual collections (VCA, backed by the `records` collection).
/// Virtual collections are identified by checking the `schema_map` —
/// if a collection name has a schema mapping, it routes through
/// `records-rest` with schemaId filtering and data-bag flattening.
pub struct FlowstateRestClient {
    http: Client,
    base_url: String,
    schema_versions: HashMap<String, u32>,
    /// Bearer token for authenticated requests. Uses `RwLock` for interior
    /// mutability so daemon mode can refresh the JWT without requiring `&mut self`.
    auth_token: RwLock<Option<String>>,
    /// Mapping from virtual collection names to their schema IDs.
    /// Populated by `load_schemas()` at startup.
    schema_map: HashMap<String, String>,
}

#[derive(serde::Deserialize)]
struct QueryResponse<T> {
    documents: Vec<T>,
}

/// Record-level fields that exist at the top level of a VCA record
/// (not inside the `data` bag). Used to determine which selector
/// fields need a `data.` prefix for virtual collection queries.
const RECORD_LEVEL_FIELDS: &[&str] = &[
    "id",
    "orgId",
    "workspaceId",
    "schemaId",
    "title",
    "status",
    "archived",
    "completed",
    "createdAt",
    "updatedAt",
    "userId",
    "_deleted",
    "_rev",
    "_attachments",
    "_meta",
];

/// Default schema versions matching the RxDB server schema definitions.
/// Version numbers must match the server-side schema `version` field.
pub fn default_schema_versions() -> HashMap<String, u32> {
    let mut versions = HashMap::new();
    versions.insert("records".to_string(), 0);
    versions.insert("tasks".to_string(), 0);
    versions.insert("milestones".to_string(), 0);
    versions.insert("projects".to_string(), 0);
    versions.insert("discussions".to_string(), 4);
    versions.insert("approvals".to_string(), 2);
    versions.insert("documents".to_string(), 3);
    versions.insert("schemas".to_string(), 0);
    versions.insert("workers".to_string(), 1);
    versions
}

/// Native RxDB collections that have their own REST endpoints.
/// Schemas with these names must NOT be routed through VCA (records-rest).
const NATIVE_COLLECTIONS: &[&str] = &[
    "records",
    "tasks",
    "milestones",
    "projects",
    "discussions",
    "approvals",
    "documents",
    "schemas",
    "workers",
];

impl FlowstateRestClient {
    /// Create a new client with default schema versions and no auth.
    pub fn new(base_url: &str) -> Self {
        Self::with_options(base_url, default_schema_versions(), None)
    }

    /// Create a client with custom schema versions and optional auth token.
    pub fn with_options(
        base_url: &str,
        schema_versions: HashMap<String, u32>,
        auth_token: Option<String>,
    ) -> Self {
        FlowstateRestClient {
            http: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            schema_versions,
            auth_token: RwLock::new(auth_token),
            schema_map: HashMap::new(),
        }
    }

    fn schema_version(&self, collection: &str) -> u32 {
        self.schema_versions.get(collection).copied().unwrap_or(0)
    }

    /// Check if a collection is virtual (VCA-backed by records).
    pub fn is_virtual(&self, collection: &str) -> bool {
        self.schema_map.contains_key(collection)
    }

    fn url(&self, collection: &str, operation: &str) -> String {
        format!(
            "{}/{}-rest/{}/{}",
            self.base_url,
            collection,
            self.schema_version(collection),
            operation
        )
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

    /// Load schema definitions from the RxDB server and build the
    /// virtual collection name → schemaId mapping for this org.
    pub async fn load_schemas(&mut self, org_id: &str) -> Result<()> {
        let url = self.url("schemas", "query");
        let body = json!({ "selector": { "orgId": org_id }, "limit": 500 });

        let resp = self
            .apply_auth(self.http.post(&url))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to load schemas: POST {url}"))?
            .error_for_status()
            .with_context(|| format!("Schema load returned error: POST {url}"))?;

        let query_resp: QueryResponse<Value> = resp
            .json()
            .await
            .with_context(|| "Failed to parse schema response")?;

        self.schema_map.clear();
        for schema in &query_resp.documents {
            if let (Some(name), Some(id)) = (
                schema.get("name").and_then(Value::as_str),
                schema.get("id").and_then(Value::as_str),
            ) {
                // Skip schemas whose names collide with native RxDB collections.
                // Native collections have their own REST endpoints and must not
                // be routed through the VCA (records-rest) path.
                if NATIVE_COLLECTIONS.contains(&name) {
                    tracing::warn!(
                        schema_name = %name,
                        schema_id = %id,
                        "Skipping schema that collides with native collection name"
                    );
                    continue;
                }
                self.schema_map.insert(name.to_string(), id.to_string());
            }
        }

        tracing::info!(
            count = self.schema_map.len(),
            "Loaded virtual collection schemas"
        );

        Ok(())
    }

    /// Query documents with default limit of 100.
    /// Automatically routes virtual collections through records-rest.
    pub async fn query<T: DeserializeOwned>(
        &self,
        collection: &str,
        selector: Value,
    ) -> Result<Vec<T>> {
        self.query_with_limit(collection, selector, 100).await
    }

    /// Query documents with explicit limit.
    /// Automatically routes virtual collections through records-rest.
    pub async fn query_with_limit<T: DeserializeOwned>(
        &self,
        collection: &str,
        selector: Value,
        limit: u32,
    ) -> Result<Vec<T>> {
        if let Some(schema_id) = self.schema_map.get(collection) {
            self.query_virtual(schema_id, selector, limit).await
        } else {
            self.query_native(collection, selector, limit).await
        }
    }

    /// Query a native collection directly.
    async fn query_native<T: DeserializeOwned>(
        &self,
        collection: &str,
        selector: Value,
        limit: u32,
    ) -> Result<Vec<T>> {
        let url = self.url(collection, "query");
        let body = json!({ "selector": selector, "limit": limit });

        let resp = self
            .apply_auth(self.http.post(&url))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("REST query failed: POST {url}"))?
            .error_for_status()
            .with_context(|| format!("REST query returned error: POST {url}"))?;

        let query_resp: QueryResponse<T> = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse query response from {url}"))?;

        Ok(query_resp.documents)
    }

    /// Query a virtual collection through records-rest.
    /// Adds schemaId to the selector, prefixes domain fields with `data.`,
    /// and flattens the `data` bag in each result document.
    async fn query_virtual<T: DeserializeOwned>(
        &self,
        schema_id: &str,
        selector: Value,
        limit: u32,
    ) -> Result<Vec<T>> {
        let url = self.url("records", "query");

        // Build selector with schemaId and data-bag prefixed fields
        let full_selector = build_vca_selector(schema_id, &selector);
        let body = json!({ "selector": full_selector, "limit": limit });

        let resp = self
            .apply_auth(self.http.post(&url))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("REST virtual query failed: POST {url}"))?
            .error_for_status()
            .with_context(|| format!("REST virtual query returned error: POST {url}"))?;

        let query_resp: QueryResponse<Value> = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse virtual query response from {url}"))?;

        // Flatten data bag and deserialize.
        // Records that fail to deserialize are logged and skipped rather
        // than aborting the entire query. This handles legacy records
        // (e.g. from the bash runner) with unexpected field shapes.
        let mut results = Vec::new();
        for doc in query_resp.documents {
            let flat = flatten_vca_record(doc);
            let id = flat
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned();
            match serde_json::from_value::<T>(flat.clone()) {
                Ok(parsed) => results.push(parsed),
                Err(e) => {
                    tracing::warn!(
                        record_id = %id,
                        error = %e,
                        "Skipping VCA record that failed to deserialize"
                    );
                }
            }
        }
        Ok(results)
    }

    /// Get a single document by ID.
    /// For virtual collections, routes through records-rest and flattens the data bag.
    pub async fn get<T: DeserializeOwned>(&self, collection: &str, id: &str) -> Result<T> {
        if self.schema_map.contains_key(collection) {
            self.get_virtual(id).await
        } else {
            self.get_native(collection, id).await
        }
    }

    /// Get from a native collection directly.
    async fn get_native<T: DeserializeOwned>(&self, collection: &str, id: &str) -> Result<T> {
        let url = format!(
            "{}/{}-rest/{}/{}",
            self.base_url,
            collection,
            self.schema_version(collection),
            id
        );

        let resp = self
            .apply_auth(self.http.get(&url))
            .send()
            .await
            .with_context(|| format!("REST get failed: GET {url}"))?
            .error_for_status()
            .with_context(|| format!("REST get returned error: GET {url}"))?;

        resp.json()
            .await
            .with_context(|| format!("Failed to parse get response from {url}"))
    }

    /// Get a virtual collection record by ID via records-rest.
    /// Record IDs are globally unique — no schemaId needed for lookups.
    async fn get_virtual<T: DeserializeOwned>(&self, id: &str) -> Result<T> {
        let url = format!(
            "{}/records-rest/{}/{}",
            self.base_url,
            self.schema_version("records"),
            id
        );

        let resp = self
            .apply_auth(self.http.get(&url))
            .send()
            .await
            .with_context(|| format!("REST virtual get failed: GET {url}"))?
            .error_for_status()
            .with_context(|| format!("REST virtual get returned error: GET {url}"))?;

        let doc: Value = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse virtual get response from {url}"))?;

        let flat = flatten_vca_record(doc);
        serde_json::from_value(flat).with_context(|| "Failed to deserialize flattened VCA record")
    }

    /// Write documents via `/set`. Body is an array of full documents.
    /// For virtual collections, wraps domain fields into the VCA data bag
    /// and writes through records-rest.
    pub async fn set<T: Serialize>(&self, collection: &str, docs: &[T]) -> Result<()> {
        if let Some(schema_id) = self.schema_map.get(collection) {
            let schema_id = schema_id.clone();
            self.set_as_virtual(&schema_id, docs).await
        } else {
            self.set_native(collection, docs).await
        }
    }

    /// Write to a native collection directly.
    async fn set_native<T: Serialize>(&self, collection: &str, docs: &[T]) -> Result<()> {
        let url = self.url(collection, "set");

        self.apply_auth(self.http.post(&url))
            .json(&docs)
            .send()
            .await
            .with_context(|| format!("REST set failed: POST {url}"))?
            .error_for_status()
            .with_context(|| format!("REST set returned error: POST {url}"))?;

        Ok(())
    }

    /// Write to a virtual collection via records-rest.
    /// Wraps domain-specific fields into the `data` bag and adds `schemaId`.
    async fn set_as_virtual<T: Serialize>(&self, schema_id: &str, docs: &[T]) -> Result<()> {
        let url = self.url("records", "set");

        // Convert docs to Value, then wrap each into VCA format
        let wrapped: Vec<Value> = docs
            .iter()
            .map(|doc| {
                let val =
                    serde_json::to_value(doc).expect("Serializable doc should convert to Value");
                wrap_vca_record(val, schema_id)
            })
            .collect();

        let resp = self
            .apply_auth(self.http.post(&url))
            .json(&wrapped)
            .send()
            .await
            .with_context(|| format!("REST virtual set failed: POST {url}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                %status,
                body = %body,
                "REST virtual set error response"
            );
            anyhow::bail!("REST virtual set returned {status}: POST {url} — {body}");
        }

        Ok(())
    }

    /// Soft-delete: GET doc, set `_deleted: true`, write via `/set`.
    pub async fn delete(&self, collection: &str, id: &str) -> Result<()> {
        if self.schema_map.contains_key(collection) {
            // Virtual collection: get raw record, mark deleted, write back
            let url = format!(
                "{}/records-rest/{}/{}",
                self.base_url,
                self.schema_version("records"),
                id
            );
            let resp = self
                .apply_auth(self.http.get(&url))
                .send()
                .await?
                .error_for_status()?;
            let mut doc: Value = resp.json().await?;
            doc["_deleted"] = json!(true);
            let set_url = self.url("records", "set");
            self.apply_auth(self.http.post(&set_url))
                .json(&[doc])
                .send()
                .await?
                .error_for_status()?;
            Ok(())
        } else {
            let mut doc: Value = self.get(collection, id).await?;
            doc["_deleted"] = json!(true);
            self.set_native(collection, &[doc]).await
        }
    }
}

/// Build a VCA-compatible selector by adding schemaId and prefixing
/// domain-specific fields with `data.`.
fn build_vca_selector(schema_id: &str, selector: &Value) -> Value {
    let mut full = json!({ "schemaId": schema_id });

    if let Value::Object(map) = selector {
        if let Value::Object(ref mut full_map) = full {
            for (key, value) in map {
                if is_record_level_field(key) {
                    full_map.insert(key.clone(), value.clone());
                } else {
                    full_map.insert(format!("data.{}", key), value.clone());
                }
            }
        }
    }

    full
}

/// Flatten a VCA record by merging `data` bag fields into the top level.
/// After merging, the `data` key is removed to avoid leaving a stale
/// nested object that could confuse downstream deserializers.
fn flatten_vca_record(mut doc: Value) -> Value {
    if let Some(data) = doc.get("data").cloned() {
        if let (Value::Object(data_map), Value::Object(ref mut doc_map)) = (data, &mut doc) {
            for (key, value) in data_map {
                // Domain fields from data bag go to top level;
                // don't overwrite existing record-level fields
                if !doc_map.contains_key(&key) {
                    doc_map.insert(key, value);
                }
            }
            // Remove the now-redundant data bag
            doc_map.remove("data");
        }
    }
    doc
}

/// Wrap a flat document into VCA record format.
/// Record-level fields stay at top level; all other fields go into `data`.
fn wrap_vca_record(doc: Value, schema_id: &str) -> Value {
    if let Value::Object(map) = doc {
        let mut record = serde_json::Map::new();
        let mut data = serde_json::Map::new();

        for (key, value) in map {
            if is_record_level_field(&key) {
                record.insert(key, value);
            } else {
                data.insert(key, value);
            }
        }

        record.insert("schemaId".to_string(), json!(schema_id));
        record.insert("data".to_string(), Value::Object(data));

        Value::Object(record)
    } else {
        doc
    }
}

/// Check if a field name is a record-level field (not stored in the data bag).
fn is_record_level_field(name: &str) -> bool {
    RECORD_LEVEL_FIELDS.contains(&name)
}
