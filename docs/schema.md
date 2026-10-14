# Database tables

os-int keeps structured records in one SQLite file, `os-int.sqlite3`, in the data directory. Raw HTTP bodies, reports, agent context packs, and repository clones live as files under `investigations/<uuid>/` and are referenced by a relative path. `os-int config` prints both locations.

This page is the map of those tables. The column list that ships starts at `V1` in [`src/storage/migrations.rs`](../src/storage/migrations.rs). `V2` adds Action audit (`proposals.decided_by`) and the per-investigation `link_types` catalog. Migrations are append-only: add a new version, and do not edit a migration that has already shipped.

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
| `investigations` | One case. `name` is unique, case-insensitively. `description` is the report's executive summary. `tags` is a JSON array. |
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
| `timeline_events` | Something that happened at `occurred_at` (a registration date, a certificate expiry, an observation change). |
| `jobs` | One background task: collector, tool, agent call, or report. Arguments stored here are redacted. |
| `agent_runs` | One question and the agent's response, including the thread id used for follow-ups. |
| `proposals` | One structured action an agent suggested. `status` stays pending until you accept or reject it. `decided_at` / `decided_by` record who and when (local user for now). |
| `link_types` | First-seen relationship verbs in an investigation. Built-in catalog metadata is copied; custom snake_case verbs are stored with an empty description. Rebuilt from `relationships` on import. Not part of the archive. |
| `ui_state` | Small key/value pairs: the last investigation, and per-investigation view, page, entity, and agent thread. `scope` is `global` or an investigation id. |

## How a collection lands

Collectors do not write these tables. They return an `EvidenceBatch`. `storage::apply` writes the batch in one transaction:

1. Insert sources. A source with the same collector, title, URL, and content hash is marked seen again instead of inserted (`find_duplicate_source`).
2. Upsert entities on `(kind, canonical value)` and attach them to the page.
3. Upsert relationships on `(from, kind, to)` and record the supporting source.
4. Sync collected findings: a `missing` or `review` diagnostic is stored, and a later `present` reading clears that finding.
5. When the content hash of a prior observation changed, insert a timeline event of kind `changed`.

## Provenance and confidence

`provenance` is one of `collected`, `external`, `analyst`, `agent`. `confidence` is one of `unknown`, `low`, `medium`, `high`, `confirmed`. Both are stored as those words. The workbench shows provenance as `[COL]`, `[EXT]`, `[ANL]`, and `[AI]`.

## Graph queries

Search Around does not add a table. `Store::search_around` loads the investigation snapshot and runs `model::search_around` over `entities` and `relationships`. Facet counts are derived from that result. Render caps live in the query, not in SQLite.

## Export

`os-int export` writes `investigation.osint.json` plus a `files/` tree. The JSON document's `data` object is the same shape as `InvestigationData`: the investigation and the rows above, with paths still relative. Import rejects an archive whose investigation id is already present unless you pass `--rename`.

Format name: `os-int-investigation`. Format version: `1` (`exchange::FORMAT_VERSION`). `proposals.decided_by` is optional (`#[serde(default)]`); version-1 archives still import. Field list, EvidenceBatch ingest shape, reserved later-version keys, and bump rules: [Ontology Exchange Format](ontology-exchange.md). Object types and link verbs: [Ontology](ontology.md).

## See also

- [Ontology Exchange Format](ontology-exchange.md)
- [Architecture — storage layout](ARCHITECTURE.md#storage-layout)
- [User guide — where files live](guide.md#where-files-live)
- [Examples — export and import](examples.md#write-a-report-and-hand-the-case-over)
