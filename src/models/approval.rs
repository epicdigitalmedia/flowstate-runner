use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A record from the `approvals` collection representing a pending, approved,
/// or rejected approval request tied to a process execution step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRecord {
    pub id: String,
    pub process_execution_id: String,
    pub step_id: String,
    pub status: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub response: Option<String>,
    #[serde(default)]
    pub annotations: Option<Value>,
    #[serde(default)]
    pub reviewer_id: Option<String>,
    #[serde(default)]
    pub document_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    pub org_id: String,
    pub workspace_id: String,
    #[serde(default = "default_empty_object")]
    pub metadata: Value,
    pub created_at: String,
    pub updated_at: String,
}

use super::default_empty_object;
