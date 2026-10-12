# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |
| < 0.1   | :x:                |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue, please report it responsibly.

### How to Report

1. **Do NOT** open a public GitHub issue for security vulnerabilities
2. Email us at: security@cogito.ai
3. Include as much information as possible:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Any suggested fixes (optional)

### What to Expect

- We will acknowledge receipt within 48 hours
- We will provide an estimated timeline for a fix
- We will keep you informed of progress
- We will credit you in the security advisory (unless you prefer anonymity)

### Scope

Cogito processes code and context data. Security considerations include:

- **Data Privacy**: We do not store or transmit code without user consent
- **Input Validation**: All inputs are validated before processing
- **Sandboxing**: Future versions will include optional sandboxing for code execution
- **Dependencies**: We keep dependencies updated and audit for vulnerabilities

### Security Best Practices for Users

1. **API Keys**: Never commit API keys to version control
2. **Local Storage**: SQLite/PostgreSQL databases should be properly secured
3. **Network**: Use TLS for API server connections in production
4. **Permissions**: Run the CLI with minimal required permissions

## Security Updates

Security updates are released as patch versions (e.g., 0.1.1, 0.1.2) and announced via:

- GitHub Security Advisories
- Release notes on GitHub Releases
- Project documentation

## Known Limitations

- **No built-in authentication** in REST API (use reverse proxy)
- **No encryption at rest** for local storage (coming in v0.2)
- **No audit logging** (planned for v0.3)

## Third-Party Dependencies

We use `cargo audit` to check for vulnerabilities in dependencies:

```bash
cargo audit
```

This runs automatically in CI/CD on every pull request.
