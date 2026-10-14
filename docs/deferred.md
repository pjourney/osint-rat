# Deferred after the phased roadmap

Phase 9 closes the merge-gated plan in [PHASED_ROADMAP.md](../PHASED_ROADMAP.md). The items below are **explicitly not** in the v1 workbench. File them as GitHub issues after merge if they are still wanted; titles are ready to paste.

None of these relax the cross-phase rules: built-ins stay passive, active tools stay refused, AI only proposes, provenance is preserved.

## DuckDB / Parquet side store

**Title:** Optional DuckDB/Parquet side store for heavy graph analytics

Phase 8 skipped this because SQLite plus incremental `WorkspaceDelta` met the 50 ms common-action budget at 100k entities ([scale.md](scale.md)). Revisit only if Object View or Search Around exceeds that budget after the case is in memory.

## Embedded map

**Title:** Embedded map for geo points (web companion)

Phase 6 ships GeoJSON export and a TUI “nearby” list. An embedded map was deferred. Keep lookups on a local geo DB; no phone-home.

## Real-time sync server / CRDTs

**Title:** Multi-writer sync server (beyond shared data dir)

Phase 7 ships markings, audit, roles, and shared-directory locking with last-write-wins. A dedicated sync server or CRDT replication is out of scope for v1. Threat model stays in [collaboration.md](collaboration.md).

## Hosted Homebrew tap and crates.io crate

**Title:** Publish a Homebrew tap and crates.io os-int crate

Release artifacts and `cargo-binstall` metadata ship in Phase 9. Until crates.io, install with `cargo binstall --git https://github.com/pjourney/osint-rat`. Publishing a tap or a crates.io package is a maintainer action after `v1.0.0` is tagged on `main` and the draft GitHub Release is published. Do not publish from a phase pull request.

## serde_yaml / unsafe-libyaml (RUSTSEC-2024-0449)

**Title:** Replace serde_yaml so cargo audit has no allowed unmaintained warning

Playbook YAML parsing uses `serde_yaml` 0.9, which depends on `unsafe-libyaml`. That crate is unmaintained ([RUSTSEC-2024-0449](https://rustsec.org/advisories/RUSTSEC-2024-0449)). `cargo audit` currently reports it as an allowed warning, not a vulnerability in os-int itself. Migrating the YAML parser is follow-up after v1.

## Session telemetry

**Title:** Optional local counters for Search Around hops and playbooks/week

The [metrics](metrics.md) methodology allows a local counter. Nothing is uploaded. A future patch may add an on-disk counter behind a default-off config key.

## Web companion UI

**Title:** Web companion for Graph / Object View

The TUI is the supported surface. A browser companion was mentioned as later work in Phase 2/6 notes and is not required for the north star.
