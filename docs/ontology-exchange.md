# Ontology Exchange Format

Two related payloads move investigation data in and out of os-int:

1. **Investigation archive** — the on-disk export written by `os-int export` (`investigation.osint.json` plus a `files/` tree). This is format `os-int-investigation`, version **`1`**.
2. **Evidence batch** — the in-memory (and, later, connector) payload collectors return. `storage::apply` writes it in one transaction. It is not versioned on disk today.

This page describes **today’s** shapes from [`src/exchange.rs`](../src/exchange.rs), [`src/model/batch.rs`](../src/model/batch.rs), and [`docs/schema.md`](schema.md), then the **reserved v2** extension points (Objects, Links, Events, Actions, Resolution bags) and the rules for bumping `FORMAT_VERSION`. v2 is **not implemented**. Do not emit the reserved keys yet.

The product thesis is in [VISION.md](../VISION.md). Phase gates are in [PHASED_ROADMAP.md](../PHASED_ROADMAP.md).

---

## Investigation archive (format version 1)

`os-int export` writes a directory:

```text
<destination>/
  investigation.osint.json    # ArchiveManifest
  files/                      # copy of the investigation directory (if it exists)
```

`exchange::MANIFEST_NAME` is `investigation.osint.json`. `exchange::FORMAT_VERSION` is `1`. Suggested directory name: `<sanitized-investigation-name>-<32 hex digits>` (`exchange::suggested_export_name` uses `Uuid::simple()`, no hyphens).

Import accepts either the directory or a path to the JSON file. When the path is a file, `files/` is taken as a sibling of that file.

### Manifest envelope

`ArchiveManifest` (`src/exchange.rs`):

| Field | Type | Today |
| --- | --- | --- |
| `format` | string | Always `os-int-investigation`. |
| `format_version` | `u32` | `1` (`exchange::FORMAT_VERSION`). |
| `exported_at` | RFC 3339 timestamp | Set at export time (`model::now()`, millisecond UTC). |
| `data` | object | `InvestigationData` — every record of one investigation. |

Paths inside `data` stay **relative** to the investigation directory. Export does not rewrite them to absolute paths on the originating machine.

### `data` (`InvestigationData`)

`InvestigationData` is also the body the workbench loads (`src/model/workspace.rs`). Join tables are flattened onto parent records except `page_entities`.

| Field | SQLite / notes |
| --- | --- |
| `investigation` | One case: `id`, `name`, `description`, `tags` (string array), `created_at`, `updated_at`. `name` is unique case-insensitively in a store. |
| `pages` | Subjects: `id`, `investigation_id`, `seq`, `title`, `kind`, `target`, `description`, `notes`, `entity_id`, timestamps. |
| `sources` | Observations. `snapshot_path` is relative. `content_hash` is SHA-256 hex of the raw snapshot. |
| `findings` | Claims. `source_ids` is the flattened `finding_sources` join. Empty `source_ids` is a hypothesis. |
| `entities` | Normalized identifiers. Unique on `(investigation_id, kind, value)`. `source_ids` is the flattened `entity_sources` join. |
| `relationships` | Directed edges. Unique on `(from_id, kind, to_id)`. `source_ids` is the flattened `relationship_sources` join. |
| `notes` | Standalone notes with their own provenance. Page notes live on `pages.notes`, not here. |
| `artifacts` | Files under the investigation directory. `path` is relative. |
| `timeline` | Point-in-time events (`timeline_events`). |
| `jobs` | Background work. `investigation_id` is optional on the struct; arguments are redacted. |
| `agent_runs` | Questions sent to a local agent CLI and the replies. |
| `proposals` | Structured `AgentAction` values with `pending` / `accepted` / `rejected` / `invalid`. |
| `page_entities` | `{ "page_id", "entity_id" }` links (`page_entities` table). A page’s `entity_id` is the primary subject and may be omitted from this list. |

**Not exported:** `ui_state` (last view, last page, agent thread). That is local workbench chrome, not case evidence.

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

Source kinds today: `dns`, `rdap`, `http`, `tls`, `ct`, `ip_intel`, `repository`, `file`, `command`, `web`, `document`, `manual`.

Artifact kinds today: `snapshot`, `file`, `report`, `agent_context`, `tool_output`, `repository`.

Timeline kinds today: `registration`, `expiration`, `updated`, `certificate_issued`, `certificate_expires`, `commit`, `release`, `published`, `observed`, `changed`, `user`, `finding`, `investigation`.

Built-in relationship verbs (`src/model/relationship.rs` `kinds`) include `resolves_to`, `alias_of`, `uses_nameserver`, `mail_handled_by`, `spf_includes`, `reverse_dns`, `announced_by`, `operated_by`, `registered_with`, `registered_to`, `redirects_to`, `serves`, `links_to`, `mentions`, `presents_certificate`, `issued_by`, `covers`, `subdomain_of`, `references`, `contributed_to`, `hosted_on`, `associated_with`, `contains`. Other `snake_case` verbs are stored as-is.

Agent actions today: `ADD_FINDING`, `ADD_NOTE`, `CREATE_PAGE`, `LINK_ENTITY`, `RUN_COLLECTOR`, `GENERATE_REPORT`. Reserved future kinds (`PROPOSE_RESOLVE`, `PROPOSE_UNRESOLVE`, playbook steps) are not in the enum yet.

### Import behavior (v1)

From `exchange::import_investigation`:

1. Parse the manifest. If `format_version` **>** `FORMAT_VERSION`, reject. Older or equal versions are accepted.
2. Reject artifact `path` or source `snapshot_path` values that are not safe relative paths (`paths::is_safe_relative_path`).
3. If an investigation with the same `id` exists: reject unless `--rename`. `--rename` assigns a new investigation UUID, suffixes the name with ` (imported)`, and rewrites `investigation_id` on child rows. **Record UUIDs are not rewritten.**
4. If the name already exists (case-insensitive), append ` (2)`, ` (3)`, … or a UUID suffix.
5. Insert `InvestigationData`, then copy `files/` into the new investigation directory.

Copy skips `.`, `..`, and names starting with `tmp-`. Directory depth is capped at 32. All symlinks are skipped (`DirEntry::metadata` does not follow them, so they are neither `is_dir` nor `is_file`). The cycle check is only for overlapping real directories (`canonicalize` plus a prefix test) before recursing.

### File tree

`files/` is a copy of `<data_dir>/investigations/<uuid>/` (snapshots, imported files, agent context packs, reports, repo clones). Database rows point at those relative paths. See [Architecture — storage layout](ARCHITECTURE.md#storage-layout).

---

## Evidence batch (collector / future connector payload)

Collectors do not write SQLite. They return an `EvidenceBatch` (`src/model/batch.rs`). `storage::apply` applies it atomically (`docs/schema.md` “How a collection lands”).

This is the ingest shape Phase 4 connectors will emit as JSON. It is **not** `investigation.osint.json`. It has **no** `FORMAT_VERSION` today. When it becomes a public on-disk / sidecar format, give it its own version field — do not overload the archive `FORMAT_VERSION`.

| Field | Role |
| --- | --- |
| `sources` | `NewSource` rows: `kind`, `title`, `url`, `retrieved_at`, `published_at`, `author`, `collector`, `content_hash`, `snapshot_path`, `content`, `fields`, `provenance`. |
| `entities` | `NewEntity`: `key` (`kind` + canonical `value`), `label`, `fields`, `sources` (indexes into `sources`), `link_to_page`. |
| `relationships` | `NewRelationship`: `from` / `to` (`EntityKey`), `kind`, `sources` (indexes), `confidence`. |
| `timeline` | `NewTimelineEvent`: `occurred_at`, `kind`, `title`, `description`, `source`, `entity`. |
| `artifacts` | `NewArtifact`: `kind`, `name`, relative `path`, `mime`, `size_bytes`, `sha256`, `fields`, `source`. |
| `findings` | `NewCollectedFinding`: `title`, `details`, `confidence`, `source`. Provenance is always `collected` when applied. |
| `resolved_findings` | Titles of collected findings that no longer apply (a later `present` reading). |
| `summary` | One-line human summary for the job result. |

`SourceKey` is a `usize` index into `sources`, not a UUID. Entity identity in a batch is `EntityKey` (kind + normalized value), not a UUID. Apply upserts entities on `(kind, canonical value)` and relationships on `(from, kind, to)`. A source with the same collector, title, URL, and content hash is marked seen again instead of inserted (`find_duplicate_source`). A new hash of a prior observation (same collector, title, and URL) inserts a `changed` timeline event.

`EvidenceBatch::is_empty` is true only when `sources`, `entities`, and `timeline` are all empty (relationships / artifacts / findings alone do not count).

---

## Today → intended v2 ontology

v1 already stores a cited graph. Later phases promote that graph to first-class ontology types. Map existing fields; do not invent a second parallel model.

| v2 concept | Phase | v1 field(s) to extend | Reserved addition (not shipped) |
| --- | --- | --- | --- |
| **Object** | 1 | `data.entities`, `data.pages` | Typed property schemas on `kind`; required/optional fields documented in `docs/ontology.md`. Prefer JSON property bags over new columns. Keep UUIDs and `seq`. |
| **Link** | 1 | `data.relationships` (`kind` is a free `snake_case` string) | Catalog metadata: description, inverse, allowed endpoint kinds. Unknown verbs still store, grouped as “Other”. |
| **Event** | 6 | `data.timeline` (instant `occurred_at`) | Interval or instant Event objects; links from entities to events; co-occurrence. Do not replace `timeline` without a bump + import of v1 events. |
| **Action** | 1, 5 | `data.proposals` + `AgentAction` | Catalog of action kinds (existing six plus reserved `PROPOSE_RESOLVE` / `PROPOSE_UNRESOLVE` / playbook steps). Audit fields (who / when). Automations propose; they do not silently write. |
| **Resolution bag** | 3 | *(none)* | Canonical object + constituents; property writes tracked for unresolve. Never auto-merge. |
| **Markings** | 7 | `investigation.tags` (free strings) | TLP (or confidentiality labels) on the investigation and optionally on objects. Visible in the UI and the manifest. |
| **Audit** | 7 | *(none; jobs/proposals are not an audit log)* | Append-only merge / accept / playbook / export / marking events. |

Ingest (Phase 4) still lands through `EvidenceBatch` → `storage::apply`. External connectors should emit the batch JSON, not a hand-built SQLite file.

Reserved **`data` keys** for a version-2 archive (do not write them at `format_version` 1):

- `objects` — only if entities/pages are insufficient and a parallel collection is required
- `links` — only if relationships need a parallel catalogued collection
- `events` — first-class Event objects (Phase 6)
- `actions` — if Actions outgrow `proposals`
- `resolutions` — resolution bags (Phase 3)
- `markings` — structured markings (Phase 7)
- `audit` — audit slice (Phase 7)

Prefer **evolving the existing arrays** (add optional fields with `#[serde(default)]`) over forking new collections. Add a new collection only when the old row cannot carry the semantics (resolution bags, audit).

Reserved **`EvidenceBatch` keys** for a future connector protocol: `objects`, `links`, `events`, `actions`, `resolutions`. Same rule: extend current fields first.

---

## `FORMAT_VERSION` bump rules

`FORMAT_VERSION` lives in `src/exchange.rs`. Import rejects any archive whose `format_version` is **greater** than the running binary. Older archives must keep importing.

Cross-phase rule (roadmap): a format bump **and** import compatibility notes land in the same PR as the behavior change.

### Must bump

Bump `FORMAT_VERSION` (1 → 2, never reuse a number) when any of the following is true:

1. A required field is added, removed, renamed, or retyped on the manifest or on `InvestigationData` (or a nested record that has no default).
2. A new first-class collection must survive export → import (resolution bags, audit, first-class Event objects, structured markings). Silent drop by a v1 reader is not acceptable — the higher version makes old binaries refuse the file instead of losing data.
3. Path layout changes (`files/` tree, snapshot addressing, manifest name).
4. Semantics of an existing field change (for example `provenance` values, or `relationships.kind` ceasing to be a free string).
5. The `format` string changes.

### May stay at the current version

- New **optional** fields with `#[serde(default)]` that a reader can ignore without changing the meaning of existing rows (for example a new optional comment on a source).
- Additive open-enum keys (`EntityKind::Other`, new relationship verbs). Unknown kind strings already round-trip.
- Documentation-only or TUI-only changes.
- `EvidenceBatch` field additions that are not part of the archive. When the batch becomes a public file format, give it a **separate** version constant.

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

- [Database tables](schema.md)
- [Architecture — storage layout](ARCHITECTURE.md#storage-layout)
- [Examples — export and import](examples.md#write-a-report-and-hand-the-case-over)
- [User guide — reports, export, and import](guide.md#reports-export-and-import)
- [Vision](../VISION.md)
- [Phased roadmap](../PHASED_ROADMAP.md)
