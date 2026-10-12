# Contributing to Cogito

Thank you for your interest in contributing to Cogito!

## Development Setup

### Prerequisites

- Rust 1.75+ (we use 2024 edition)
- Git
- A text editor with Rust support (VS Code + rust-analyzer recommended)

### Quick Start

```bash
# Clone the repository
git clone https://github.com/Rene-Kuhm/cogito.git
cd cogito

# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build the project
cargo build

# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run -- scan
```

### IDE Setup

**VS Code**
1. Install [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
2. Install [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb) for debugging

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
cargo fmt && cargo clippy && cargo test
```

### Formatting Rules

- Use 4 spaces for indentation
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Document all public APIs with doc comments
- Use meaningful variable names

### Documentation

Public APIs must have doc comments:

```rust
/// Compress context while preserving meaning.
///
/// # Arguments
///
/// * `content` - The input context to compress
/// * `options` - Compression configuration
///
/// # Example
///
/// ```
/// use cogito_core::{Cogito, CompressionOptions};
///
/// let cogito = Cogito::new();
/// let result = cogito.compress_context("...", options).await?;
/// ```
pub async fn compress_context(&self, content: &str, options: CompressionOptions) -> Result<CompressedContext> {
    // ...
}
```

## Commit Messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add semantic tag auto-detection
fix: correct token estimation for Unicode
docs: update SPEC.md with new API endpoints
refactor: extract compression strategies into separate module
test: add benchmarks for fingerprint generation
chore: update dependencies
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
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run benchmarks
cargo bench

# Run doc tests
cargo test --doc
```

### Test Coverage

We aim for meaningful tests, not coverage metrics. Tests should:

1. Test behavior, not implementation
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
cargo test
cargo doc --no-deps
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
│   ├── cogito-core/     # Core engine
│   │   └── src/
│   │       ├── lib.rs         # Main entry
│   │       ├── types.rs        # Shared types
│   │       ├── fingerprint/   # Code fingerprinting
│   │       ├── compressor/     # Context compression
│   │       ├── semantic/       # Semantic tags
│   │       └── decision/       # Decision log
│   ├── cogito-cli/      # CLI tool
│   └── cogito-api/      # REST API
├── SPEC.md              # Project specification
└── README.md            # Documentation
```

## Issues

### Bug Reports

Please include:
- Cogito version
- Rust version
- Operating system
- Minimal reproduction case
- Expected vs actual behavior

### Feature Requests

Please include:
- Problem you're trying to solve
- Proposed solution
- Alternatives considered

## License

By contributing, you agree that your contributions will be licensed under the Apache License 2.0.

## Questions?

- Open an issue for bugs/features
- Join discussions in PRs
- Check existing issues before creating new ones
