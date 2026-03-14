# Security Policy

FlowState Runner takes the security of our software products and services seriously. If you believe you have found a security vulnerability, we encourage you to report it to us responsibly.

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| latest  | :white_check_mark: |

As an actively developed project, we provide security patches for the latest release on the `main` branch.

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please use one of the following methods:

### 1. GitHub Security Advisories (Preferred)

Use [GitHub Security Advisories](https://github.com/epicdm/flowstate-runner/security/advisories/new) to privately report a vulnerability. This allows us to collaborate on a fix before public disclosure.

### 2. Email

Send an email to [security@epicdm.com](mailto:security@epicdm.com) with the following information:

- A description of the vulnerability
- Steps to reproduce the issue
- The potential impact
- Any suggested fixes (if applicable)

### What to Include

When reporting a vulnerability, please provide:

- **Type of vulnerability** (e.g., command injection, authentication bypass, privilege escalation)
- **Affected component** (module name, file path)
- **Steps to reproduce** with enough detail to confirm the issue
- **Impact assessment** of what an attacker could achieve
- **Environment details** (OS, Rust version, etc.)

## Response Timeline

| Action             | Timeframe                              |
| ------------------ | -------------------------------------- |
| Acknowledgment     | Within 2 business days                 |
| Initial assessment | Within 5 business days                 |
| Status update      | Every 7 business days until resolution |
| Fix release        | Depends on severity (see below)        |

## Severity Classification and Response

| Severity     | Description                                               | Target Fix Time |
| ------------ | --------------------------------------------------------- | --------------- |
| **Critical** | Remote code execution, authentication bypass, data breach | 24-48 hours     |
| **High**     | Privilege escalation, significant data exposure           | 7 days          |
| **Medium**   | Limited data exposure, denial of service                  | 30 days         |
| **Low**      | Minor information disclosure, best practice violations    | 90 days         |

Severity is assessed using [CVSS v3.1](https://www.first.org/cvss/calculator/3.1) scoring.

## Security Update Process

When a vulnerability is confirmed:

1. **Triage** - The security team assesses severity and impact
2. **Fix Development** - A patch is developed on a private branch
3. **Review** - The fix undergoes security-focused code review
4. **Release** - The fix is released as a patch version
5. **Advisory** - A GitHub Security Advisory is published with:
   - CVE identifier (if applicable)
   - Affected versions
   - Fixed versions
   - Mitigation steps
6. **Disclosure** - Public disclosure occurs after users have had time to update

## Scope

The following are in scope for security reports:

- All code in this repository
- Docker container configurations
- Authentication and authorization mechanisms (token handling, JWT exchange)
- Command execution and subprocess spawning
- API client interactions (REST, MCP)
- Agent execution (Claude CLI, Anthropic API)

### Out of Scope

- Vulnerabilities in third-party services we integrate with (report to those vendors directly)
- Social engineering attacks
- Denial of service attacks against production infrastructure
- Issues already reported in public GitHub issues

## Contact

- **Security Reports**: [security@epicdm.com](mailto:security@epicdm.com)
- **General Issues**: [GitHub Issues](https://github.com/epicdm/flowstate-runner/issues)
