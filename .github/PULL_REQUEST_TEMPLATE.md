## Summary

<!-- Analyst-visible change in a few sentences. -->

## Phase

<!-- Delete the rows that do not apply. Phase N+1 work on main waits until phase N is signed off. -->

- [ ] Phase 0 — Foundation & vision freeze
- [ ] Phase 1 — Ontology runtime v1
- [ ] Phase 2 — Graph & Search Around
- [ ] Phase 3 — Entity resolution
- [ ] Phase 4 — Ingest plane & connectors
- [ ] Phase 5 — Playbooks & automations
- [ ] Phase 6 — Temporal & geospatial
- [ ] Phase 7 — Team, markings & audit
- [ ] Phase 8 — Scale, API & STIX
- [ ] Phase 9 — Distribution, demos & open platform
- [ ] Not a phase-gated change

## Checks

CI runs these on every PR and they remain required:

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --all-targets`

Also:

- [ ] Tests added for any new or changed model/storage path
- [ ] Docs updated when behavior an analyst can observe has changed
- [ ] Export/import notes updated in `docs/ontology-exchange.md` if the archive or `EvidenceBatch` shape changed (`FORMAT_VERSION` bumped when the [bump rules](../docs/ontology-exchange.md#format_version-bump-rules) say so)
- [ ] Built-in collectors/tools stay passive; no silent AI writeback
- [ ] No collector-count vanity metrics; success is the phase criteria in `PHASED_ROADMAP.md`
- [ ] `PHASED_ROADMAP.md` status table updated when this PR closes a phase

## Success metrics (phase PRs)

<!-- List the phase success criteria this PR claims, or write "n/a". -->
