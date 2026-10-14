# Contributing to os-int

Thanks for considering a contribution. os-int is a terminal workbench for public-information investigations. Changes should leave that workbench maintainable: small modules, provenance intact, and collected text treated as data.

## Before you start

1. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), especially "Where to make a change" and the recorded decisions.
2. Read the [user guide](docs/guide.md) if your change affects what an analyst sees or types.
3. Keep each change in the layer that owns it.
   - Domain types stay free of Ratatui and SQLite.
   - Collectors return a batch. They do not write the database.
   - The TUI renders `App`. It does not perform HTTP or spawn processes itself.
   - Subprocesses go through `crate::exec` with an explicit argv.
4. Treat retrieved content as untrusted. Sanitize it before display or before it is placed in an agent context.

## Development setup

Rust **1.88** or newer (edition 2024).

```bash
git clone https://github.com/pjourney/osint-rat.git
cd osint-rat
cargo test --all-targets
```

Use a throwaway data directory so you do not mix experiments with real cases:

```bash
cargo run -- --data-dir /tmp/os-int-dev --config /tmp/os-int-dev/config.toml
```

Checks this repository expects, and that CI runs on every pull request (`.github/workflows/ci.yml`):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

These three commands stay required. CI also runs `cargo audit`. The [phased roadmap](PHASED_ROADMAP.md) is complete; new work is ordinary pull requests against `main`, still bound by the [cross-phase rules](PHASED_ROADMAP.md#cross-phase-rules). Any pull request that adds or changes a model or storage path must include tests for that path in the same PR.

To publish an external collector or playbook, copy [`examples/community/`](examples/community/) and follow [docs/plugins.md](docs/plugins.md). Title the pull request `community-plugin: <id> — <one-line description>`.

`cargo fmt` (without `--check`) rewrites formatting. `cargo run -- doctor` is a useful smoke test after a change to startup, paths, or tool detection.

## Where the code lives

| Task | Location |
| --- | --- |
| A new record field | `src/model/`, then `src/storage/` |
| An object type, property, or link verb | `src/model/ontology.rs` and [docs/ontology.md](docs/ontology.md) |
| A passive collector or diagnostic | `src/collectors/` |
| File ingest (STIX, MISP, CSV, batch) | `src/ingest/` and [docs/connectors.md](docs/connectors.md) |
| STIX export | `src/ingest/stix_export.rs`, `os-int export --format stix` |
| Localhost HTTP API | `src/api/`, `os-int serve`, [docs/api.md](docs/api.md) |
| Incremental UI store / scale benches | `src/model/workspace.rs`, `src/storage/delta.rs`, [docs/scale.md](docs/scale.md) |
| A playbook or watch set | YAML under `playbooks/`, code in `src/playbook/`, [docs/playbooks.md](docs/playbooks.md) |
| An external sidecar | `[[collectors.external]]` emitting Evidence Batch JSON; example under `examples/connectors/` |
| A community plugin (collector or playbook) | Copy [`examples/community/`](examples/community/); follow [docs/plugins.md](docs/plugins.md) |
| The bundled demo investigation | `src/demo.rs` and [`examples/demo-case/`](examples/demo-case/); rewrite JSON with the ignored `write_checked_in_demo_archive` test |
| Release / install docs | `.github/workflows/release.yml`, `scripts/install.sh`, [docs/install.md](docs/install.md) |
| A built-in external tool | `src/tools/registry.rs` |
| Process spawning, timeouts, redaction | `src/exec/` |
| Agent profiles, context fences, proposal parsing | `src/agent/` |
| Key bindings and the in-app help text | `src/tui/keymap.rs` |
| Screen layout | `src/tui/screens.rs` |
| State transitions | `src/app/` |
| Reports and import/export | `src/report/`, `src/exchange/` |
| Behavior an analyst relies on | [docs/guide.md](docs/guide.md) in the same change |

New collectors implement `Collector` and register in `CollectorRegistry::builtin`. Update the collector matrix in the user guide when the default set or the accepted target kinds change.

## Style

- English for identifiers, comments, and documentation.
- Small functions with a single purpose.
- Errors from collectors and optional tools become job failures with a message a person can act on.
- Avoid `.unwrap()` on production paths. Tests may use `.expect("reason")`.
- Do not execute commands through a shell.
- Built-in collectors and tools stay passive. Configuration marked `active` is refused. Do not add remote access, exploitation, credential attacks, or active network probes.

## Tests

Follow Arrange–Act–Assert. Name the values `input`, `actual`, and `expected` when that makes the assertion easier to read.

Add tests next to the code they cover:

| Area | What to test |
| --- | --- |
| Parsers and `build_batch` | No network. Feed fixture text, assert the batch. |
| New or changed `src/model/` / `src/storage/` paths | Required on phase PRs. Cover apply, load, and export/import when the on-disk shape changes. |
| `App::dispatch` | UI state transitions without a terminal. |
| `sanitize` and path checks | Terminal escapes, path traversal, fence neutralization. |
| Report rendering | Provenance labels and source ids survive into the document. |
| Config | Partial files still fill defaults (`src/config.rs`). |

Live collectors can be exercised with `os-int collect`. CI must not depend on the public internet.

## Pull requests

- One concern per pull request when practical.
- Describe the analyst-visible change in the pull request body.
- Update [docs/guide.md](docs/guide.md) when commands, keys, collectors, or settings change.
- Update [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) when you change a recorded decision.
- Do not commit `target/`, databases, logs, or ad-hoc investigation snapshots. The bundled demo archive under `examples/demo-case/` is the exception and is generated from `src/demo.rs`.

### Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --all-targets`
- [ ] Tests added for any new or changed model/storage path
- [ ] Docs updated when behavior an analyst can observe has changed (`guide.md`, and [ontology-exchange.md](docs/ontology-exchange.md) when export or `EvidenceBatch` changes)
- [ ] New collector or tool is passive, and untrusted output is sanitized
- [ ] Phase PR: [PHASED_ROADMAP.md](PHASED_ROADMAP.md) status/checklist updated; no collector-count vanity metrics
- [ ] Community plugin PR: [docs/plugins.md](docs/plugins.md) review bar (passive-only, provenance, tests)

Recommended GitHub labels: `phase-0` … `phase-9` (historical; the roadmap is complete). For a new sidecar or playbook, use the [community plugin](.github/ISSUE_TEMPLATE/community_plugin.md) issue template.

## Security issues

Please follow [SECURITY.md](SECURITY.md). Use a private report rather than a public issue for vulnerabilities that could be exploited.

## See also

- [README](README.md)
- [Vision](VISION.md)
- [Phased roadmap](PHASED_ROADMAP.md)
- [User guide](docs/guide.md)
- [Examples](docs/examples.md)
- [FAQ](docs/faq.md)
- [Database tables](docs/schema.md)
- [Ontology Exchange Format](docs/ontology-exchange.md)
- [Playbooks](docs/playbooks.md)
- [Plugins](docs/plugins.md)
- [Install](docs/install.md)
- [Collaboration](docs/collaboration.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Code of conduct](CODE_OF_CONDUCT.md)
