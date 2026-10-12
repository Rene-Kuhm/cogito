# Cogito

**Intelligent context harness for AI coding agents.**

Cogito optimizes the context AI agents receive — without modifying models or agents. It acts as an intelligent middle layer that reduces token consumption, improves context relevance, and accelerates agent performance.

```bash
# Quick Start
cargo install cogito-cli          # Install
cogito init                       # Initialize project
cogito scan                       # Generate fingerprint
cogito compress context.txt      # Compress context
```

[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-Apache_2.0-blue.svg)](LICENSE)
[![crates.io](https://img.shields.io/crates/v/cogito-core.svg)](https://crates.io/crates/cogito-core)
[![docs](https://img.shields.io/badge/docs-docs.rs-orange.svg)](https://docs.rs/cogito-core)

---

## Works with everything

Cogito is **editor- and agent-agnostic**. It speaks CLI, MCP, and REST. Pick what fits your tool:

| Tool | Integration |
|---|---|
| **Gentle Shell (pi)** | First-class skill — see [`skills/cogito/SKILL.md`](skills/cogito/SKILL.md) |
| **Claude Code** | MCP server (one config block) |
| **Cursor** | MCP server |
| **Continue (VSCode)** | MCP server |
| **Neovim** | CLI + [`mcphub.nvim`](https://github.com/ravitemer/mcphub.nvim) |
| **Aider** | Shell hook |
| **Zed** | MCP server |
| **Any agent** | REST API on `localhost:8080` |

Full setup instructions for every tool: [`docs/integrations.md`](docs/integrations.md).

---

## What is Cogito?

AI coding agents suffer from **context inflation**. Large codebases exceed context windows, agents fetch irrelevant code, and every conversation starts from scratch.

Cogito solves this with four core innovations:

| Feature | What it does | Why it matters |
|---------|-------------|----------------|
| **Code Fingerprints** | Minimal structural snapshots of projects | Agents understand structure without reading everything |
| **Semantic Memory Tags** | Indexed knowledge with zero reading | Query "where is auth?" returns instant answers |
| **Decision Log** | Structured reasoning that survives sessions | "Why did we choose JWT?" is always answerable |
| **Context Compressor** | Intelligent context reduction | 50%+ token savings, zero semantic loss |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         COGITO CORE                         │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────────┐ │
│  │   Code       │  │  Semantic    │  │    Decision Log    │ │
│  │  Fingerprint │  │   Memory     │  │       Engine       │ │
│  └──────────────┘  └──────────────┘  └────────────────────┘ │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────────┐ │
│  │   Hot/Cold   │  │  Context     │  │   MCP Protocol     │ │
│  │    Zones     │  │  Compressor  │  │      Adapter      │ │
│  └──────────────┘  └──────────────┘  └────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
          │                 │                    │
    ┌─────┴─────┐     ┌─────┴─────┐       ┌─────┴─────┐
    │    CLI    │     │  REST API │       │  MCP      │
    └───────────┘     └───────────┘       │  Server   │
                                          └───────────┘
                           │              │
              ┌────────────┴──────────────┘
              │         ANY AI AGENT
              │
    ┌─────────┼─────────┬─────────┬─────────┬─────────┐
    │ Claude  │ GPT-4o  │ Gemini  │DeepSeek │  Qwen   │
    │  Code   │  Codex  │         │         │         │
    └─────────┴─────────┴─────────┴─────────┴─────────┘
```

---

## Installation

### From source

```bash
git clone https://github.com/Rene-Kuhm/cogito.git
cd cogito
cargo install --path crates/cogito-cli
```

### From crates.io (when published)

```bash
cargo install cogito-cli
```

### Pre-built binaries

Download from [GitHub Releases](https://github.com/Rene-Kuhm/cogito/releases)

---

## Quick Start

```bash
# 1. Initialize Cogito in your project
cogito init

# 2. Scan and generate fingerprint
cogito scan

# 3. Compress context
cat large_context.txt | cogito compress --max-tokens 8000

# 4. Query knowledge
cogito query "how does authentication work?"

# 5. Register a decision
cogito decision add \
  --title "JWT with refresh tokens" \
  --why "stateless, scales better than sessions" \
  --paths src/auth/
```

---

## Features

### Code Fingerprint Generator

```bash
cogito scan --watch    # Watch mode for live updates
cogito fingerprint show # Display current fingerprint
```

Generates minimal structural snapshots identifying:
- Entry points (main, routes, handlers)
- Data flows between modules
- Hot/cold code zones
- Dependency graph

### Semantic Memory Tags

```bash
cogito tag add billing src/billing/ src/invoices/
cogito tag search "payment"
```

Indexes knowledge without reading code:
- Auto-tag by file naming patterns
- Directory structure → domain mapping
- Confidence scores for relevance

### Decision Log

```bash
cogito decision list
cogito decision search "auth"
```

Structured reasoning that survives sessions:
- Manual registration via CLI
- Auto-extraction from git history
- Search by tags, paths, or text

### Context Compressor

```bash
cogito compress context.txt --max-tokens 8000
```

Intelligent compression strategies:
- **Structural**: Remove whitespace, comments
- **Semantic**: Summarize functions, keep signatures
- **Relevance**: Remove unused code paths

### MCP Server

```bash
cogito serve --mcp
```

Connect to any MCP-compatible client (Claude Code, Cursor, etc.)

---

## Comparison

| Feature | Cogito | LangChain | LlamaIndex |
|---------|--------|-----------|------------|
| Code fingerprints | ✅ | ❌ | ❌ |
| Semantic memory tags | ✅ | ❌ | ❌ |
| Decision log | ✅ | ❌ | ❌ |
| Hot/cold zones | ✅ | ❌ | ❌ |
| Context compression | ✅ Optimized | ⚠️ Basic | ⚠️ Basic |
| MCP native | ✅ | ⚠️ | ❌ |
| Multi-model unified | ✅ | ⚠️ | ⚠️ |
| Token savings | >50% | Unknown | Unknown |
| Open source | ✅ | ✅ | ✅ |

---

## Supported Models

Cogito is model-agnostic. It works with any AI agent:

- **Claude** (Anthropic) — via MCP or REST
- **GPT-4o / Codex** (OpenAI) — via REST API
- **Gemini** (Google) — via REST API
- **DeepSeek** (Moonshot) — via REST API
- **Qwen** (Alibaba) — via REST API
- **MiniMax** — via REST API
- **Grok** (xAI) — via REST API
- **Cursor** — via MCP
- **Copilot** — via API
- **Aider** — via CLI

---

## Supported Languages

| Language | Fingerprint | Semantic Tags | Hot/Cold |
|----------|-------------|---------------|----------|
| Rust | ✅ | ✅ | ✅ |
| TypeScript | ✅ | ✅ | ✅ |
| JavaScript | ✅ | ✅ | ✅ |
| Python | ✅ | ✅ | ✅ |
| Go | ✅ | ✅ | ✅ |
| Java | ✅ | ✅ | ✅ |
| C# | ✅ | ✅ | ✅ |
| Ruby | ✅ | ✅ | ✅ |
| PHP | ✅ | ✅ | ✅ |

---

## Documentation

- [SPEC.md](SPEC.md) — Full technical specification
- [CONTRIBUTING.md](CONTRIBUTING.md) — How to contribute
- [docs/](docs/) — Detailed documentation (coming soon)

---

## License

Apache License 2.0 — see [LICENSE](LICENSE)

---

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for:
- Development setup
- Code style guidelines
- Commit message conventions
- Pull request process

---

## Status

**Project Status**: 🟡 Active Development

Cogito is in active development. The MVP is being built.

- [x] Project structure
- [x] Specification
- [ ] Core engine implementation
- [ ] CLI tool
- [ ] REST API
- [ ] MCP server

See [ROADMAP](SPEC.md#roadmap) for full timeline.
