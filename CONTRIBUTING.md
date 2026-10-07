# Contributing to os-int

Thank you for considering a contribution. os-int is meant to be a maintainable investigation workbench, not a pile of one-off scripts.

## Before you start

1. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
2. Keep changes in the layer they belong to: domain types stay free of Ratatui; collectors never write to SQLite; the TUI never performs HTTP.
3. Treat retrieved content as untrusted. Sanitize before display or agent context.

## Development setup

Rust 1.88 or newer. The usual commands:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Configuration and data for a throwaway run:

```bash
cargo run -- --data-dir /tmp/os-int-dev --config /tmp/os-int-dev/config.toml
```

## Style

- English for identifiers and comments.
- Prefer small functions with a single purpose.
- Errors from collectors and optional tools become job failures; do not `.unwrap()` on production paths.
- Do not execute commands through a shell. Use `crate::exec` with an explicit argv.
- New collectors implement `Collector` and register in `CollectorRegistry::builtin`.

## Tests

Add tests next to the code they cover:

- parsers and `build_batch` functions (no network)
- `App::dispatch` for UI state transitions
- sanitization and path validation
- report rendering

Live collectors may be exercised with `os-int collect`, but CI should not depend on the public internet.

## Pull requests

- One concern per PR when practical.
- Update `docs/ARCHITECTURE.md` if you change a recorded decision.
- Do not commit `target/`, databases, or investigation snapshots.

## Security issues

Please follow [SECURITY.md](SECURITY.md) instead of opening a public issue.
