use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A process definition record from the FlowState `processes` collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Process {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub status: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub start_step_id: Option<String>,
    pub org_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub trigger: Option<ProcessTrigger>,
    #[serde(default)]
    pub execution_config: Option<ExecutionConfig>,
    #[serde(default)]
    pub input_schema: Option<Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub max_subprocess_depth: Option<u32>,
    #[serde(default)]
    pub document_id: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_empty_object")]
    pub metadata: Value,
    #[serde(default = "default_empty_object")]
    pub extended: Value,
}

use super::default_empty_object;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTrigger {
    #[serde(rename = "type")]
    pub trigger_type: String,
    #[serde(default)]
    pub entity_trigger: Option<EntityTrigger>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntityTrigger {
    pub entity_type: String,
    #[serde(default)]
    pub selector: Value,
    #[serde(default)]
    pub conditions: Vec<EntityTriggerCondition>,
    #[serde(default)]
    pub debounce: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntityTriggerCondition {
    pub property_path: String,
    pub operator: String,
    #[serde(default)]
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionConfig {
    #[serde(default)]
    pub max_concurrent_executions: Option<u32>,
    #[serde(default)]
    pub queue_behavior: Option<String>,
    #[serde(default)]
    pub timeout_minutes: Option<u32>,
    #[serde(default)]
    pub singleton: Option<bool>,
    #[serde(default)]
    pub priority: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStep {
    pub id: String,
    pub process_id: String,
    pub org_id: String,
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub step_type: String,
    #[serde(default)]
    pub order: Option<f64>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub next_step_id: Option<String>,
    #[serde(default)]
    pub action: Option<Value>,
    #[serde(default)]
    pub conditions: Vec<Value>,
    #[serde(default)]
    pub inputs: Option<Value>,
    #[serde(default)]
    pub outputs: Vec<Value>,
    #[serde(default)]
    pub required_variables: Vec<String>,
    #[serde(default)]
    pub output_extraction: Option<Value>,
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub estimated_duration_minutes: Option<u32>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_empty_object")]
    pub metadata: Value,
    #[serde(default = "default_empty_object")]
    pub extended: Value,
    pub created_at: String,
    pub updated_at: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepTemplate {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub step_type: String,
    pub org_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub action: Option<Value>,
    #[serde(default)]
    pub inputs: Option<Value>,
    #[serde(default)]
    pub outputs: Vec<Value>,
    #[serde(default)]
    pub output_extraction: Option<Value>,
    #[serde(default)]
    pub required_variables: Vec<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_empty_object")]
    pub metadata: Value,
    pub created_at: String,
    pub updated_at: String,
}
