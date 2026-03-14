# Contributing to FlowState Runner

Thank you for your interest in contributing to FlowState Runner! This document provides guidelines and information for contributors.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Contributor License Agreement (CLA)](#contributor-license-agreement-cla)
- [Getting Started](#getting-started)
- [How to Contribute](#how-to-contribute)
- [Development Setup](#development-setup)
- [Pull Request Process](#pull-request-process)
- [Coding Standards](#coding-standards)

## Code of Conduct

By participating in this project, you agree to abide by our Code of Conduct. We are committed to providing a welcoming and inclusive environment for all contributors.

## Contributor License Agreement (CLA)

### Why We Require a CLA

Before we can accept your contributions, you must sign our Contributor License Agreement (CLA). This is a standard practice for open source projects and serves several important purposes:

1. **Legal Clarity**: Ensures that Epic Digital Interactive Media LLC has the necessary rights to distribute your contributions under the Apache 2.0 license.
2. **Patent Protection**: Provides patent grants that protect both the project and its users.
3. **Dual Licensing**: Allows us to offer commercial licenses for Epic FlowState Cloud while keeping the core open source.

### How to Sign the CLA

Our automated CLA bot checks every pull request for a signed CLA. Here's what to expect:

1. **Open a pull request** - The CLA bot automatically runs on every PR.
2. **Review the CLA** - If you haven't signed yet, the bot will post a comment with a link to the CLA document.
3. **Sign by commenting** - Post the following comment on your PR:
   > I have read the CLA Document and I hereby sign the CLA
4. **Status check passes** - Once signed, the CLA status check turns green and your PR can proceed to review.

Your signature is stored in the repository and only needs to be done once. All future PRs will pass the CLA check automatically.

**Individual Contributors**: Post the signing comment on your PR to sign the Individual CLA.

**Corporate Contributors**: If you are contributing on behalf of your employer, please contact [cla@epicdigitalmedia.com](mailto:cla@epicdigitalmedia.com) to arrange a Corporate CLA before submitting PRs.

### CLA Terms Summary

By signing the CLA, you agree that:

1. You have the right to submit your contribution.
2. You grant Epic Digital Interactive Media LLC a perpetual, worldwide, non-exclusive, royalty-free license to use, modify, and distribute your contribution.
3. You grant a patent license for any patents you hold that are necessarily infringed by your contribution.
4. Your contribution is provided "as is" without warranty.

## Getting Started

1. **Fork the repository** on GitHub.
2. **Clone your fork** locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/flowstate-runner.git
   cd flowstate-runner
   ```
3. **Build the project**:
   ```bash
   cargo build
   ```
4. **Create a branch** for your changes:
   ```bash
   git checkout -b feature/your-feature-name
   ```

## How to Contribute

### Reporting Bugs

- Check if the bug has already been reported in [Issues](https://github.com/epicdm/flowstate-runner/issues).
- If not, create a new issue with a clear title and description.
- Include steps to reproduce, expected behavior, and actual behavior.
- Add relevant labels.

### Suggesting Features

- Open a new issue with the "enhancement" label.
- Describe the feature and its use case.
- Explain why this feature would benefit the project.

### Code Contributions

1. Ensure your code follows our [coding standards](#coding-standards).
2. Write tests for new functionality.
3. Update documentation as needed.
4. Submit a pull request.

## Development Setup

### Prerequisites

- Rust 1.75+ (MSRV)
- A running FlowState server (for integration tests)
- Optional: `gitleaks` for secret scanning

### Commands

```bash
# Build
cargo build

# Run tests (skip config file test that requires local config)
cargo test -- --skip test_config_loads_from_file

# Lint
cargo clippy -- -D warnings

# Format check
cargo fmt --check

# Format fix
cargo fmt
```

## Pull Request Process

1. **Ensure CLA is signed**: Your PR cannot be merged without a signed CLA.
2. **Run the pre-commit checks**: All four checks must pass (test, build, clippy, fmt).
3. **Update documentation**: If your changes affect user-facing features, update the relevant documentation.
4. **Add tests**: All new features and bug fixes should include tests.
5. **Follow commit conventions**: Use conventional commits (e.g., `feat:`, `fix:`, `docs:`).
6. **Request review**: Tag relevant maintainers for review.

### Commit Message Format

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <subject>

<body>

Built with Epic Flowstate
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`

Examples:

- `feat(executor): add parallel step execution`
- `fix(scanner): resolve trigger condition evaluation`
- `docs(readme): update configuration table`

## Coding Standards

### Rust

- Edition 2021, MSRV 1.75
- `Result<T, E>` for all fallible operations — no `.unwrap()` in library/binary code
- Doc comments (`///`) on all public functions, structs, enums, and methods
- `snake_case` for functions/variables, `PascalCase` for types/traits, `SCREAMING_SNAKE_CASE` for constants
- No `unsafe` unless absolutely necessary with documented safety invariants

### Error Handling

- Use `thiserror` for error types, `anyhow` for application errors
- No `println!` or `dbg!` in non-test code — use `tracing` instead

### Testing

- Write tests for all new functionality
- Use `wiremock` for HTTP mock servers
- Use `tokio::test` for async tests
- Test files: `tests/<module>_test.rs`
- Handler tests: `tests/handlers/<handler>.rs`

### Code Style

- Follow the existing code style
- Run `cargo fmt` and `cargo clippy` before committing
- Prefer iterators and combinators over explicit loops where clearer
- Use `Vec::with_capacity()` when size is known
- Use `Cow<'_, str>` when ownership is conditionally needed

## Questions?

If you have questions about contributing, please:

1. Check existing documentation.
2. Search closed issues for similar questions.
3. Open a new issue with the "question" label.

Thank you for contributing to FlowState Runner!

---

Copyright 2026 Epic Digital Interactive Media LLC. Licensed under Apache 2.0.
