# Changelog

## 0.2.0 — 2026-10-15

Evidence Cockpit UI. The workbench is now a three-pane analyst cockpit: Rail, Board, and Inspector.

### Added

- Three-pane layout with a 6-item rail: Case, Entities, Evidence, Graph, Review, Jobs (`1`–`6`).
- Review lane: AI, playbook, and merge proposals wait here. `Shift+A` then `y` accepts. Enter never writes.
- Five cockpit themes (Dark, Light, Mono, Phosphor, Amber) with 256/16-colour fallbacks. Preview without the TUI: `os-int theme preview` / `--all`.
- Evidence tables with filtering, sorting, and Inspector source chains on every selected object.
- Motion modes `full`, `reduced`, and `off` (`ui.motion`). Slow sessions degrade to reduced.
- Graph as an edge-labelled tree from the focused node (corroboration heat, not risk).
- CLI parity: `os-int review list|accept|reject` — accept requires `--yes` or an interactive `y`.
- `--plain` and `OS_INT_PLAIN` for screen readers: ASCII glyphs, motion off, linear Rail / Board / Inspector.

### Changed

- Help is generated from the command registry. Ingest, playbooks, tools, settings, audit, and artifacts live in the palette (`Ctrl+K` / `:`).
- Forms and the agent prompt submit with `Ctrl+S`. Enter never writes.

### Breaking

- Removed standalone views: Notes, Artifacts, Audit, Sources, Pages, Agent, Tools, Settings, Timeline. Notes, sources, and history are Inspector tabs; the rest is the palette or a Board toggle (`t` for time).
- Removed keys: `p`, `v`, `r`, `i`, `b`, `o`/`d` on Welcome, `[` / `]`, `f`, `l`, `u`, and number keys `7`–`0`. Collect is `c`. There is no `ui.legacy_keys` fallback.
- Themes `default` and `high-contrast` are gone. Config still loads them as Cockpit Dark with an unknown-theme warning.
- Form and agent submit is `Ctrl+S`, not Enter.

### Install

Linux aarch64 has no prebuilt archive; install from source. Until crates.io, use `cargo binstall --git https://github.com/pjourney/osint-rat`.

## 0.1.0 — 2026-10-14

First tagged GitHub Release: linux x86_64, macOS x86_64 and aarch64, and Windows binaries plus SHA-256 checksums.
