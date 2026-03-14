use tracing_subscriber::EnvFilter;

/// Initialize the tracing subscriber for structured logging.
///
/// Log level controlled by `RUST_LOG` env var, defaulting to `info`.
/// Output format: compact text to stderr.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .compact()
        .init();
}
