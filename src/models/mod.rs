use serde_json::Value;

pub mod agent;
pub mod approval;
pub mod execution;
pub mod process;
pub mod trigger;

/// Default serde helper: returns `{}` instead of `null` for Value fields.
pub(crate) fn default_empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

// Re-export commonly used types at the models level
pub use agent::{
    AgentConfig, AgentMetrics, ExtractionMode, OutputExtraction, OutputSpec, StepInput,
};
pub use approval::ApprovalRecord;
pub use execution::{
    ExecutionContext, ExecutionState, ExecutionStatus, PauseReason, ProcessExecutionRecord,
    ResolvedStep, StepHistoryEntry, StepOutcome,
};
pub use process::{Process, ProcessStep, StepTemplate};
pub use trigger::{Op, StepCondition, TriggerCondition, TriggerConfig};
