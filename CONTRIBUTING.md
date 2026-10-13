# Contributing to Cogito

Thank you for your interest in contributing to Cogito!

## Development Setup

### Prerequisites

- Rust 1.75+ (2024 edition)
- Git
- A text editor with Rust support (VS Code + rust-analyzer recommended)

### Quick Start

```bash
# Clone the repository
git clone https://github.com/Rene-Kuhm/cogito-harness.git
cd cogito

# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build the project
cargo build

# Run tests
cargo test --all

# Run with logging
RUST_LOG=debug cargo run -- scan
```

### IDE Setup

**VS Code**
1. Install [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
2. Install [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vsdbg-lldb) for debugging

**Neovim**
```vim
:Plug 'rust-lang/rust.vim'
:Plug 'neovim/nvim-lspconfig'
:Plug 'simrat39/rust-tools.nvim'
```

## Code Style

We follow standard Rust conventions:

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt --check

# Lint
cargo clippy

# Run all checks before committing
cargo fmt && cargo clippy && cargo test --all
```

### Formatting Rules

- Use 4 spaces for indentation
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Document all public APIs with doc comments
- Use meaningful variable names

### Documentation

Public APIs must have doc comments. Example from the public `Cogito` API:

```rust
/// Open a Cogito instance bound to a project directory.
///
/// Opens (creating if needed) `<project>/.cogito/state.db`. This is the
/// correct constructor when the path is supplied by a caller — for
/// example an MCP tool argument — rather than inherited from the process
/// working directory.
pub fn for_project(project: &std::path::Path) -> Result<Self> {
    Self::with_project_sqlite(&project.join(".cogito"))
}
```

## Commit Messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add Kotlin symbol extraction to tree-sitter parser
fix: correct import resolution for Go multi-import statements
docs: update benchmarks.md with ripgrep results
refactor: extract FTS5 query construction into index module
test: add import-resolution tests for Python and Java
chore: update tree-sitter dependencies
```

### Types

- `feat` - New feature
- `fix` - Bug fix
- `docs` - Documentation only
- `style` - Formatting, missing semicolons, etc.
- `refactor` - Code change that neither fixes a bug nor adds a feature
- `test` - Adding tests
- `chore` - Maintenance tasks

## Testing

```bash
# Run all tests
cargo test --all

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run doc tests
cargo test --doc
```

### Test Coverage

We aim for meaningful tests, not coverage metrics. Tests should:

1. Test behaviour, not implementation
2. Cover edge cases
3. Be independent and idempotent

## Pull Request Process

### 1. Fork and Branch

```bash
git checkout -b feat/my-new-feature
```

### 2. Make Changes

- Follow the code style guidelines
- Add tests for new functionality
- Update documentation as needed
- Keep commits atomic and meaningful

### 3. Test Your Changes

```bash
cargo fmt
cargo clippy
cargo test --all
```

### 4. Submit PR

- Use a clear title following Conventional Commits
- Describe *what* changed and *why*
- Link any related issues
- Request review from maintainers

### 5. Review Process

- Maintainers will review your PR
- Address any feedback
- Once approved, a maintainer will merge

## Project Structure

```
cogito/
├── crates/
│   ├── cogito-harness-core/        # Core engine library
│   │   └── src/
│   │       ├── lib.rs             # Public API (Cogito struct)
│   │       ├── types.rs           # Shared types (Language, Project, Fingerprint, Symbol…)
│   │       ├── fingerprint/       # Code fingerprint generation
│   │       ├── index/             # SQLite/FTS5 symbol and file index
│   │       ├── map/               # Map rendering, outline, symbols, import layout
│   │       └── storage/           # SQLite connection, schema, migrations
│   └── cogito-harness/            # CLI binary
│       └── src/
│           ├── main.rs            # Entrypoint
│           ├── mcp.rs             # MCP server (4 tools over stdio)
│           ├── agents.rs          # Agent registry and config shapes
│           ├── claude_hook.rs     # PreToolUse guardrail decision logic
│           └── commands/          # CLI subcommands (mod.rs, bench.rs)
├── docs/
│   ├── integrations.md            # Per-agent setup and behaviour
│   ├── benchmarks.md              # Token savings benchmark details
│   └── publishing.md              # Release process (crates.io, npm, Homebrew)
├── examples/                      # Standalone usage examples
├── packages/cogito-harness/       # npm package for Gentle Shell
├── skills/cogito/                 # Pi skill
├── SPEC.md                        # Product specification
└── README.md                      # Top-level documentation
```

## Issues

### Bug Reports

Please include:
- `cogito --version` or git commit
- Rust version
- Operating system
- Minimal reproduction case
- Expected vs actual behaviour

### Feature Requests

Please include:
- Problem you are trying to solve
- Proposed solution
- Alternatives considered

## License

By contributing, you agree that your contributions will be licensed under the Apache License 2.0.

## Questions?

- Open an issue for bugs/features
- Join discussions in PRs
- Check existing issues before creating new ones
