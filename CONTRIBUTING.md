# Contributing to novel2audiobook

Thank you for your interest in contributing!

## Development Setup

1.  Clone the repository.
2.  Install Rust (stable).
3.  Install development dependencies: `cargo build`.

## Code Style

-   Follow standard Rust formatting: `cargo fmt`.
-   Ensure code is clippy-clean: `cargo clippy -- -D warnings`.
-   Write idiomatic Rust (use `Result` for errors, avoiding `unwrap`/`expect` where possible).

## Project Structure

The project is modularized into:
-   `src/core`: Domain types and configuration.
-   `src/services`: External integrations (LLM, TTS).
-   `src/utils`: Shared utilities.

## Testing

Run tests before submitting PRs:
```bash
cargo test
```

## Pull Requests

1.  Fork the repo and create a new branch.
2.  Make sure tests pass.
3.  Submit a PR with a description of changes.
