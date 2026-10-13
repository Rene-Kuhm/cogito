# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |
| < 0.1   | :x:                |

## Reporting a Vulnerability

If you find a security issue, please report it **privately**. Do not open a
public issue, pull request, or discussion for it.

### How to report

Use GitHub's private vulnerability reporting on this repository:

**https://github.com/Rene-Kuhm/cogito-harness/security/advisories/new**

That form is private between you and the maintainer, and it gives us a place to
collaborate on a fix and publish an advisory that credits you.

If you cannot use the form, open a public issue that says only that you have a
security report and asks for a private channel. Do not include details in it.

Please include:

- A description of the issue and its impact
- Steps to reproduce, or a proof of concept
- Affected versions
- Any suggested fix (optional)

### What to expect

- Acknowledgement within 48 hours
- An initial assessment and a timeline for a fix
- Updates as the fix progresses
- Credit in the published advisory, unless you prefer to stay anonymous

### Scope

Things worth reporting:

- Command or argument injection through a project path, tag, or decision
- Path traversal when reading or writing `.cogito/`
- SQLite injection or unintended data disclosure through the knowledge store
- Anything that makes `cogito setup` modify a file it should not touch
- Privilege or sandbox escapes in the MCP server

Things that are usually not vulnerabilities here:

- **A malicious repository.** Cogito reads project files and git history by
  design, and the MCP server is intended to run with your permissions. Only run
  an agent against code you would run yourself.
- **Agent permissions.** What an agent may do with the tools Cogito exposes is
  the agent's policy, not Cogito's.
- **Data written under `.cogito/`.** It contains file paths, module names and
  whatever you choose to record as decisions. Do not record secrets in it.

### Security model

Cogito has no network listener by default. `cogito mcp` speaks JSON-RPC over
stdio with the agent that started it. `cogito-api` binds a local port and has no
authentication of its own: if you expose it beyond localhost, put it behind a
reverse proxy that authenticates.

The knowledge store is a local SQLite file at `<project>/.cogito/state.db`. It
is not encrypted at rest.

## Security Updates

Fixes are released as patch versions and announced through GitHub Security
Advisories and the release notes.
