/// Typed error categories for the flowstate-runner.
///
/// NOTE: Currently scaffolded for Phase 3+. All modules currently use `anyhow::Result`.
/// Phase 3 will add `#[from]` conversions (e.g., `Io(#[from] std::io::Error)`) and
/// migrate the executor's return type from `anyhow::Result` to `Result<(), RunnerError>`.
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("Config error: {0}")]
    Config(String),

    #[error("REST client error: {0}")]
    Rest(String),

    #[error("Template error: {0}")]
    Template(String),

    #[error("Condition error: {0}")]
    Condition(String),

    #[error("Output error: {0}")]
    Output(String),

    #[error("Handler error: {0}")]
    Handler(String),

    #[error("Executor error: {0}")]
    Executor(String),

    #[error("State error: {0}")]
    State(String),

    #[error("MCP client error: {0}")]
    Mcp(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Agent execution error: {0}")]
    Agent(String),

    #[error("Subprocess error: {0}")]
    Subprocess(String),

    #[error("I/O error: {0}")]
    Io(String),
}
