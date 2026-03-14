# FlowState Runner

A Rust-based process execution engine that reads FlowState workflow definitions and executes their steps as local commands on a Linux machine. It orchestrates multi-step workflows with support for conditional branching, human approvals, agent-driven automation, and subprocess nesting.

## Features

- **Sequential step execution** — Runs process steps in order, persisting state after each step
- **11 step types** — start, end, action (REST/script), decision, delay, notification, approval, human task, agent task, subprocess, and parallel gateways
- **Entity-triggered scanning** — Polls for entity changes that match process triggers and creates executions automatically
- **Pause and resume** — Supports approval workflows, human tasks, and subprocess waits with automatic resumption
- **Agent execution** — Runs Claude via CLI or Anthropic API for AI-driven steps
- **Variable interpolation** — Template engine with `${varName}` substitution across all step inputs
- **Condition evaluation** — Rich operator set including equality, comparison, regex, date, and list membership checks
- **Daemon mode** — Continuous scan-resume loop with configurable interval, graceful shutdown, and JWT refresh
- **Health endpoint** — Axum-based `/health` route for container orchestration
- **TTL caching** — In-memory caches for processes and steps to reduce database queries

## Architecture

```
CLI (scan | resume | run <id> | daemon)
        │
        ├── Scanner ──► Finds triggered entities, creates executions
        ├── Resumer ──► Checks paused executions for resumability
        └── Executor ─► Steps through execution sequentially
                │
                ├── StartHandler        (initialize variables)
                ├── ActionHandler       (REST calls, scripts, subprocesses)
                ├── DecisionHandler     (conditional routing)
                ├── DelayHandler        (timed waits)
                ├── NotificationHandler (email, Slack, etc.)
                ├── ApprovalHandler     (pause for approval)
                ├── HumanTaskHandler    (pause for human input)
                ├── AgentTaskHandler    (Claude CLI / Anthropic API)
                ├── SubprocessHandler   (nested workflow execution)
                └── EndHandler          (finalize and complete)
```

### Clients

| Client | Purpose |
|--------|---------|
| **REST** | CRUD operations on native FlowState collections |
| **MCP** | Queries virtual collections (processes, steps, executions, schemas) |
| **OBS** | Posts execution metrics and telemetry |

## Prerequisites

- Rust 1.75+ (MSRV)
- A running FlowState server (REST + MCP endpoints)
- Optional: `claude` CLI or `ANTHROPIC_API_KEY` for agent task steps
- Optional: `gitleaks` for secret scanning in pre-commit hooks

## Configuration

Configuration is loaded from `.flowstate/config.json` in the project root, with environment variable overrides:

| Variable | Default | Description |
|----------|---------|-------------|
| `FLOWSTATE_REST_URL` | `http://localhost:7080` | FlowState REST API base URL |
| `FLOWSTATE_MCP_URL` | `http://localhost:7080/mcp` | FlowState MCP server URL |
| `FLOWSTATE_AUTH_TOKEN` | — | Static Bearer token (legacy auth) |
| `FLOWSTATE_API_TOKEN` | — | API token for JWT exchange |
| `FLOWSTATE_AUTH_URL` | — | Auth endpoint for token exchange |
| `OBS_SERVER_URL` | — | Observability/metrics endpoint |
| `HEALTH_PORT` | `9090` | Health check HTTP port |
| `MAX_SUBPROCESS_DEPTH` | `5` | Max nesting depth for subprocesses |
| `AGENT_EXECUTOR` | `claude-cli` | Agent backend (`claude-cli` or `anthropic-api`) |
| `WORKER_MODE` | `false` | Enable worker mode |

## Usage

```bash
# Build
cargo build --release

# Run a specific execution by ID
flowstate-runner run <execution_id>

# Scan for triggered entities once
flowstate-runner scan

# Check and resume paused executions once
flowstate-runner resume

# Run as a daemon (scan + resume loop)
flowstate-runner daemon --interval 60

# Specify a custom project root
flowstate-runner --project-root /path/to/project daemon
```

## Docker

```bash
# Build the image
docker build -t flowstate-runner .

# Run as daemon
docker run -d \
  -e FLOWSTATE_REST_URL=http://kong:8000 \
  -e FLOWSTATE_MCP_URL=http://kong:8000/mcp \
  -e FLOWSTATE_API_TOKEN=your-token \
  -e FLOWSTATE_AUTH_URL=http://auth:8080/exchange \
  -v $(pwd)/.flowstate/config.json:/app/.flowstate/config.json:ro \
  -p 9090:9090 \
  flowstate-runner
```

The container runs as a non-root user (`runner`), exposes the health check on port 9090, and defaults to `daemon --interval 60`.

## Development

### Setup

```bash
# Clone the repository
git clone https://github.com/epicdm/flowstate-runner.git
cd flowstate-runner

# Enable the pre-commit hook
git config core.hooksPath .githooks
```

### Commands

```bash
# Run tests (skip config file test that requires local config)
cargo test -- --skip test_config_loads_from_file

# Build
cargo build

# Lint
cargo clippy -- -D warnings

# Format check
cargo fmt --check

# Format fix
cargo fmt
```

### Pre-Commit Hook

The repository includes a pre-commit hook at `.githooks/pre-commit` that runs:

1. **gitleaks** — Scans staged files for secrets/credentials (warns if not installed)
2. **cargo fmt --check** — Verifies code formatting
3. **cargo clippy** — Runs the Rust linter with warnings as errors
4. **cargo test** — Runs the test suite

Enable it with:

```bash
git config core.hooksPath .githooks
```

### Commit Convention

```
type(scope): description

Built with Epic Flowstate
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) for details on our code of conduct, the CLA process, and how to submit pull requests.

## Security

For information about reporting security vulnerabilities, see [SECURITY.md](SECURITY.md).

## License

Copyright 2026 Epic Digital Interactive Media LLC

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for the full license text.

## Trademarks

Epic FlowState™ and FlowState™ are trademarks of Epic Digital Interactive Media LLC. See [NOTICE](NOTICE) for trademark usage guidelines.
