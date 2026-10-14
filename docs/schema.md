# Database tables

os-int keeps structured records in one SQLite file, `os-int.sqlite3`, in the data directory. Raw HTTP bodies, reports, agent context packs, and repository clones live as files under `investigations/<uuid>/` and are referenced by a relative path. `os-int config` prints both locations.

This page is the map of those tables. The column list that ships starts at `V1` in [`src/storage/migrations.rs`](../src/storage/migrations.rs). `V2` adds Action audit (`proposals.decided_by`) and the per-investigation `link_types` catalog. `V3` adds `resolutions` (canonical + constituents, reversible property/source writes). `V4` adds `playbook_runs` and `automations` (local workbench rows; not exported). `V5` adds `events` and `event_entities` (first-class instants and intervals). `V6` adds `investigations.marking`, append-only `audit_log`, and `investigation_members`. Migrations are append-only: add a new version, and do not edit a migration that has already shipped.

Every investigation-scoped table has:

| Column | Role |
| --- | --- |
| `id` | UUID primary key. Stable across export and import. |
| `investigation_id` | Owning investigation. Deleted with the investigation (`ON DELETE CASCADE`). |
| `seq` | Per-investigation sequence number. This is the short reference (`S-12` is `seq = 12` on `sources`). |

`seq` is unique per table per investigation. Short references are not unique across tables: `S-1` and `E-1` are different records.

## Tables

| Table | What a row is |
| --- | --- |
| `investigations` | One case. `name` is unique, case-insensitively. `description` is the report's executive summary. `tags` is a JSON array. `marking` is `unmarked` or a `tlp:*` key (default `unmarked`). |
| `pages` | A subject you are working: title, page kind, optional canonical target, notes, and the primary entity. |
| `entities` | A normalized identifier. Unique on `(investigation_id, kind, value)`, so `Example.COM.` and `example.com` are one row. |
| `sources` | One observation: kind, title, URL, retrieval time, collector id, content hash, relative snapshot path, sanitized text, and a JSON field list. |
| `findings` | A claim: title, details, confidence, provenance. |
| `finding_sources` | Sources cited by a finding. |
| `entity_sources` | Sources that support an entity. |
| `relationships` | A directed edge: `from_id`, `to_id`, snake_case `kind`, provenance, confidence. Unique on `(from_id, kind, to_id)`. |
| `relationship_sources` | Sources that support a relationship. |
| `page_entities` | Entities that appear on a page. |
| `notes` | A standalone note with its own provenance. Page notes are the `notes` column on `pages`, not a row here. |
| `artifacts` | A file in the investigation directory: kind, relative path, MIME type, size, SHA-256. |
| `timeline_events` | Something that happened at `occurred_at` (a registration date, a certificate expiry, an observation change). Short ref `T-n`. |
| `events` | First-class Event objects: `started_at`, optional `ended_at`, kind, title, provenance, fields. Unique on `(investigation_id, kind, started_at, title)`. Short ref `C-n`. |
| `event_entities` | Entities linked to an Event. |
| `jobs` | One background task: collector, tool, agent call, or report. Arguments stored here are redacted. |
| `agent_runs` | One question and the agent's response, including the thread id used for follow-ups. |
| `proposals` | One structured action an agent suggested. `status` stays pending until you accept or reject it. `decided_at` / `decided_by` record who and when (local user for now). |
| `link_types` | First-seen relationship verbs in an investigation. Built-in catalog metadata is copied; custom snake_case verbs are stored with an empty description. Rebuilt from `relationships` on import. Not part of the archive. |
| `resolutions` | One merge bag: canonical entity, constituent UUIDs, status (`active` / `unresolved`), actor, reason, property writes, and source citations copied onto the canonical. Constituent entity rows are not deleted. `canonical_id` is a real foreign key. `constituent_ids` is a JSON array of UUIDs, so SQLite / `PRAGMA foreign_key_check` / `os-int doctor` cannot see orphans inside it. |
| `ui_state` | Small key/value pairs: the last investigation, and per-investigation view, page, entity, and agent thread. `scope` is `global` or an investigation id. |
| `playbook_runs` | One playbook execution. Local workbench chrome, like `ui_state`. Not exported. |
| `automations` | Watch-set rows: playbook id, seed, last digest, optional interval. Local; not exported. |
| `audit_log` | Append-only governance events (merge, accept, playbook, export, marking, membership). Short ref `L-n`. `UPDATE` is refused by a trigger. Deleted only with the investigation. |
| `investigation_members` | `user_id` + `role` (`owner` / `analyst` / `reader`). Empty means single-user owner. |

## How a collection lands

Collectors, file ingest, and external sidecars do not write these tables. They return an `EvidenceBatch`. `storage::apply` writes the batch in one transaction:

1. Insert sources. A source with the same collector, title, URL, and content hash is marked seen again instead of inserted (`find_duplicate_source`).
2. Upsert entities on `(kind, canonical value)` and attach them to the page.
3. Upsert relationships on `(from, kind, to)` and record the supporting source.
4. Sync collected findings: a `missing` or `review` diagnostic is stored, and a later `present` reading clears that finding.
5. When the content hash of a prior observation changed, insert a timeline log row of kind `changed` and a first-class Event with `change_intelligence=observation_hash`. New collected diagnostics and clears also promote Events (`diagnostic_open` / `diagnostic_clear`).
6. Insert batch `events` (instants or intervals) and link `event_entities`.

## Provenance and confidence

`provenance` is one of `collected`, `external`, `analyst`, `agent`. `confidence` is one of `unknown`, `low`, `medium`, `high`, `confirmed`. Both are stored as those words. The workbench shows provenance as `[COL]`, `[EXT]`, `[ANL]`, and `[AI]`.

## Graph queries

Search Around does not add a table. `Store::search_around` loads the investigation snapshot and runs `model::search_around` over `entities` and `relationships`. Facet counts are derived from that result. Render caps live in the query, not in SQLite.

## Export

`os-int export` writes `investigation.osint.json` plus a `files/` tree. The JSON document's `data` object is the same shape as `InvestigationData`: the investigation and the rows above, with paths still relative. Import rejects an archive whose investigation id is already present unless you pass `--rename`.

Format name: `os-int-investigation`. Format version: `4` (`exchange::FORMAT_VERSION`). Version 2 added `data.resolutions`. Version 3 added `data.events`. Version 4 adds `investigation.marking`, `data.audit`, and `data.members`. `proposals.decided_by`, `resolutions`, `events`, `audit`, `members`, and `marking` are optional (`#[serde(default)]`); version-1–3 archives still import. `--rename` remaps Event and audit UUIDs and `entity_ids` / `target_ids`. File ingest uses a **separate** envelope (`os-int-evidence-batch`, `format_version` 1) and does not bump the archive version. Optional batch `events` is additive. Playbook YAML uses a third envelope (`os-int-playbook`, `format_version` 1); `playbook_runs` / `automations` stay local like `ui_state` and do not bump the archive version. Timeline kind `playbook_run` is an additive open-enum key. Field list, EvidenceBatch ingest shape, reserved later-version keys, and bump rules: [Ontology Exchange Format](ontology-exchange.md). Connectors: [connectors.md](connectors.md). Object types and link verbs: [Ontology](ontology.md). Playbooks: [playbooks.md](playbooks.md). GeoJSON is a separate export (`os-int export --format geojson` or `os-int geojson`), not this archive.

## See also

- [Ontology Exchange Format](ontology-exchange.md)
- [Playbooks](playbooks.md)
- [Collaboration](collaboration.md)
- [Architecture — storage layout](ARCHITECTURE.md#storage-layout)
- [User guide — where files live](guide.md#where-files-live)
- [Examples — export and import](examples.md#write-a-report-and-hand-the-case-over)
