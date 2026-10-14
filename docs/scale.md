# Scale notes

Phase 8 target: investigations around **100k objects** stay workable. Common TUI mutations (add a finding, set a marking, finish a small collection) must not freeze the UI for more than **50 ms** once the case is loaded.

DuckDB / Parquet is **not** shipped. SQLite plus an incremental UI store met the budget on the hardware below. Revisit a side store only if a later profile shows Object View or Search Around exceeding that budget after the case is in memory.

## Incremental UI store

Until Phase 8, every mutation called `Store::load_investigation` and rebuilt `Workspace` indexes. That is still used when opening a case and after resolution / playbook / report paths that rewrite many tables.

Hot paths now merge a `WorkspaceDelta`:

| Mutation | What is patched |
| --- | --- |
| Add finding / note / page | Returned row (+ page entity when created) |
| Set marking / add member | Investigation row, audit, or member list |
| Page notes (`e`) | That page |
| Collector `apply_batch` | Touched sources, entities, relationships, findings, page links |
| Finished collection job | Job row (no second full reload) |

`Workspace::apply_delta` upserts by UUID and updates short-ref indexes in place. Deleted findings (`resolved_findings`) are tombstoned so they do not linger. Member lists and page-entity links only append or upsert; removals still take a full reload. A correctness test (`apply_delta_matches_full_reload_after_batch`) compares patched timeline, event, artifact, and finding **records** (sorted by id) to a full reload.

## Common-action budget

**N = 50 ms.** Adding one finding on a 250-entity (and on measured 1k / 10k / 100k) fixture is an in-memory upsert of one row after a single SQLite insert. That stays well under 50 ms. Full reload of 100k entities is the cost of **opening** the case, not of typing `n` on Findings.

## Benchmarks

Command (writes JSON; creates throwaway investigations in the data dir):

```bash
os-int bench
os-int bench --sizes 1000,10000
```

Measured on the Phase 8 development VM (Linux 6.12, 4 vCPU Intel Xeon, 16 GB RAM, bundled SQLite, **release** `os-int`). Numbers are order-of-magnitude guides, not SLAs.

| Entities | Load snapshot (ms) | Build `Workspace` index (ms) | Search Around 2 hops (ms) | Incremental finding patch (ms) | Full reload (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1k | 1.4 | 0.25 | 0.19 | 0.003 | 1.4 |
| 10k | 13 | 2.4 | 1.8 | 0.005 | 12 |
| 100k | 127 | 46 | 28 | 0.011 | 127 |

Re-run `os-int bench` on your hardware and paste the JSON if you are filing a scale issue. Fields:

| Key | Meaning |
| --- | --- |
| `seed_ms` | Insert time for the synthetic domain chain |
| `load_ms` | `load_investigation` |
| `index_ms` | `Workspace::new` |
| `search_around_2hop_ms` | In-memory hop expansion from `E-1` |
| `incremental_finding_patch_ms` | SQLite `create_finding` plus in-memory `apply_delta` |
| `full_reload_ms` | Another `load_investigation` |
| `ui_mutation_budget_ms` | 50 |

Synthetic graph: domains `n{i}.scale.test` with `associated_with` every third node. Not an OSINT fixture; it only stresses entity/relationship volume.

## What is still a full snapshot

- Opening an investigation (TUI, `os-int get`, HTTP Object View).
- `Store::search_around` on the CLI/API (loads the case, then the same pure algorithm as the Graph screen).
- Merge / unresolve, playbook apply, ingest of a large file, report generation.

Those are expected to cost about `load_ms` above. The workbench keeps the snapshot in RAM afterward so Search Around on the Graph screen does not hit SQLite again.
