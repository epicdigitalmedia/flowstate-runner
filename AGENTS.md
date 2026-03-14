# FlowState Runner — Agent Guidelines

## Language & Tooling

- **Language:** Rust (edition 2021, MSRV 1.75)
- **Async runtime:** tokio (full features)
- **Error handling:** `thiserror` for error types, `anyhow` for application errors
- **Logging:** `tracing` crate — no `println!` or `dbg!` in non-test code
- **HTTP:** `reqwest` for all HTTP clients
- **CLI:** `clap` (derive API)
- **Serialization:** `serde` + `serde_json`

## Required Practices

- `Result<T, E>` for all fallible operations — no `.unwrap()` in library/binary code
- Doc comments (`///`) on all public functions, structs, enums, and methods
- snake_case for functions/variables, PascalCase for types/traits, SCREAMING_SNAKE_CASE for constants
- Derive `Debug`, `Clone`, `PartialEq` where appropriate
- Prefer borrowing (`&T`, `&mut T`) over ownership when possible
- Use iterators and combinators over explicit loops where clearer
- `Vec::with_capacity()` when size is known
- `Cow<'_, str>` when ownership is conditionally needed
- Exhaustive pattern matching — avoid catch-all `_` when possible
- No `unsafe` unless absolutely necessary with documented safety invariants

## Pre-Commit Checklist

```bash
cargo test -- --skip test_config_loads_from_file
cargo build
cargo clippy -- -D warnings
cargo fmt --check
```

All four must pass before committing.

Additionally, verify:

- All public functions, structs, enums, and methods have `///` doc comments
- No `.unwrap()` in non-test code (allowed in tests, doc examples, and the `value_to_string` JSON fallback)
- No `dbg!` or `println!` statements in non-test code — use `tracing` instead

## Commit Format

```
type(scope): description

Built with Epic Flowstate
```

Use `--no-gpg-sign` if 1Password signing is unavailable.

## Testing

- Use `wiremock` for HTTP mock servers in tests
- Use `tokio::test` for async tests
- Test files: `tests/<module>_test.rs`
- Handler tests: `tests/handlers/<handler>.rs`
- Integration tests: `tests/integration_test.rs`
- Skip flaky test: `-- --skip test_config_loads_from_file`

## Concurrency

- **Async runtime:** tokio with `#[tokio::main]` and `full` features
- All I/O is async — REST calls, MCP calls, file reads use `await`
- The daemon loop uses `tokio::select!` for signal-aware sleeping (SIGTERM/SIGINT)
- Health server runs in a background `tokio::spawn` task on its own port
- No shared mutable state between async tasks — each execution owns its `ExecutionState`
- Use `tokio::test` for async test functions

## Type System

- **Serde patterns:** `#[serde(rename_all = "camelCase")]` on all model structs for JS/DB interop
- `#[serde(default)]` on optional fields so deserialization succeeds with missing keys
- `serde_json::Map<String, Value>` for open-ended key-value maps (variables, metadata)
- `Option<T>` for truly optional fields; avoid sentinel values
- Status fields use string constants (`STATUS_RUNNING`, etc.) for DB compatibility — the `ExecutionStatus` enum is scaffolded for a future migration
- `StepOutcome` enum (`Completed`, `Paused`, `Failed`) is the handler return type — exhaustive matching ensures all outcomes are handled

## Security

- **Non-root container:** Dockerfile creates a `runner` system user; `USER runner` is set before ENTRYPOINT
- **Auth is opt-in:** `FLOWSTATE_AUTH_TOKEN` env var enables Bearer auth on REST/MCP calls. Not required on Docker internal networks where services trust each other
- **No secrets in code:** Config loaded from `.flowstate/config.json` + environment variables. Real secrets go through 1Password via `flowstate-env`
- **No `.unwrap()` in non-test code:** All fallible operations return `Result<T, E>`

## Secrets

Follow FlowState pattern: `std::env` for non-sensitive config, 1Password via `flowstate-env` for secrets. Never use `.env` files for real secrets.

## Architecture

```
src/
├── main.rs          # CLI entrypoint (clap)
├── cli.rs           # Argument parsing
├── config.rs        # Config from .flowstate/config.json + env
├── context.rs       # RunContext initialization
├── health.rs        # Axum health server (/health)
├── logging.rs       # Tracing subscriber setup
├── executor.rs      # Core execution loop
├── scanner.rs       # Entity trigger scanning
├── resumer.rs       # Paused execution resumption
├── template.rs      # Step template resolution + interpolation
├── output.rs        # Output mapping + extraction
├── state.rs         # Plan directory computation
├── conditions.rs    # Condition evaluation engine
├── error.rs         # Error types
├── agent/           # Agent executor implementations
├── clients/         # HTTP clients (REST, MCP, obs)
├── handlers/        # Step type handlers
└── models/          # Data models (process, execution, etc.)
```
