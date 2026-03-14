use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Typed execution status for future migration from string-based status.
/// Currently scaffolded — the executor uses string constants (`STATUS_*`)
/// for DB compatibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Paused {
        step_id: String,
        reason: PauseReason,
    },
    Completed {
        elapsed_ms: u64,
    },
    Failed {
        step_id: String,
        error: String,
    },
}

/// The reason an execution is paused. Persisted to `metadata._pause_reason`
/// in the DB record so the resumer can inspect why execution stopped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PauseReason {
    Approval {
        approval_id: String,
    },
    HumanTask {
        discussion_id: String,
        posted_at: String,
    },
    Subprocess {
        child_execution_id: String,
    },
    AgentTask {
        discussion_id: String,
        posted_at: String,
    },
}

/// Contextual information about what triggered an execution: the entity type
/// and ID, the user who owns the entity, and subprocess depth tracking.
///
/// The `tags` field uses a custom deserializer to accept both a single string
/// (`"brainstorm"`) and an array (`["brainstorm"]`), since the bash process
/// runner stored tags as a plain string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionContext {
    #[serde(default)]
    pub entity_type: String,
    #[serde(default)]
    pub entity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub depth: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    /// Process name stored in context by the bash runner. Optional for
    /// backwards compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
}

/// Deserialize a value that may be a single string or an array of strings.
fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct StringOrVec;

    impl<'de> de::Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or array of strings")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Vec<String>, E> {
            if value.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![value.to_owned()])
            }
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut vec = Vec::new();
            while let Some(item) = seq.next_element::<String>()? {
                vec.push(item);
            }
            Ok(vec)
        }

        fn visit_none<E: de::Error>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }

        fn visit_unit<E: de::Error>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }
    }

    deserializer.deserialize_any(StringOrVec)
}

fn default_max_depth() -> u32 {
    5
}

/// A record of a single step execution within the step history array.
///
/// The bash runner stored fields as `name`/`type` while the Rust runner uses
/// `stepName`/`stepType` (camelCase). Serde aliases handle both formats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepHistoryEntry {
    pub step_id: String,
    #[serde(default, alias = "name", skip_serializing_if = "Option::is_none")]
    pub step_name: Option<String>,
    #[serde(default, alias = "type", skip_serializing_if = "Option::is_none")]
    pub step_type: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessExecutionRecord {
    pub id: String,
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_version: Option<String>,
    pub org_id: String,
    pub workspace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step_id: Option<String>,
    #[serde(default)]
    pub variables: Map<String, Value>,
    #[serde(default)]
    pub step_history: Vec<StepHistoryEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ExecutionContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_empty_object")]
    pub metadata: Value,
    pub created_at: String,
    pub updated_at: String,
}

use super::default_empty_object;

fn default_max_retries() -> u32 {
    3
}

use std::collections::HashMap;

/// The outcome returned by a handler after executing a step.
#[derive(Debug, Clone)]
pub enum StepOutcome {
    /// Step completed successfully with outputs to map to variables.
    Completed { outputs: HashMap<String, Value> },
    /// Step needs to pause waiting for external input.
    Paused(PauseReason),
    /// Step failed with an error message.
    Failed { error: String },
}

/// In-memory working state for an execution. Wraps the DB record with
/// typed fields and runtime-only state. Created from `ProcessExecutionRecord`
/// via `from_record()`, persisted back via `to_record()` (Phase 2).
#[derive(Debug, Clone)]
pub struct ExecutionState {
    pub id: String,
    pub process_id: String,
    pub process_name: String,
    pub external_id: Option<String>,
    pub status: String,
    pub current_step_id: Option<String>,
    pub variables: Map<String, Value>,
    pub step_history: Vec<StepHistoryEntry>,
    pub context: Option<ExecutionContext>,
    pub metadata: Map<String, Value>,
    pub plan_dir: Option<String>,
    pub started_at: Option<String>,
    pub progress: Option<u32>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub outputs: Option<Value>,
    pub org_id: String,
    pub workspace_id: String,
    pub user_id: Option<String>,
    pub process_version: Option<String>,
    pub parent_execution_id: Option<String>,
    pub depth: Option<u32>,
    pub inputs: Option<Value>,
    pub error: Option<Value>,
    pub archived: bool,
    pub created_at: String,
    pub completed_at: Option<String>,
    /// The reason execution is paused. `None` when not paused. Persisted to
    /// `metadata._pause_reason` so resume logic can inspect why we stopped.
    pub pause_reason: Option<PauseReason>,
}

impl ExecutionState {
    /// Create an in-memory state from a DB record.
    pub fn from_record(record: ProcessExecutionRecord, process_name: String) -> Self {
        let metadata_map = record.metadata.as_object().cloned().unwrap_or_default();

        // Restore pause_reason from metadata if present; ignore decode errors
        // (unknown variants become None rather than crashing on resume).
        let pause_reason: Option<PauseReason> = metadata_map
            .get("_pause_reason")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        ExecutionState {
            id: record.id,
            process_id: record.process_id,
            process_name,
            external_id: record.external_id,
            status: record.status,
            current_step_id: record.current_step_id,
            variables: record.variables,
            step_history: record.step_history,
            context: record.context,
            metadata: metadata_map,
            plan_dir: None,
            started_at: record.started_at,
            progress: record.progress,
            retry_count: record.retry_count,
            max_retries: record.max_retries,
            outputs: record.outputs,
            org_id: record.org_id,
            workspace_id: record.workspace_id,
            user_id: record.user_id,
            process_version: record.process_version,
            parent_execution_id: record.parent_execution_id,
            depth: record.depth,
            inputs: record.inputs,
            error: record.error,
            archived: record.archived,
            created_at: record.created_at,
            completed_at: record.completed_at,
            pause_reason,
        }
    }

    /// Calculate duration_ms from started_at and completed_at if both are present.
    fn compute_duration_ms(&self) -> Option<u64> {
        let started = self.started_at.as_ref()?;
        let completed = self.completed_at.as_ref()?;
        let start: chrono::DateTime<chrono::Utc> = started.parse().ok()?;
        let end: chrono::DateTime<chrono::Utc> = completed.parse().ok()?;
        let duration = end.signed_duration_since(start);
        if duration.num_milliseconds() >= 0 {
            Some(duration.num_milliseconds() as u64)
        } else {
            None
        }
    }

    /// Convert in-memory state back to a DB record for persistence.
    pub fn to_record(&self) -> ProcessExecutionRecord {
        // Build metadata with _pause_reason inserted or removed as appropriate.
        // Keeping pause_reason in metadata (rather than a top-level column) avoids
        // a schema migration and keeps the DB record shape stable.
        let mut metadata = self.metadata.clone();
        match &self.pause_reason {
            Some(reason) => match serde_json::to_value(reason) {
                Ok(encoded) => {
                    metadata.insert("_pause_reason".to_string(), encoded);
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to serialize PauseReason — dropping");
                }
            },
            None => {
                metadata.remove("_pause_reason");
            }
        }

        ProcessExecutionRecord {
            id: self.id.clone(),
            process_id: self.process_id.clone(),
            process_version: self.process_version.clone(),
            org_id: self.org_id.clone(),
            workspace_id: self.workspace_id.clone(),
            user_id: self.user_id.clone(),
            status: self.status.clone(),
            progress: self.progress,
            started_at: self.started_at.clone(),
            completed_at: self.completed_at.clone(),
            duration_ms: self.compute_duration_ms(),
            current_step_id: self.current_step_id.clone(),
            variables: self.variables.clone(),
            step_history: self.step_history.clone(),
            context: self.context.clone(),
            external_id: self.external_id.clone(),
            parent_execution_id: self.parent_execution_id.clone(),
            depth: self.depth,
            retry_count: self.retry_count,
            max_retries: self.max_retries,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            error: self.error.clone(),
            archived: self.archived,
            metadata: Value::Object(metadata),
            created_at: self.created_at.clone(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// A fully resolved step: the result of merging a ProcessStep with its
/// StepTemplate (if template_id is set). All fields are final.
#[derive(Debug, Clone)]
pub struct ResolvedStep {
    pub id: String,
    pub process_id: String,
    pub name: String,
    pub step_type: String,
    pub action: Option<Value>,
    pub inputs: Option<Value>,
    pub outputs: Vec<Value>,
    pub output_extraction: Option<Value>,
    pub conditions: Vec<Value>,
    pub next_step_id: Option<String>,
    pub required_variables: Vec<String>,
    pub estimated_duration_minutes: Option<u32>,
    pub metadata: Map<String, Value>,
}
