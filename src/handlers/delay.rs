use crate::handlers::{Handler, RunContext};
use crate::models::execution::{ExecutionState, ResolvedStep, StepOutcome};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;

/// Handler for delay steps.
///
/// Currently supports only numeric duration (seconds). ISO 8601 duration
/// parsing and schedule-based delays (cron expressions) are not yet
/// implemented and will be added in a future phase.
pub struct DelayHandler;

#[async_trait]
impl Handler for DelayHandler {
    async fn execute(
        &self,
        step: &ResolvedStep,
        _state: &ExecutionState,
        _ctx: &RunContext,
    ) -> Result<StepOutcome> {
        let action = match &step.action {
            Some(a) => a,
            None => {
                return Ok(StepOutcome::Failed {
                    error: "Delay step missing action configuration".to_string(),
                });
            }
        };

        let duration_secs = match action.get("duration") {
            Some(serde_json::Value::Number(n)) => match n.as_f64() {
                Some(f) => f,
                None => {
                    return Ok(StepOutcome::Failed {
                        error: "Delay duration is not a valid number".to_string(),
                    });
                }
            },
            Some(serde_json::Value::String(s)) => match s.parse::<f64>() {
                Ok(f) => f,
                Err(_) => {
                    return Ok(StepOutcome::Failed {
                        error: format!("Delay duration '{}' is not a valid number", s),
                    });
                }
            },
            _ => {
                return Ok(StepOutcome::Failed {
                    error: "Delay step missing 'duration' field in action".to_string(),
                });
            }
        };

        if duration_secs > 0.0 {
            tracing::info!(
                step_id = %step.id,
                duration_secs = duration_secs,
                "Delay handler sleeping"
            );
            tokio::time::sleep(Duration::from_secs_f64(duration_secs)).await;
        }

        Ok(StepOutcome::Completed {
            outputs: HashMap::new(),
        })
    }
}
