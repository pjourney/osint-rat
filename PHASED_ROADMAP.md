# os-int phased roadmap (enterprise-class ontology workbench)

**North star:** os-int is a local-first investigation ontology for OSINT and cyber cases—fuse sources into objects and links, resolve identities, Search Around the graph, run playbooks, and keep humans in control of every Action (including AI). Collectors are plugins. The graph and resolution engine are the product.

**Bar:** enterprise investigation platforms–class investigative loop (ontology, fusion, entity resolution, Search Around, temporal/geo, governed Actions, collaboration)—not recon CLI parity.

**How to use this document**

1. Implement **one phase per pull request series** (a phase may be several PRs, but the phase gate is one merge decision).
2. Do not start phase *N+1* work on `main` until phase *N* **merge criteria** are met and signed off (PR checklist + CI green).
3. Every phase PR must update this file’s **Phase status** table and link the closing PR(s).
4. Prefer small, reviewable PRs inside a phase; the phase ends only when the success criteria pass.

**Phase status**

| Phase | Name | Status |
| --- | --- | --- |
| 0 | Foundation & vision freeze | Done ([#4](https://github.com/pjourney/osint-rat/pull/4)) |
| 1 | Ontology runtime v1 | Done ([#5](https://github.com/pjourney/osint-rat/pull/5)) |
| 2 | Graph & Search Around | Done ([#6](https://github.com/pjourney/osint-rat/pull/6)) |
| 3 | Entity resolution | Done ([#7](https://github.com/pjourney/osint-rat/pull/7)) |
| 4 | Ingest plane & connectors | Done ([#8](https://github.com/pjourney/osint-rat/pull/8)) |
| 5 | Playbooks & automations | Done ([#9](https://github.com/pjourney/osint-rat/pull/9)) |
| 6 | Temporal & geospatial | Done ([#10](https://github.com/pjourney/osint-rat/pull/10)) |
| 7 | Team, markings & audit | Done ([#11](https://github.com/pjourney/osint-rat/pull/11)) |
| 8 | Scale, API & STIX | Done ([#12](https://github.com/pjourney/osint-rat/pull/12)) |
| 9 | Distribution, demos & open platform | In progress |

---

## Phase 0 — Foundation & vision freeze

### Goal

Lock the product thesis, document the ontology exchange shape, and add engineering guardrails so later phases do not invent incompatible batch formats.

### Features

- `VISION.md` stating the enterprise-class north star and non-goals (no active probing; no silent AI writeback; no chasing raw module count).
- This `PHASED_ROADMAP.md` as the merge-gated plan (this file).
- Document **Ontology Exchange Format** intent in `docs/ontology-exchange.md`: current `EvidenceBatch` / `investigation.osint.json` fields mapped to future Objects, Links, Events, Actions, Resolution bags; mark `FORMAT_VERSION` bump rules.
- CI reminders: phase PRs must include tests for any new model/storage path; `cargo fmt`, `clippy -D warnings`, `cargo test` remain required.
- GitHub milestones or labels `phase-0` … `phase-9` (optional but recommended).
- README short “What this is / is not” blurb aligned with the vision (no feature lies).

### Success criteria (must all pass before merge)

- [x] `VISION.md` and `PHASED_ROADMAP.md` are on `main`.
- [x] `docs/ontology-exchange.md` exists and accurately describes today’s export + the intended v2 additions (even if v2 is not implemented yet).
- [x] README vision blurb matches `VISION.md` (no contradiction with Boundaries / passive-only).
- [x] CI green on the phase PR(s).
- [x] Maintainer checklist in the PR: “No collector-count vanity metrics; success metrics listed in Phase 0 PR description.”

### How to complete

1. Branch from latest `main`.
2. Add `VISION.md` (thesis, capability pillars table summary, non-goals, success metrics: time-to-graph, % findings with sources, hops explored, playbooks run).
3. Add this roadmap file (or replace a draft) and set Phase 0 status to In progress → Done in the closing PR.
4. Write `docs/ontology-exchange.md` by reading `src/model/batch.rs`, `src/exchange.rs`, and `docs/schema.md`; document current format version and reserved extension points.
5. Edit README intro only as needed for alignment; do not rewrite the whole README.
6. Open PR titled `phase-0: vision, roadmap, ontology exchange docs`; fill the success checklist in the PR body.
7. Merge only when checklist is complete.

---

## Phase 1 — Ontology runtime v1

### Goal

Make Objects, properties, link types, and Actions first-class—not only freeform entities and ad hoc relationship strings.

### Features

- **Object type registry**: formalize page/entity kinds with documented property schemas (required/optional, typed fields) in model + docs (`docs/ontology.md`).
- **Link-type catalog**: built-in verbs (`resolves_to`, `subdomain_of`, …) registered with description, inverse (where applicable), and allowed endpoint kinds; custom snake_case links still allowed but catalogued when first used.
- **Object View** (TUI): for a selected entity/page, show properties, linked objects (grouped by link type), supporting sources, findings; keyboard path from Entities/Pages.
- **Action type stubs** in model: catalog of Action kinds already used by the agent (`ADD_FINDING`, `CREATE_PAGE`, …) plus reserved future kinds; store accepted/rejected Actions with audit fields (who/when—local user for now).
- Migration(s) only if storage needs new tables/columns for property bags or link metadata; keep UUIDs and short refs.
- Update `docs/ARCHITECTURE.md` “Where to make a change” for ontology edits.

### Success criteria

- [x] Object View can open any entity in a real investigation and show ≥1 property group and linked objects when relationships exist.
- [x] Link-type catalog is used when collectors emit known verbs (unknown verbs still store, but appear under “Other”).
- [x] Unit tests cover catalog lookup, inverse registration, and migration apply/rollback or forward-only safety.
- [x] `os-int doctor` still passes; existing investigations open without data loss.
- [x] Export/import round-trip still works for a fixture case (document any format bump).
- [x] Architecture + ontology docs updated; CI green.

### How to complete

1. Inventory `EntityKind`, `relation::*`, and agent `AgentAction` in `src/model/`.
2. Design minimal schema additions (prefer JSON property bags on entities over dozens of columns).
3. Implement registry modules + migrations + storage load/apply updates.
4. Add Object View screen/overlay wired from Entities (and optionally Pages).
5. Add fixtures under tests; run `cargo test`, clippy, fmt.
6. Update docs; PR `phase-1: ontology runtime v1` with checklist; merge when criteria pass.

---

## Phase 2 — Graph & Search Around

### Goal

Deliver a commercial link-analysis graph surface: explore the ontology visually and expand by query, not only tables.

### Features

- **Graph screen** (TUI MVP acceptable): nodes = entities (or pages), edges = relationships; select node → detail (reuse Object View data); filter by kind / link type / provenance.
- **Search Around**: from current selection, expand 1–N hops (default 1–2) with filters (link types, entity kinds, time window if available); results merge into the graph or a result set.
- **Histogram / facet strip**: counts for selected set (by kind, top link types); selecting a facet filters the graph.
- Commands: palette entries `Graph`, `Search Around`; keys documented in guide + keymap help.
- Performance guardrails: cap nodes/edges rendered; warn and require confirm above threshold.
- Tests for Search Around query logic against an in-memory or SQLite fixture graph.

### Success criteria

- [x] From a domain investigation with real relationships, user can open Graph, see connected infra, and Search Around ≥2 hops without crashing.
- [x] Caps enforced; facet filter changes visible set.
- [x] Keyboard + palette documented; `?` help updated.
- [x] Search Around pure logic unit-tested (hop counts, filters, caps).
- [x] No regression in table Entities view; CI green.

### How to complete

1. Prototype layout algorithm suitable for terminal (or staged webview—document choice in PR; TUI-first preferred for consistency).
2. Implement graph query in storage/graph layer (`src/storage/graph.rs` or successor).
3. Wire TUI screen + keymap + palette.
4. Build a fixture investigation (or generate in test) with known topology; assert Search Around results.
5. Update `docs/guide.md` Graph section; PR `phase-2: graph and search-around`; merge on checklist.

---

## Phase 3 — Entity resolution

### Goal

Entity resolution with reversible merge: merge records that are the same real-world thing without destroying history.

### Features

- **Resolution bags**: canonical object + constituent objects; property writes tracked for clean unresolve.
- **Manual merge / split UI**: select 2+ entities → propose merge → confirm; unresolve from Object View.
- **Suggestion engine (rules v1)**: same canonical email/domain/IP/repo URL; username↔profile URL; optional shared cert SAN clusters—scored suggestions, never auto-merge.
- Agent Action: `PROPOSE_RESOLVE` / `PROPOSE_UNRESOLVE` (accept required).
- Timeline events: `entities_resolved` / `entities_unresolved` with actor + constituents.
- Docs: analyst guide for safe merge practices.

### Success criteria

- [x] Merge two entities; Object View shows constituents; graph shows one node; sources from both remain cited on findings/links.
- [x] Unresolve restores constituents; no orphaned FKs in SQLite integrity check (`doctor` / pragma).
- [x] Suggestions appear for a fixture with known duplicates; auto-merge does not occur.
- [x] Export/import preserves resolution metadata (format bump if needed, with migration notes).
- [x] Tests for merge, unresolve, and suggestion scoring; CI green.

### How to complete

1. Spec resolution metadata (align with a “canonical / winner / others” mental model—adapted to local UUIDs).
2. Migrations + storage APIs; update workspace load.
3. UI flows + agent actions + timeline.
4. Fixtures proving round-trip; PR `phase-3: entity resolution`; merge on checklist.

---

## Phase 4 — Ingest plane & connectors

### Goal

Multi-source fusion: anything an analyst has becomes ontology Objects/Links with provenance—not only built-in collectors.

### Features

- **Connector trait / external collector protocol**: config `[[collectors.external]]` or sidecar that returns Ontology Exchange / EvidenceBatch JSON; documented in `docs/connectors.md`.
- **Built-in ingest**:
  - STIX 2.1 bundle import (subset: indicator, observed-data, malware, identity, relationship).
  - MISP event JSON import (attributes → objects/links best-effort).
  - Bulk CSV/JSON observable import with column mapping.
- **High-signal built-in collectors** (still passive): username pack; email enrichment pack; subdomain expansion loop (CT + optional `subfinder`/`amass -passive` via tool registry with entity promotion and `subdomain_of`); structured WHOIS fallback when RDAP is thin.
- Optional **API key connectors** (user-configured only): e.g. URLScan, OTX—thin wrappers, rate limits, secrets redacted.
- Guide updates: Username/Email/ASN pages no longer “empty.”

### Success criteria

- [x] External connector example (in-repo) produces entities/relationships via apply path without forking Rust.
- [x] STIX and MISP sample fixtures import into a new investigation with cited sources.
- [x] `os-int collect` on email and username yields useful graph edges without custom tools.
- [x] Subdomain expansion creates `subdomain_of` / `resolves_to` (not only `mentions`) under a configured cap.
- [x] Active tools remain refused; SECURITY/Boundaries unchanged in spirit.
- [x] Tests + CI green; docs updated.

### How to complete

1. Stabilize batch JSON schema from Phase 0 docs; implement external runner.
2. Add importers under `src/exchange/` or `src/ingest/`; CLI/palette commands.
3. Implement username/email/subdomain collectors; wire tool parsers to proper link types.
4. Ship `examples/connectors/hello-batch/` and sample STIX/MISP files.
5. PR `phase-4: ingest plane & connectors`; merge on checklist.

---

## Phase 5 — Playbooks & automations

### Goal

Capture analyst logic as reusable, reviewable workflows: templated analysis steps and investigative playbooks that stay under human control.

### Features

- **Playbook format** (YAML): seed object type + steps (`collect`, `search_around`, `run_tool`, `assert_finding_template`, `propose_resolve`).
- Palette: **Run playbook**; CLI: `os-int playbook run <file> --investigation …`.
- Bundled playbooks: `domain-infra`, `mail-policy`, `cert-estate`, `repo-surface`.
- **Automations v1**: schedule or manual “watch set” that re-runs a playbook’s collect steps and opens Action proposals on changes (no silent finding writes).
- Agent playbook prompts: palette seeds a fixed question + suggested Actions for the active agent.

### Success criteria

- [x] Bundled `domain-infra` playbook runs on `example.com` (or fixture) and leaves a cited graph + optional pending proposals only.
- [x] Invalid playbook YAML fails with a clear error; no partial DB corruption (transactional apply).
- [x] Automation dry-run mode shows planned Actions without applying.
- [x] Docs + examples; tests for playbook parser and one integration test; CI green.

### How to complete

1. Design YAML schema; implement parser + executor using existing JobManager/collectors/Search Around.
2. Add bundled files under `playbooks/`.
3. Wire CLI + palette + automation stub (cron-like local scheduler or `os-int watch` loop).
4. PR `phase-5: playbooks & automations`; merge on checklist.

---

## Phase 6 — Temporal & geospatial

### Goal

Time and place as first-class investigative dimensions (enterprise investigation platforms Timeline + geospatial investigation lite).

### Features

- **Event objects**: interval or instant; link entities to events; timeline UI overlays multiple entities.
- **Co-occurrence queries**: entities sharing time windows or appearing in the same sources/events.
- **Change intelligence**: promote observation-hash changes and diagnostic flips into Event/Finding candidates (still analyst-approved if AI-suggested).
- **Geo properties**: country/lat-lon on IP/ASN/org when available (RDAP, optional MaxMind-class local DB path in config—no phone-home).
- Map: **GeoJSON export** + optional TUI/list “nearby” by country/prefix; embedded map may wait for web companion.
- Guide: temporal/geo workflows.

### Success criteria

- [x] Timeline shows events from collection + imported events; filter by entity works.
- [x] Co-occurrence query returns expected pairs on a fixture.
- [x] GeoJSON export validates and includes ≥1 point/polygon/property from a fixture IP/ASN case.
- [x] Optional geo DB missing → graceful degrade (no crash).
- [x] Tests + CI green; docs updated.

### How to complete

1. Model `Event` + storage + migrations.
2. Extend timeline screen; add queries.
3. Attach geo fields in IP/ASN collectors; export command.
4. PR `phase-6: temporal and geospatial`; merge on checklist.

---

## Phase 7 — Team, markings & audit

### Goal

Small-cell collaboration with governance (enterprise investigation platforms collaboration / ACL spirit, local-first default).

### Features

- **Markings**: TLP (or simple confidentiality labels) on investigations and optionally objects; shown in UI and export manifest.
- **Audit log**: append-only events for merges, accepted Actions, playbook runs, exports, marking changes (local user id / machine name configurable).
- **Team sync (optional)**: documented sync server or “shared data dir” mode with locking; role model `owner` / `analyst` / `reader` for a case.
- Conflict policy for concurrent edits (last-write-wins with audit, or CRDT—choose and document).
- No cloud requirement: all features usable offline for a single analyst.

### Success criteria

- [x] Audit log records merge + accept Action + export on a test run; export can include audit slice.
- [x] TLP marking visible in UI and `investigation.osint.json`.
- [x] If sync is implemented: second client can open the same case read-only or as analyst per role; documented threat model in `docs/collaboration.md`.
- [x] Single-user path unchanged when sync disabled.
- [x] Tests for audit + markings; CI green.

### How to complete

1. Spec threat model (local trust boundary).
2. Implement markings + audit storage first (ships value alone).
3. Add sync only if design review passes; otherwise ship “export as handoff” hardening and defer sync to a follow-up PR still under Phase 7 criteria.
4. PR `phase-7: markings audit collaboration`; merge on checklist.

---

## Phase 8 — Scale, API & STIX

### Goal

Investigations larger than whole-workspace reload; ontology as an operational bus (enterprise ontology platforms ontology SDK spirit).

### Features

- **Incremental UI store**: stop full workspace reload on every mutation where possible; measure and document limits (target: 100k objects workable).
- **Ontology HTTP API** (localhost): CRUD-ish read + Action propose endpoints; auth token in config; OpenAPI snippet.
- **CLI completeness**: scriptable list/get/search-around/resolve for automation.
- **STIX 2.1 export** (complement Phase 4 import); document mapping gaps.
- Optional DuckDB/Parquet side store for heavy graph analytics—only if benchmarks require it.

### Success criteria

- [x] Benchmark doc: load/query times for fixture sizes (1k / 10k / 100k entities) on reference hardware; no UI freeze > N ms on common actions (state N in PR).
- [x] HTTP API serves Object View data for one investigation with token auth; unauthorized requests fail.
- [x] STIX export imports into a second empty investigation with acceptable fidelity (documented).
- [x] Regression tests; CI green; `docs/api.md` published.

### How to complete

1. Profile workspace reload; implement incremental paths for hot entities.
2. Add `os-int serve` or `os-int api` with tight bind default `127.0.0.1`.
3. STIX exporter; mapping table in docs.
4. PR `phase-8: scale api stix`; merge on checklist.

---

## Phase 9 — Distribution, demos & open platform

### Goal

Install in sixty seconds; show enterprise-class loop immediately; invite external ontology plugins.

### Features

- **Release pipeline**: GitHub Actions → binaries (linux/mac/win), checksums; `cargo-binstall` / Homebrew tap notes.
- **Demo case**: `examples/demo-case/` export; README “import this first” path shows Graph + resolved entities + playbook output.
- **Plugin marketplace docs**: how to publish an external collector / playbook; one community template PR.
- **Dependabot** + `cargo audit` in CI; SECURITY.md still accurate.
- README repositioned to ontology workbench (screenshots/GIF of Graph + Object View + Search Around).
- Public success metrics dashboard section in README (methodology, not vanity stars).

### Success criteria

- [x] Tagged release installs via documented one-liner on Linux and macOS (Windows noted).
- [x] New user: import demo case → open Graph → Search Around → see cited findings in <10 minutes without reading ARCHITECTURE.
- [x] External connector from Phase 4 still works against release binary.
- [x] CI includes audit; docs consistent; Phase status table all **Done**.
- [x] Maintainer sign-off that the north-star goals from the vision are met; deferred items listed in [`docs/deferred.md`](docs/deferred.md) (ready-to-file GitHub issue titles).

### How to complete

1. Add release workflow; produce a dry-run tag on a test tag if needed.
2. Curate demo investigation from real public domains (example.com / intentional fixtures only).
3. README + CONTRIBUTING plugin section; close the roadmap statuses.
4. PR `phase-9: distribution demos open-platform`; merge on checklist; cut `v1.0.0` (or next semver) only after merge.

The north-star loop from [VISION.md](VISION.md) is met on `main`: ontology objects and links, reversible resolution, Search Around, playbooks that only propose, governed Actions, passive built-ins, local-first data. Follow-ups that were explicitly optional or later (DuckDB, embedded map, sync server, hosted tap, crates.io, local counters, web companion) are listed in [docs/deferred.md](docs/deferred.md).

---

## Cross-phase rules

1. **Passive-only built-ins.** Tools/collectors marked active stay refused.
2. **AI never silently writes.** Proposals only; accepted AI findings keep `agent` provenance.
3. **Provenance is sacred.** Fusion and resolution must not relabel `COL` as something else.
4. **Format bumps** bump exchange `FORMAT_VERSION` and include import compatibility notes.
5. **Docs in the same PR** as behavior changes (`guide.md`, keymap, ARCHITECTURE).
6. **One phase gate at a time** on `main`. Feature flags may land dark but default-off until the phase merges.

## Metrics (track from Phase 1 onward)

- Time from seed domain to first useful graph (≥20 linked entities) on a standard fixture.
- % of findings with ≥1 cited source.
- Search Around hops used per session (telemetry optional/local).
- Playbooks run successfully per week (local counter OK).
- External connector count in-tree vs out-of-tree.

