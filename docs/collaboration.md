# Collaboration, markings, and audit

os-int is local-first. A single analyst on one machine is the default. Phase 7 adds **markings**, an **append-only audit log**, and an optional **shared data directory** with `owner` / `analyst` / `reader` roles. There is no cloud service and no requirement to be online.

Threat model: the trust boundary is the people who can read the data directory (or an exported archive). os-int does not encrypt the SQLite file, does not authenticate over a network, and does not stop someone with filesystem access from copying the case. Roles are a coordination aid for a small cell sharing a disk or mount, not a DRM system.

## Markings

Each investigation has a Traffic Light Protocol marking: `unmarked`, `TLP:CLEAR`, `TLP:GREEN`, `TLP:AMBER`, `TLP:AMBER+STRICT`, or `TLP:RED`. It is shown on Overview, in the title bar, in `os-int list`, and on `investigation` inside `investigation.osint.json`.

Objects may carry an optional marking in `entities.fields.marking` (`os-int marking set … --entity E-1`). Missing means unmarked.

Only the **owner** can change the investigation marking. Analysts may set object markings.

```bash
os-int marking get "Example Corp"
os-int marking set "Example Corp" TLP:AMBER
os-int marking set "Example Corp" TLP:GREEN --entity E-4
```

The palette command **Set marking** cycles the same labels.

## Audit log

`audit_log` is append-only (SQLite trigger refuses `UPDATE`). Rows record:

| Kind | When |
| --- | --- |
| `entities_merged` / `entities_unresolved` | Manual or accepted merge / split |
| `action_accepted` | An analyst accepts an Action proposal |
| `playbook_run` | A playbook apply (not dry-run) |
| `export` | `os-int export` of an archive (writers only; readers may still export the existing slice) |
| `marking_changed` | Investigation or object marking |
| `member_added` / `member_role_changed` / `member_removed` | Team changes |

Each row stores `actor` (`[identity] user_id`) and `machine` (`[identity] machine_name`). Export includes `data.audit`. The Audit screen and `os-int audit` list it. This log is **not** the investigative Timeline.

```bash
os-int audit "Example Corp"
os-int audit "Example Corp" --kind export
```

## Identity

```toml
[identity]
user_id = "analyst"
machine_name = "workstation"
```

Empty values fall back to `$USER` / `$USERNAME` and the hostname (`local` / `unknown` if those are missing). Configure this before sharing a data directory so audit rows are distinguishable.

## Shared data directory

Point two workbenches at the same `--data-dir` (or `storage.data_dir` / `OS_INT_DATA_DIR`). SQLite WAL plus a 5 second busy timeout serializes writers. **Conflict policy: last-write-wins with audit** (`model::CONFLICT_POLICY`). Concurrent updates to different rows both land. Concurrent updates to the same row: the later commit wins; both attempts that succeeded are visible in the audit log if they were audited operations.

There is no sync server. Copying `os-int.sqlite3` plus `investigations/` is the same as sharing the directory. **Export as handoff** is the offline path: `os-int export` writes markings and the audit slice; `os-int import` loads them. Format version **4**.

### Roles

| Role | Read / export | Collect, merge, accept, playbook | Change TLP / team |
| --- | --- | --- | --- |
| `owner` | yes | yes | yes |
| `analyst` | yes | yes | no |
| `reader` | yes | no | no |

An investigation with **no members** treats every local identity as owner (single-user path). The first `os-int team add` seeds the current identity as owner, then adds the new member.

```bash
os-int team list "Example Corp"
os-int team add "Example Corp" bob --role analyst
os-int team set-role "Example Corp" bob --role reader
os-int team remove "Example Corp" bob
```

Unlisted users on a case that already has members are **readers**.

## What this is not

- Not a multi-tenant server and not an enterprise investigation platform ACL replica.
- Built-in collectors stay passive. Active tools stay refused. AI still only proposes; accept is an audited Action.
- Provenance on findings, entities, and sources is unchanged.

## See also

- [User guide — markings, audit, and team](guide.md#markings-audit-and-team)
- [Ontology Exchange Format](ontology-exchange.md)
- [Database tables](schema.md)
- [Architecture](ARCHITECTURE.md)
