use crate::conditions::evaluate_condition;
use crate::handlers::{Handler, RunContext};
use crate::models::execution::{ExecutionState, ResolvedStep, StepOutcome};
use crate::models::trigger::Op;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

pub struct DecisionHandler;

#[async_trait]
impl Handler for DecisionHandler {
    async fn execute(
        &self,
        step: &ResolvedStep,
        state: &ExecutionState,
        _ctx: &RunContext,
    ) -> Result<StepOutcome> {
        let vars_value = Value::Object(state.variables.clone());
        let mut outputs = HashMap::new();

        for condition_val in &step.conditions {
            // Accept both "field" (Rust convention) and "propertyPath" (bash runner convention)
            let field = match condition_val
                .get("field")
                .or_else(|| condition_val.get("propertyPath"))
                .and_then(Value::as_str)
            {
                Some(f) => f,
                None => continue,
            };
            let op_str = match condition_val.get("operator").and_then(Value::as_str) {
                Some(o) => o,
                None => continue,
            };
            let op = match Op::parse(op_str) {
                Ok(op) => op,
                Err(_) => {
                    tracing::warn!(
                        operator = op_str,
                        "Unknown operator in decision condition, skipping"
                    );
                    continue;
                }
            };
            let expected = condition_val.get("value").unwrap_or(&Value::Null);
            let value_from = condition_val.get("valueFrom").and_then(Value::as_str);
            let target_step_id = match condition_val.get("targetStepId").and_then(Value::as_str) {
                Some(t) => t,
                None => continue,
            };

            let matched = evaluate_condition(
                &vars_value,
                field,
                &op,
                expected,
                value_from,
                Some(&state.variables),
            );

            if matched {
                tracing::info!(
                    step_id = %step.id,
                    field = field,
                    operator = op_str,
                    target = target_step_id,
                    "Decision condition matched"
                );
                outputs.insert(
                    "_next_step_override".to_string(),
                    Value::String(target_step_id.to_string()),
                );
                break;
            }
        }

        if !outputs.contains_key("_next_step_override") {
            tracing::info!(
                step_id = %step.id,
                "No decision condition matched, using default path"
            );
        }

        Ok(StepOutcome::Completed { outputs })
    }
}
