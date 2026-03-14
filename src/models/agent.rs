use serde::{Deserialize, Serialize};

/// Configuration for an agent-task step specifying which AI agent to invoke.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub memory_context: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub team_member_id: Option<String>,
}

/// Token usage and performance metrics collected from an agent execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMetrics {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub cost: Option<f64>,
}

/// Specification for mapping a handler output to an execution variable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutputSpec {
    pub name: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub json_path: Option<String>,
    #[serde(default)]
    pub target_variable: Option<String>,
    #[serde(default, rename = "type")]
    pub value_type: Option<String>,
    #[serde(default)]
    pub default_value: Option<serde_json::Value>,
}

/// Configuration for extracting structured data from raw handler output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutputExtraction {
    pub mode: ExtractionMode,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub merge_result: bool,
}

/// The extraction strategy used to parse handler output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExtractionMode {
    /// Use a jq expression to extract data (not yet implemented).
    Jq,
    /// Use a regex pattern with capture groups.
    Regex,
    /// Use a custom script for extraction (not yet implemented).
    Script,
}

/// A named input parameter for a process step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepInput {
    pub name: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub description: Option<String>,
}
