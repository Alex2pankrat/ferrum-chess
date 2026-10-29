# Contributing

Ferrum Chess aims to be a respectful, offline-first, accessible native chess application. Small focused changes, tests, and clear explanations are welcome. Please report security issues privately to the maintainers rather than posting exploit details publicly.

## Setup

Install current stable Rust with `rustfmt` and `clippy`, native dependencies described in the README, and a local Stockfish binary. Clone the repository and run `cargo run`. Windows releases additionally require WiX 4 and cargo-dist (see the release workflow).

## Workflow

Create a feature branch from `main`, make focused commits and rebase on `main` before opening a pull request. Keep history linear. Use Conventional Commits in the imperative without a final period, e.g. `feat(ui): highlight legal moves`, `fix(engine): report startup errors`, `docs: explain installation`, `refactor: simplify state`, `test: cover castling`, `chore: update dependencies`; use `!` after the type or scope for breaking changes. Explain behavioural changes, test coverage, screenshots for UI changes, and any license implications in the PR.

Run all of the following before requesting review:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Also manually test Stockfish, both player sides, PGN round trips, and the Windows installer when changing those areas. Never commit binaries, credentials, or large generated build artifacts.
