// src/cli.rs
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// FlowState process execution engine.
#[derive(Debug, Parser)]
#[command(name = "flowstate-runner", version, about)]
pub struct Cli {
    /// Path to the project root containing `.flowstate/config.json`.
    #[arg(long, default_value = ".")]
    pub project_root: PathBuf,

    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the scanner once and exit with a report.
    Scan,

    /// Run the resumer once and exit with a report.
    Resume,

    /// Execute a specific ProcessExecution by ID.
    Run {
        /// The execution ID to run (e.g., `exec_abc123`).
        execution_id: String,
    },

    /// Run in daemon mode: scan + resume in a loop.
    Daemon {
        /// Interval in seconds between scan/resume cycles (must be >= 1).
        #[arg(long, default_value = "60", value_parser = clap::value_parser!(u64).range(1..))]
        interval: u64,
    },
}
