# Ontology Exchange Format

Two related payloads move investigation data in and out of os-int:

1. **Investigation archive** — the on-disk export written by `os-int export` (`investigation.osint.json` plus a `files/` tree). This is format `os-int-investigation`, version **`4`**.
2. **Evidence batch** — the in-memory and connector payload collectors and ingest return. `storage::apply` writes it in one transaction. On disk / sidecar stdout it uses envelope `format` `os-int-evidence-batch`, `format_version` **1** (`ingest::BATCH_FORMAT_VERSION`). That constant is not the archive `FORMAT_VERSION`.
3. **STIX 2.1 bundle** — `os-int export --format stix` / `os-int ingest --format stix`. A separate envelope (`type: bundle`, `spec_version: 2.1`). It does **not** bump archive `FORMAT_VERSION`. Mapping and gaps: [connectors.md](connectors.md#stix-21).

This page describes **today’s** shapes from [`src/exchange.rs`](../src/exchange.rs), [`src/model/batch.rs`](../src/model/batch.rs), and [`docs/schema.md`](schema.md), then reserved later-version extension points and the rules for bumping `FORMAT_VERSION`. Version 2 ships **resolution bags** as `data.resolutions`. Version 3 ships **first-class Event objects** as `data.events`. Version 4 ships **structured markings** on `investigation.marking`, **`data.audit`**, and **`data.members`**. Playbook YAML is a **separate** envelope (`os-int-playbook`, `PLAYBOOK_FORMAT_VERSION` 1) and is not this archive. Do not emit reserved keys that are not listed as implemented.

The product thesis is in [VISION.md](../VISION.md). Phase gates are in [PHASED_ROADMAP.md](../PHASED_ROADMAP.md).

---

## Investigation archive (format version 4)

`os-int export` writes a directory:

```text
<destination>/
  investigation.osint.json    # ArchiveManifest
  files/                      # copy of the investigation directory (if it exists)
```

`exchange::MANIFEST_NAME` is `investigation.osint.json`. `exchange::FORMAT_VERSION` is `4`. Version 1–3 archives still import (`events` / `audit` / `members` default to empty; `investigation.marking` defaults to `unmarked`). Suggested directory name: `<sanitized-investigation-name>-<32 hex digits>` (`exchange::suggested_export_name` uses `Uuid::simple()`, no hyphens).

Import accepts either the directory or a path to the JSON file. When the path is a file, `files/` is taken as a sibling of that file.

### Manifest envelope

`ArchiveManifest` (`src/exchange.rs`):

| Field | Type | Today |
| --- | --- | --- |
| `format` | string | Always `os-int-investigation`. |
| `format_version` | `u32` | `4` (`exchange::FORMAT_VERSION`). Readers accept any version ≤ 4. |
| `exported_at` | RFC 3339 timestamp | Set at export time (`model::now()`, millisecond UTC). |
| `data` | object | `InvestigationData` — every record of one investigation. |

Paths inside `data` stay **relative** to the investigation directory. Export does not rewrite them to absolute paths on the originating machine.

### `data` (`InvestigationData`)

`InvestigationData` is also the body the workbench loads (`src/model/workspace.rs`). Join tables are flattened onto parent records except `page_entities`.

| Field | SQLite / notes |
| --- | --- |
| `investigation` | One case: `id`, `name`, `description`, `tags` (string array), `marking` (`unmarked` / `tlp:clear` / `tlp:green` / `tlp:amber` / `tlp:amber+strict` / `tlp:red`; default `unmarked` on older archives), `created_at`, `updated_at`. `name` is unique case-insensitively in a store. |
| `pages` | Subjects: `id`, `investigation_id`, `seq`, `title`, `kind`, `target`, `description`, `notes`, `entity_id`, timestamps. |
| `sources` | Observations. `snapshot_path` is relative. `content_hash` is SHA-256 hex of the raw snapshot. |
| `findings` | Claims. `source_ids` is the flattened `finding_sources` join. Empty `source_ids` is a hypothesis. |
| `entities` | Normalized identifiers. Unique on `(investigation_id, kind, value)`. `source_ids` is the flattened `entity_sources` join. |
| `relationships` | Directed edges. Unique on `(from_id, kind, to_id)`. `source_ids` is the flattened `relationship_sources` join. |
| `notes` | Standalone notes with their own provenance. Page notes live on `pages.notes`, not here. |
| `artifacts` | Files under the investigation directory. `path` is relative. |
| `timeline` | Point-in-time log rows (`timeline_events`). |
| `events` | First-class Event objects (Phase 6). Instants or intervals with `entity_ids`. Empty / omitted on version-1 and version-2 archives (`#[serde(default)]`). |
| `jobs` | Background work. `investigation_id` is optional on the struct; arguments are redacted. |
| `agent_runs` | Questions sent to a local agent CLI and the replies. |
| `proposals` | Structured `AgentAction` values with `pending` / `accepted` / `rejected` / `invalid`. Optional `decided_by` (local actor) is present after accept/reject on current binaries; missing on version-1 archives. |
| `resolutions` | Resolution bags (Phase 3). Empty / omitted on version-1 archives (`#[serde(default)]`). |
| `page_entities` | `{ "page_id", "entity_id" }` links (`page_entities` table). A page’s `entity_id` is the primary subject and may be omitted from this list. |
| `audit` | Append-only governance log (Phase 7). Empty / omitted on version-1–3 archives (`#[serde(default)]`). |
| `members` | Case members and roles. Empty means single-user owner. Empty / omitted on version-1–3 archives. |

**Not exported:** `ui_state` (last view, last page, agent thread), `playbook_runs`, and `automations`. Those are local workbench chrome, not case evidence. Playbook YAML lives under [`playbooks/`](../playbooks/) or a path you pass to `os-int playbook run`; it is not packed into the archive.

Every investigation-scoped row has `id` (UUID, stable across export/import), `investigation_id`, and `seq` (per-table short reference: `P-1`, `S-12`, `E-4`, …), with two exceptions: `page_entities` is only `{ page_id, entity_id }` (no `id`, `investigation_id`, or `seq`), and `JobRecord.investigation_id` is `Option<Uuid>`. Short refs are not unique across tables.

### Shared field encodings

Serde shapes as they appear in the JSON:

| Concept | Encoding |
| --- | --- |
| UUID | Hyphenated string. |
| Timestamp | RFC 3339 UTC, millisecond precision (`chrono` + `model::now()`). |
| `provenance` | `collected` \| `external` \| `analyst` \| `agent`. |
| `confidence` | `unknown` \| `low` \| `medium` \| `high` \| `confirmed`. |
| Open kinds (`PageKind`, `EntityKind`, `SourceKind`, `ArtifactKind`, `TimelineKind`) | Storage key string (`domain`, `ip`, `dns`, …). Unknown keys become `Other` on read. |
| `Fields` | JSON array of `[key, value]` string pairs (`serde(transparent)` on `Vec<(String, String)>`). |
| `JobStatus` | `queued` \| `running` \| `succeeded` \| `failed` \| `cancelled`. |
| `ProposalStatus` | `pending` \| `accepted` \| `rejected` \| `invalid`. |
| `AgentAction` | Internally tagged: `{ "action": "ADD_FINDING", ... }` (`SCREAMING_SNAKE_CASE`). |

Page kinds today: `generic`, `person`, `organization`, `domain`, `ip`, `asn`, `url`, `email`, `username`, `repository`, `file`, `event`.

Entity kinds today: `person`, `organization`, `domain`, `hostname`, `ip`, `email`, `username`, `url`, `repository`, `file`, `location`, `phone`, `asn`, `certificate`, `unknown`.

Source kinds today: `dns`, `rdap`, `http`, `tls`, `ct`, `ip_intel`, `repository`, `file`, `command`, `web`, `document`, `manual`, `stix`, `misp`.

Artifact kinds today: `snapshot`, `file`, `report`, `agent_context`, `tool_output`, `repository`.

Timeline kinds today: `registration`, `expiration`, `updated`, `certificate_issued`, `certificate_expires`, `commit`, `release`, `published`, `observed`, `changed`, `user`, `finding`, `investigation`, `entities_resolved`, `entities_unresolved`, `playbook_run`. `playbook_run` is additive (`TimelineKind::Other` on a down-level reader); it does not bump `FORMAT_VERSION`.

Built-in relationship verbs (`src/model/relationship.rs` `kinds`) include `resolves_to`, `alias_of`, `uses_nameserver`, `mail_handled_by`, `spf_includes`, `reverse_dns`, `announced_by`, `operated_by`, `registered_with`, `registered_to`, `redirects_to`, `serves`, `links_to`, `mentions`, `presents_certificate`, `issued_by`, `covers`, `subdomain_of`, `references`, `contributed_to`, `hosted_on`, `associated_with`, `contains`. Other `snake_case` verbs are stored as-is.

Agent actions today: `ADD_FINDING`, `ADD_NOTE`, `CREATE_PAGE`, `LINK_ENTITY`, `RUN_COLLECTOR`, `GENERATE_REPORT`, `PROPOSE_RESOLVE`, `PROPOSE_UNRESOLVE`, `RUN_PLAYBOOK`, `ASSERT_FINDING_TEMPLATE`. `RUN_PLAYBOOK` may only name a bundled id; nested playbooks and file paths are refused. `ASSERT_FINDING_TEMPLATE` stays pending until accept. Automations propose; they do not silently write findings. Catalog: [ontology.md](ontology.md). Playbooks: [playbooks.md](playbooks.md).

### `data.resolutions` (format version 2)

Each bag:

| Field | Notes |
| --- | --- |
| `id`, `investigation_id`, `seq` | Stable UUID and short ref `V-n`. |
| `canonical_id` | Winner entity UUID. |
| `constituent_ids` | All members, including the canonical. |
| `status` | `active` or `unresolved`. |
| `reason`, `created_by`, `created_at` | Why and who merged. |
| `unresolved_by`, `unresolved_at` | Set after split. |
| `property_writes` | `{ key, previous, new_value }` applied to the canonical (`__label` / `__notes` for columns). |
| `source_writes` | Source UUIDs copied onto the canonical. |

Constituent entity rows remain. Unresolve undoes the writes in this bag. `--rename` remaps every record UUID, including `resolutions.id`, `canonical_id`, `constituent_ids`, and `source_writes`.

### `data.events` (format version 3)

Each Event:

| Field | Notes |
| --- | --- |
| `id`, `investigation_id`, `seq` | Stable UUID and short ref `C-n` (`RefKind::Event`). Timeline log rows stay `T-n`. |
| `kind` | Same open enum as timeline log rows (`observed`, `changed`, `finding`, …). |
| `title`, `description` | Display text. |
| `started_at` | Instant start (UTC RFC 3339). |
| `ended_at` | Optional. Present means an interval; absent means an instant. |
| `provenance`, `source_id`, `page_id` | Same meaning as other records. |
| `fields` | Property bag. Change intelligence sets `change_intelligence` to `observation_hash`, `diagnostic_open`, or `diagnostic_clear`. |
| `entity_ids` | Linked entities. `--rename` remaps these. Resolution bags: overlay and co-occurrence use `member_ids_of`. |

The Timeline screen overlays `timeline` log rows and `events`. When both exist for the same kind/time/title, the Event wins so intervals stay visible. Observation-hash changes and diagnostic flips still write a log row and also promote an Event candidate (analyst-approved if an agent suggested a finding).

### `data.audit` and `data.members` (format version 4)

Each audit row: `id`, `investigation_id`, `seq` (short ref `L-n`), `kind` (`entities_merged`, `entities_unresolved`, `action_accepted`, `playbook_run`, `export`, `marking_changed`, `member_added`, `member_role_changed`, `member_removed`), `actor`, `machine`, `occurred_at`, `summary`, `details` (field bag), `target_ids` (related UUIDs). `--rename` remaps `id`, `investigation_id`, and `target_ids`.

Each member: `investigation_id`, `user_id`, `role` (`owner` / `analyst` / `reader`), `added_by`, `added_at`. `--rename` rewrites `investigation_id` only (`user_id` is not a record UUID).

Object-level markings are optional keys on `entities.fields` (`marking`). They do not have their own collection.

### Import behavior (v1, v2, v3, and v4)

From `exchange::import_investigation`:

1. Parse the manifest. If `format_version` **>** `FORMAT_VERSION`, reject. Older or equal versions are accepted.
2. Reject artifact `path` or source `snapshot_path` values that are not safe relative paths (`paths::is_safe_relative_path`).
3. If an investigation with the same `id` exists: reject unless `--rename`. `--rename` assigns a new investigation UUID, suffixes the name with ` (imported)`, remaps every record UUID (pages, sources, entities, relationships, resolutions, events, audit entries, and all internal references), and rewrites `investigation_id` on child rows and members.
4. If the name already exists (case-insensitive), append ` (2)`, ` (3)`, … or a UUID suffix.
5. Insert `InvestigationData`, then copy `files/` into the new investigation directory.

Copy skips `.`, `..`, and names starting with `tmp-`. Directory depth is capped at 32. All symlinks are skipped (`DirEntry::metadata` does not follow them, so they are neither `is_dir` nor `is_file`). The cycle check is only for overlapping real directories (`canonicalize` plus a prefix test) before recursing.

### File tree

`files/` is a copy of `<data_dir>/investigations/<uuid>/` (snapshots, imported files, agent context packs, reports, repo clones). Database rows point at those relative paths. See [Architecture — storage layout](ARCHITECTURE.md#storage-layout).

---

## Evidence batch (collector / future connector payload)

Collectors do not write SQLite. They return an `EvidenceBatch` (`src/model/batch.rs`). `storage::apply` applies it atomically (`docs/schema.md` “How a collection lands”).

This is the ingest shape connectors emit as JSON. It is **not** `investigation.osint.json`. The public sidecar / file envelope is:

```json
{ "format": "os-int-evidence-batch", "format_version": 1, "batch": { … } }
```

`ingest::BATCH_FORMAT_VERSION` is **1**. A raw batch object (no envelope) is accepted. Versions greater than 1 are refused. An `os-int-investigation` archive is refused here (`os-int import` is the archive path). Do not overload the archive `FORMAT_VERSION` (currently **4**). Protocol, caps, and examples: [connectors.md](connectors.md). Optional `events` on a batch is additive (`#[serde(default)]`) and does not bump the batch envelope.

| Field | Role |
| --- | --- |
| `sources` | `NewSource` rows: `kind`, `title`, `url`, `retrieved_at`, `published_at`, `author`, `collector`, `content_hash`, `snapshot_path`, `content`, `fields`, `provenance`. |
| `entities` | `NewEntity`: `key` (`kind` + canonical `value`), `label`, `fields`, `sources` (indexes into `sources`), `link_to_page`. |
| `relationships` | `NewRelationship`: `from` / `to` (`EntityKey`), `kind`, `sources` (indexes), `confidence`. |
| `timeline` | `NewTimelineEvent`: `occurred_at`, `kind`, `title`, `description`, `source`, `entity`. Untrusted timestamps are parsed as UTC; malformed values are skipped. |
| `events` | `NewEvent`: `started_at`, optional `ended_at`, `kind`, `title`, `description`, `source`, `entities`, `fields`. Optional. |
| `artifacts` | `NewArtifact`: `kind`, `name`, relative `path`, `mime`, `size_bytes`, `sha256`, `fields`, `source`. |
| `findings` | `NewCollectedFinding`: `title`, `details`, `confidence`, `source`, `provenance`. Built-in diagnostics are `collected` (upsert / clear). File and sidecar ingest forces `external` and does not use the collected-diagnostic upsert path. |
| `resolved_findings` | Titles of collected findings that no longer apply (a later `present` reading). Honored only when every source in the batch is `collected`. File/sidecar ingest ignores this field. |
| `summary` | One-line human summary for the job result. |

`SourceKey` is a `usize` index into `sources`, not a UUID. Entity identity in a batch is `EntityKey` (kind + normalized value), not a UUID. Apply upserts entities on `(kind, canonical value)` and relationships on `(from, kind, to)`. A source with the same collector, title, URL, and content hash is marked seen again instead of inserted (`find_duplicate_source`). A new hash of a prior observation (same collector, title, and URL) inserts a `changed` timeline event.

`EvidenceBatch::is_empty` is true only when `sources`, `entities`, `timeline`, and `events` are all empty (relationships / artifacts / findings alone do not count).

---

## Today → intended v2 ontology

v1 already stores a cited graph. Phase 1 registers object types, link verbs, and Action kinds against those existing fields. Later phases still map; do not invent a second parallel model.

| v2 concept | Phase | v1 field(s) to extend | Reserved addition (not shipped) |
| --- | --- | --- | --- |
| **Object** | 1 | `data.entities`, `data.pages` | Typed property schemas on `kind` in `docs/ontology.md` and `model/ontology.rs`. Property bags stay `entities.fields`. UUIDs and `seq` unchanged. |
| **Link** | 1 | `data.relationships` (`kind` is still a free `snake_case` string) | Catalog metadata: description, inverse, allowed endpoint kinds. Unknown verbs still store, grouped as “Other”. First-use rows go in SQLite `link_types` (not exported). |
| **Event** | 6 | `data.events` (format version 3) plus overlay of `data.timeline` | Interval or instant Event objects; links from entities to events; co-occurrence. Log rows remain. |
| **Action** | 1, 5 | `data.proposals` + `AgentAction` | Catalog of action kinds including `RUN_PLAYBOOK` and `ASSERT_FINDING_TEMPLATE`. Optional `decided_by` plus existing `decided_at`. Automations propose; they do not silently write. |
| **Resolution bag** | 3 | `data.resolutions` (format version 2) | Canonical object + constituents; property writes tracked for unresolve. Never auto-merge. |
| **Markings** | 7 | `investigation.marking` (format version 4); optional `entities.fields.marking` | TLP on the investigation and optionally on objects. Visible in the UI and the manifest. |
| **Audit** | 7 | `data.audit` (format version 4) | Append-only merge / accept / playbook / export / marking / membership events. |

Ingest (Phase 4) still lands through `EvidenceBatch` → `storage::apply`. External connectors should emit the batch JSON, not a hand-built SQLite file.

Reserved **`data` keys** for a version-2 archive (do not write them at `format_version` 1):

- `objects` — only if entities/pages are insufficient and a parallel collection is required
- `links` — only if relationships need a parallel catalogued collection
- `events` — **shipped in version 3** (Phase 6)
- `actions` — if Actions outgrow `proposals`
- `resolutions` — **shipped in version 2** (Phase 3)
- `markings` — **shipped in version 4** as `investigation.marking` (not a parallel collection)
- `audit` — **shipped in version 4** as `data.audit`
- `members` — **shipped in version 4** as `data.members`

Prefer **evolving the existing arrays** (add optional fields with `#[serde(default)]`) over forking new collections. Add a new collection only when the old row cannot carry the semantics (resolution bags, audit).

Reserved **`EvidenceBatch` keys** for a future connector protocol: `objects`, `links`, `actions`, `resolutions`. `events` is implemented (optional). Same rule: extend current fields first.

---

## `FORMAT_VERSION` bump rules

`FORMAT_VERSION` lives in `src/exchange.rs` and is **`4`**. Phase 3 added `resolutions` (1 → 2). Phase 6 added `events` (2 → 3). Phase 7 added markings, `data.audit`, and `data.members` (3 → 4) so a down-level binary refuses the file instead of dropping the governance slice. Version-1–3 archives still import (`resolutions` / `events` / `audit` / `members` default to empty; `marking` defaults to unmarked). Import still rejects any archive whose `format_version` is **greater** than the running binary.

Checked-in proof: `tests/fixtures/archives/v1-minimal/investigation.osint.json` (no `decided_by`, no `resolutions`, no `events`, no `audit`) plus `export_then_import_round_trips`, `export_import_preserves_resolution_bags`, `export_import_preserves_events_and_rename_remaps_them`, and `export_import_preserves_marking_and_audit_and_rename_remaps_them`. `--rename` remaps every record UUID, including `events.id`, `audit.id`, and `audit.target_ids`.

Cross-phase rule (roadmap): a format bump **and** import compatibility notes land in the same PR as the behavior change.

### Must bump

Bump `FORMAT_VERSION` (1 → 2 → 3 → 4, never reuse a number) when any of the following is true:

1. A required field is added, removed, renamed, or retyped on the manifest or on `InvestigationData` (or a nested record that has no default).
2. A new first-class collection must survive export → import (resolution bags, audit, first-class Event objects, structured markings). Silent drop by a v1 reader is not acceptable — the higher version makes old binaries refuse the file instead of losing data.
3. Path layout changes (`files/` tree, snapshot addressing, manifest name).
4. Semantics of an existing field change (for example `provenance` values, or `relationships.kind` ceasing to be a free string).
5. The `format` string changes.

### May stay at the current version

- New **optional** fields with `#[serde(default)]` that a reader can ignore without changing the meaning of existing rows (for example a new optional comment on a source).
- Additive open-enum keys (`EntityKind::Other`, new relationship verbs). Unknown kind strings already round-trip.
- Documentation-only or TUI-only changes.
- `EvidenceBatch` field additions that are not part of the archive. The batch file/sidecar envelope has its **own** version constant (`ingest::BATCH_FORMAT_VERSION`, currently 1). Bump that constant — not archive `FORMAT_VERSION` — when the envelope gains a required field.
- Playbook YAML (`os-int-playbook`, `PLAYBOOK_FORMAT_VERSION` 1) and SQLite `playbook_runs` / `automations` tables. Those are not archive collections. Newer playbook YAML is refused by the playbook parser; it does not bump investigation `FORMAT_VERSION`.

If an optional field is load-bearing (resolution, markings that must not be stripped, audit), bump anyway so a down-level binary does not import a silent subset and re-export it as complete.

### Compatibility notes required on every bump

In the same PR:

1. Update this page: new version number, what changed, how v1 archives still import.
2. Update [`docs/schema.md`](schema.md) “Export” and [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) “Change the on-disk export”.
3. Keep a fixture (or the existing `export_then_import_round_trips` test plus a checked-in v1 snippet) proving old archives load.
4. State in the phase PR whether `--rename` ID behavior changed.

### What not to do

- Do not set `#[serde(deny_unknown_fields)]` on `ArchiveManifest` or `InvestigationData` while version 1 is current; that blocks the optional-field path.
- Do not bump the version for vanity or for a field that is not read on import.
- Do not name or imply a third-party investigation-platform product in the format string. The format name stays `os-int-investigation`.

---

## See also

- [Ontology](ontology.md)
- [Playbooks](playbooks.md)
- [Database tables](schema.md)
- [Architecture — storage layout](ARCHITECTURE.md#storage-layout)
- [Examples — export and import](examples.md#write-a-report-and-hand-the-case-over)
- [User guide — reports, export, and import](guide.md#reports-export-and-import)
- [Collaboration](collaboration.md)
- [Vision](../VISION.md)
- [Phased roadmap](../PHASED_ROADMAP.md)
