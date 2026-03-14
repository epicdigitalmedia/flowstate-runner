// src/main.rs
use anyhow::Result;
use clap::Parser;
use flowstate_runner::cli::{Cli, Command};
use flowstate_runner::config::Config;
use flowstate_runner::context::build_run_context;
use flowstate_runner::executor;
use flowstate_runner::handlers::{dispatch_handler, RunContext};
use flowstate_runner::models::execution::{ExecutionState, ProcessExecutionRecord, ResolvedStep};
use flowstate_runner::models::process::{Process, ProcessStep, StepTemplate};
use flowstate_runner::resumer;
use flowstate_runner::scanner;
use flowstate_runner::state::compute_plan_dir;
use flowstate_runner::template::resolve_template;
use std::collections::HashMap;
use std::time::Instant;

#[tokio::main]
async fn main() {
    flowstate_runner::logging::init();
    let start_time = Instant::now();
    let cli = Cli::parse();

    // Load config
    let config = match Config::load(&cli.project_root) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Failed to load configuration");
            std::process::exit(1);
        }
    };
    tracing::info!(
        org_id = %config.org_id,
        workspace_id = %config.workspace_id,
        "Configuration loaded"
    );

    // Build context
    let (ctx, templates) = match build_run_context(config).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(error = %e, "Failed to initialize");
            std::process::exit(1);
        }
    };

    // Spawn health server in background
    if let Err(e) =
        flowstate_runner::health::spawn_health_server(ctx.config.health_port, start_time).await
    {
        tracing::error!(error = %e, "Failed to start health server");
        std::process::exit(1);
    }

    match cli.command {
        Command::Scan => match scanner::scan(&ctx).await {
            Ok(report) => {
                tracing::info!(
                    created = report.created.len(),
                    skipped = report.skipped,
                    errors = report.errors.len(),
                    "Scan report"
                );
                if !report.errors.is_empty() {
                    std::process::exit(1);
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Scan failed");
                std::process::exit(1);
            }
        },
        Command::Resume => match resumer::resume(&ctx, &templates).await {
            Ok(report) => {
                tracing::info!(
                    resumed = report.resumed.len(),
                    still_waiting = report.still_waiting,
                    errors = report.errors.len(),
                    "Resume report"
                );
                if !report.errors.is_empty() {
                    std::process::exit(1);
                }
            }
            Err(e) => {
                tracing::error!(error = format!("{:#}", e), "Resume failed");
                std::process::exit(1);
            }
        },
        Command::Run { execution_id } => {
            match run_execution(&execution_id, &ctx, &templates).await {
                Ok(0) => {}
                Ok(code) => std::process::exit(code),
                Err(e) => {
                    tracing::error!(error = format!("{:#}", e), "Execution failed");
                    std::process::exit(1);
                }
            }
        }
        Command::Daemon { interval } => {
            tracing::info!(interval_secs = interval, "Starting daemon mode");

            let mut sigterm = tokio::signal::unix::signal(
                tokio::signal::unix::SignalKind::terminate(),
            )
            .expect("Failed to register SIGTERM handler");

            loop {
                // Refresh JWT before each cycle. The exchanger caches the
                // token internally and only hits the auth server when the
                // current JWT is close to expiry.
                if let Err(e) = ctx.refresh_auth_if_needed().await {
                    tracing::warn!(error = %e, "JWT refresh failed, continuing with current token");
                }

                match scanner::scan(&ctx).await {
                    Ok(report) => {
                        tracing::info!(
                            created = report.created.len(),
                            skipped = report.skipped,
                            errors = report.errors.len(),
                            "Scan cycle complete"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Scan cycle failed");
                    }
                }

                match resumer::resume(&ctx, &templates).await {
                    Ok(report) => {
                        tracing::info!(
                            resumed = report.resumed.len(),
                            still_waiting = report.still_waiting,
                            errors = report.errors.len(),
                            "Resume cycle complete"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Resume cycle failed");
                    }
                }

                // Evict expired cache entries to prevent unbounded memory growth
                ctx.evict_caches();

                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(interval)) => {}
                    _ = sigterm.recv() => {
                        tracing::info!("Received SIGTERM, shutting down");
                        break;
                    }
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("Received SIGINT, shutting down");
                        break;
                    }
                }
            }
        }
    }
}

/// Load and execute a specific ProcessExecution.
///
/// Fetches the execution record, its process, and all steps from REST.
/// Resolves templates and runs the executor loop with persistence enabled.
///
/// Returns the desired exit code: 0 for success/paused, 2 for failed.
/// The caller handles `Err` separately (exit code 1).
async fn run_execution(
    execution_id: &str,
    ctx: &RunContext,
    templates: &HashMap<String, StepTemplate>,
) -> Result<i32> {
    // Load execution record (virtual collection — goes through MCP)
    let record: ProcessExecutionRecord = ctx.get("processexecutions", execution_id).await?;

    // Load process (virtual collection — goes through MCP)
    let process: Process = ctx.get("processes", &record.process_id).await?;

    // Load steps (virtual collection — goes through MCP)
    let raw_steps: Vec<ProcessStep> = ctx
        .query(
            "processsteps",
            serde_json::json!({
                "processId": process.id,
                "orgId": ctx.config.org_id,
                "workspaceId": ctx.config.workspace_id,
            }),
        )
        .await?;

    // Resolve templates
    let steps: HashMap<String, ResolvedStep> = raw_steps
        .iter()
        .map(|s| {
            let tmpl = s.template_id.as_ref().and_then(|tid| templates.get(tid));
            let resolved = resolve_template(s, tmpl);
            (s.id.clone(), resolved)
        })
        .collect();

    // Build execution state
    let mut state = ExecutionState::from_record(record, process.name.clone());
    state.plan_dir = compute_plan_dir(
        ctx.config.plan_base_dir.to_str().unwrap_or("."),
        state.external_id.as_deref(),
    );

    // Expose planDir as a variable for template resolution in step actions
    if let Some(ref dir) = state.plan_dir {
        state
            .variables
            .insert("planDir".to_string(), serde_json::Value::String(dir.clone()));
    }

    tracing::info!(
        execution_id,
        process = %process.name,
        step_count = steps.len(),
        "Starting execution"
    );

    // Execute
    executor::execute(&mut state, &steps, &dispatch_handler, ctx, true).await?;

    // Return exit code based on final status
    match state.status.as_str() {
        "completed" => {
            tracing::info!(execution_id, "Execution completed successfully");
            Ok(0)
        }
        "paused" => {
            tracing::info!(execution_id, "Execution paused");
            Ok(0)
        }
        "failed" => {
            tracing::error!(execution_id, "Execution failed");
            Ok(2)
        }
        _ => {
            tracing::warn!(execution_id, status = %state.status, "Unexpected final status");
            Ok(0)
        }
    }
}
