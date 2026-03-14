use crate::handlers::{Handler, RunContext};
use crate::models::execution::{ExecutionState, ResolvedStep, StepOutcome};
use crate::template::interpolate_json;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// Handles the `start` step type.
///
/// Merges step inputs into execution variables. Input values may contain
/// `${var}` references which are interpolated against current variables.
/// Returns `StepOutcome::Completed` with the merged inputs as outputs
/// (the executor's direct-merge fallback writes them to variables).
pub struct StartHandler;

#[async_trait]
impl Handler for StartHandler {
    async fn execute(
        &self,
        step: &ResolvedStep,
        state: &ExecutionState,
        _ctx: &RunContext,
    ) -> Result<StepOutcome> {
        let mut outputs = HashMap::new();

        if let Some(Value::Object(inputs)) = &step.inputs {
            for (key, value) in inputs {
                let interpolated = interpolate_json(value, &state.variables);
                outputs.insert(key.clone(), interpolated);
            }
        }

        tracing::info!(
            step_id = %step.id,
            input_count = outputs.len(),
            "Start handler merged inputs"
        );

        Ok(StepOutcome::Completed { outputs })
    }
}
