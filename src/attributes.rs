use crate::clients::rest::FlowstateRestClient;
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;

/// Bidirectional lookup table for attribute tags and categories.
///
/// Built from a slice of JSON records fetched from the attributes collection.
/// Records missing `id`, `name`, or `type` fields are silently skipped.
#[derive(Debug, Clone, Default)]
pub struct AttributeMap {
    tag_name_to_id_map: HashMap<String, String>,
    tag_id_to_name_map: HashMap<String, String>,
    category_name_to_id_map: HashMap<String, String>,
    category_id_to_name_map: HashMap<String, String>,
}

impl AttributeMap {
    /// Load attributes from FlowState via the REST API.
    ///
    /// Queries the `attributes` collection scoped to the given org and
    /// workspace, then builds a complete lookup map.
    /// Returns an error if the REST call fails.
    pub async fn load(
        rest: &FlowstateRestClient,
        org_id: &str,
        workspace_id: &str,
    ) -> Result<Self> {
        let records: Vec<Value> = rest
            .query(
                "attributes",
                serde_json::json!({
                    "orgId": org_id,
                    "workspaceId": workspace_id
                }),
            )
            .await
            .context("Failed to load attributes from FlowState")?;
        tracing::info!(
            count = records.len(),
            "Loaded attributes for tag/category resolution"
        );
        Ok(Self::from_records(&records))
    }

    /// Build an `AttributeMap` from a slice of raw attribute records.
    ///
    /// Records that lack `id`, `name`, or `type` string fields are ignored.
    /// Unknown `type` values are also ignored without error.
    pub fn from_records(records: &[Value]) -> Self {
        let mut map = Self::default();
        for record in records {
            let id = match record.get("id").and_then(Value::as_str) {
                Some(s) => s,
                None => continue,
            };
            let name = match record.get("name").and_then(Value::as_str) {
                Some(s) => s,
                None => continue,
            };
            let attr_type = match record.get("type").and_then(Value::as_str) {
                Some(s) => s,
                None => continue,
            };
            match attr_type {
                "tag" => {
                    map.tag_name_to_id_map
                        .insert(name.to_owned(), id.to_owned());
                    map.tag_id_to_name_map
                        .insert(id.to_owned(), name.to_owned());
                }
                "category" => {
                    map.category_name_to_id_map
                        .insert(name.to_owned(), id.to_owned());
                    map.category_id_to_name_map
                        .insert(id.to_owned(), name.to_owned());
                }
                _ => {}
            }
        }
        map
    }

    /// Look up a tag ID by name. Returns `None` if the name is not registered.
    pub fn tag_name_to_id(&self, name: &str) -> Option<&str> {
        self.tag_name_to_id_map.get(name).map(String::as_str)
    }

    /// Look up a tag name by ID. Returns `None` if the ID is not registered.
    pub fn tag_id_to_name(&self, id: &str) -> Option<&str> {
        self.tag_id_to_name_map.get(id).map(String::as_str)
    }

    /// Look up a category ID by name. Returns `None` if the name is not registered.
    pub fn category_name_to_id(&self, name: &str) -> Option<&str> {
        self.category_name_to_id_map.get(name).map(String::as_str)
    }

    /// Look up a category name by ID. Returns `None` if the ID is not registered.
    pub fn category_id_to_name(&self, id: &str) -> Option<&str> {
        self.category_id_to_name_map.get(id).map(String::as_str)
    }

    /// Resolve a slice of tag names to their IDs, skipping any that are unknown.
    pub fn resolve_tag_names(&self, names: &[&str]) -> Vec<String> {
        names
            .iter()
            .filter_map(|name| self.tag_name_to_id(name).map(str::to_owned))
            .collect()
    }

    /// Resolve a slice of tag IDs to their names, skipping any that are unknown.
    pub fn resolve_tag_ids(&self, ids: &[&str]) -> Vec<String> {
        ids.iter()
            .filter_map(|id| self.tag_id_to_name(id).map(str::to_owned))
            .collect()
    }
}
