# Architecture

How `os-int` is put together, and the decisions that shape it. Read this before making a structural change. Day-to-day use is documented in the [user guide](guide.md).

## Contents

- [Who this is for](#who-this-is-for)
- [Principles](#principles)
- [Layers](#layers)
- [Where to make a change](#where-to-make-a-change)
- [Data flow](#data-flow)
- [Storage layout](#storage-layout)
- [Agent integration](#agent-integration)
- [Decisions](#decisions)
- [Glossary](#glossary)

## Who this is for

Contributors. The document assumes you can read Rust and want to know which module owns a behavior before you edit it.

A useful order:

1. Skim [Principles](#principles) and [Where to make a change](#where-to-make-a-change).
2. Read the layer that matches your change.
3. If you are about to reverse something in [Decisions](#decisions), update this file in the same pull request.

## Principles

1. **os-int orchestrates information, tools, sources, and agents.** Built-in collectors cover the passive basics. Everything else plugs in through the external tool registry or an agent CLI.
2. **Provenance is a first-class concept.** Every observation is tied to a `Source`. Findings, entities, and relationships reference sources. Every record carries a `Provenance` (`collected`, `external`, `analyst`, `agent`) so AI inference is never silently promoted to fact.
3. **The UI thread never blocks on I/O.** HTTP, DNS, TLS, subprocesses, agent calls, HTML parsing, and report rendering run as background jobs on Tokio and report back through a bounded channel. SQLite writes stay on the UI task because they are small, transactional, and sub-millisecond for typical investigations. See [Decisions](#decisions).
4. **Untrusted content is data.** Everything collected from the network, files, repositories, or tool output is sanitized before it is rendered, and fenced as untrusted evidence before it is shown to an agent.
5. **Observation, not attack.** Built-in collectors perform ordinary DNS queries, RDAP lookups, single HTTP(S) fetches of user-specified targets, TLS handshakes, a public certificate-transparency search, and public repository inspection. Diagnostics describe published records (mail policy, response headers, certificate validity, DNSSEC, and `security.txt`). Gaps become collected findings and are removed when a later collection shows the check is in a good state. Tools marked `active` are refused. When a later collection of the same record has a new content hash, storage writes an "observation changed" timeline event.

## Layers

```text
                ┌──────────────────────────────────────────────┐
  terminal ───▶ │ tui/      event loop, rendering, overlays     │
                ├──────────────────────────────────────────────┤
                │ app/      App state, Actions, update logic    │  ◀── testable without a terminal
                ├──────────────┬───────────────┬───────────────┤
                │ jobs/        │ agent/        │ report/        │
                │ collectors/  │ tools/        │ exchange/      │
                ├──────────────┴───────────────┴───────────────┤
                │ storage/  SQLite (rusqlite) + file store      │
                ├──────────────────────────────────────────────┤
                │ model/    plain domain types (no Ratatui)     │
                └──────────────────────────────────────────────┘
```

| Module | Responsibility |
| --- | --- |
| `model/` | Domain types: `Investigation`, `Page`, `Source`, `Finding`, `Entity`, `Relationship`, `Note`, `Artifact`, `TimelineEvent`, `JobRecord`, `AgentRun`, `Proposal`. Object/link/Action registries and Object View data in `ontology`. Search Around and graph visibility in `graph`. Short references (`S-12`, `E-4`, …). No UI or storage code. |
| `storage/` | `Store` wraps a SQLite connection: migrations, CRUD, applying collector output transactionally, loading a full `Workspace`. Graph query (`Store::search_around`) runs the same hop expansion as `model::search_around` on a loaded snapshot. |
| `files.rs` | Investigation directory layout and content-addressed snapshot storage. |
| `collectors/` | `Collector` trait plus built-ins: DNS, RDAP, HTTP, TLS, certificate transparency, IP intelligence, repository, file. `diagnose` turns observations into structured readings. Collectors return a `CollectionOutput` and never touch the database. |
| `tools/` | External tool registry: `PATH` detection, built-in and user-defined tool definitions, argument templates. |
| `exec/` | The single place where subprocesses are spawned: no shell, explicit argv, timeouts, bounded output, process-group kill, cancellation, argument redaction for display and logs. |
| `agent/` | Agent profiles (Claude Code, Codex, generic), `AgentContext` and its prompt serialization, the streaming runner and output parsers, structured action proposals. |
| `jobs/` | `JobManager`: spawns tasks with cancellation tokens, a concurrency limit, and a bounded event channel. |
| `report/` | Markdown and JSON report generation. Renderers implement a small trait so HTML or PDF can be added later. |
| `exchange/` | Versioned, path-independent import and export of investigations. |
| `app/` | `App` owns config, store, workspace, job manager, and all UI state (including `GraphState`). Keys map to `Action`s; `App::dispatch` performs state transitions and side effects. Per-investigation `ui_state` restores the last view, page, and agent thread after restart. Long notes open `$EDITOR` / `$VISUAL` without a shell. |
| `tui/` | Terminal setup and restore, the async event loop, theme, keymap, widgets, screens, and overlays. Rendering is a pure function of `App`. |
| `cli/` | Clap definitions and non-TUI commands (`list`, `new`, `open`, `export`, `import`, `collect`, `report`, `doctor`, `config`). |

## Where to make a change

| You want to… | Start here |
| --- | --- |
| Add a field to a record | `model/`, then a migration in `storage/migrations.rs`, then the row mapping in `storage/` |
| Add a passive collector | Implement `Collector`, register it in `CollectorRegistry::builtin`, and document it in [guide.md](guide.md#collectors) |
| Add a diagnostic reading | `collectors/diagnose.rs`. Record gaps with `record_findings` so a later good reading clears them |
| Add a built-in external tool | `tools/registry.rs` (`builtin_tools`). Keep `activity` passive |
| Change how processes are spawned | `exec/` only. Callers pass an argv. They do not build shell strings |
| Change what the agent is allowed to propose | `model/agent.rs` (`AgentAction`), the Action catalog in `model/ontology.rs`, and the instructions in `agent/context.rs`. Reserved kinds stay refused on accept |
| Change an object type, property schema, or link verb | `model/ontology.rs` and [ontology.md](ontology.md). Prefer a JSON property bag field over a new column |
| Change a key binding | `tui/keymap.rs`, including `HELP_TEXT`, and the keyboard tables in the user guide |
| Change a screen | `tui/screens.rs`. Put the state change in `app/`, not in the widget. Object View is `View::Object`. Graph is `View::Graph` |
| Change Search Around or graph caps | `model/graph.rs` (pure query), then `storage/graph.rs` (`Store::search_around`), then `app/graph.rs` and `tui/screens.rs` |
| Add a palette command | `app/commands.rs`, then handle the id in `App::run_command` |
| Add a report section | `report/`. Keep AI-written text labelled as inference |
| Change the on-disk export | `exchange/`. Follow the bump rules in [ontology-exchange.md](ontology-exchange.md#format_version-bump-rules). Bump `FORMAT_VERSION` if older archives would no longer import, or if a new collection must survive round-trip |

Collectors, tools, and the TUI do not write SQLite. `storage::apply` is the path that turns a `CollectionOutput` into rows.

## Data flow

```text
key press ──▶ keymap ──▶ Action ──▶ App::dispatch ──┬─▶ Store (sync, small writes)
                                                   └─▶ JobManager::spawn ──▶ Tokio task
                                                                                │
   draw ◀── App (dirty) ◀── App::handle_job_event ◀── bounded mpsc ◀────────────┘
```

- The event loop (`tui::run`) `select!`s over terminal events, job events, an optional tick (only while jobs are running, for elapsed-time display), and `SIGTERM` / `SIGHUP`. It redraws only when state is marked dirty.
- Collector output is applied in one SQLite transaction: sources are inserted, entities are upserted by `(kind, canonical value)`, relationships are upserted by `(from, kind, to)`, and every link records its supporting source.
- A whole-workspace reload follows mutations. That keeps the UI consistent with the database. It is fast for investigations with thousands of records.

## Storage layout

```text
<data_dir>/                      platform data dir, or --data-dir
  os-int.sqlite3                 all structured data
  logs/os-int.log.YYYY-MM-DD     tracing output (never the TUI)
  cache/rdap/*.json              IANA RDAP bootstrap cache
  investigations/<uuid>/
    snapshots/ab/abcdef….bin     content-addressed raw HTTP bodies, RDAP JSON, …
    files/                       imported evidence files (content-addressed)
    agent/                       exact context packs sent to agents (audit trail)
    reports/                     generated reports
    repos/                       partial clones of public repositories
```

On Linux the platform data directory is `~/.local/share/os-int` and the config file is `~/.config/os-int/config.toml`. macOS and Windows use the `directories` crate paths documented in the [user guide](guide.md#where-files-live).

Database rows store **relative** paths inside the investigation directory, so an exported investigation can be imported on another machine unchanged.

## Agent integration

- An `AgentProfile` is configuration: program, argv template, adapter (output parser), how the prompt is delivered (`stdin`, argument, or file), working directory, and timeout. Profiles for Claude Code and Codex are built in. Any other CLI can be added in `config.toml` without recompiling.
- Placeholders (`{prompt}`, `{instructions}`, `{prompt_file}`, `{question}`, `{workdir}`) are substituted inside individual arguments. Arguments are never re-split and never pass through a shell.
- Claude Code runs as `claude -p --output-format stream-json --verbose --include-partial-messages --tools "" --no-session-persistence --system-prompt {instructions}` with the context pack on stdin. `--tools ""` disables every built-in tool, so prompt-injected content cannot make the agent run commands or read files.
- Codex runs as `codex exec --json --skip-git-repo-check --sandbox read-only --ephemeral`, with the prompt on stdin.
- The `AgentContext` is serialized to Markdown. Trusted os-int instructions are passed separately (system prompt) when the agent supports it. All collected material is wrapped in `<<<UNTRUSTED … nonce=…>>>` fences with a per-request random nonce, and any fence-like text inside the material is neutralized.
- The exact prompt is saved as an artifact for every run, so an analyst can audit what the agent saw.
- Agents may propose structured actions in a fenced `os-int-actions` JSON block: `ADD_FINDING`, `ADD_NOTE`, `CREATE_PAGE`, `LINK_ENTITY`, `RUN_COLLECTOR`, `GENERATE_REPORT`. Proposals are stored as pending and applied only after the analyst accepts them. Accepted AI findings keep `provenance = agent`.

## Decisions

| Decision | Rationale |
| --- | --- |
| `rusqlite` (bundled SQLite), synchronous, on the UI task | Simple, robust, portable. Writes are tiny. A storage actor can be introduced later without changing callers. |
| `hickory-resolver` for DNS instead of shelling out to `dig` | Works on every platform without external tools. `dig` remains available as an external tool. |
| RDAP via the IANA bootstrap registry (cached 7 days) | Queries the authoritative registry directly. |
| ASN data via Team Cymru's public DNS interface | Passive, free, no API key. |
| Manual redirect following | Records the full redirect chain and refuses redirects from public to private or loopback addresses. |
| Own text input widgets | Avoids locking the crate to a Ratatui version older than the one in use. `$EDITOR` is supported for long notes. |
| `aws-lc-rs` as the single rustls crypto provider | It is the default for the reqwest and rustls versions this crate uses. One provider avoids compiling two. |
| Short ids (`S-12`) are per-investigation sequence numbers. UUIDs are the primary keys | Humans and agents cite short ids. UUIDs keep import and export collision-free. |
| Whole-workspace reload after mutations | Keeps the UI consistent with the database with very little code. Fast for investigations with thousands of records. |
| Collectors return batches and do not touch SQLite | The CLI and the TUI share one apply path, and collectors stay testable without a database. |
| Subprocesses go through `exec/` | One place enforces "no shell", timeouts, output bounds, cancellation, and redaction. |

## Glossary

| Term | Meaning |
| --- | --- |
| Investigation | One case. The unit of export, import, and the agent context pack. |
| Page | A subject inside a case. May carry a collector target. |
| Source | One observation, with provenance, a retrieval time, and usually a content hash. |
| Entity | A normalized identifier. Upserted by kind plus canonical value. Graph nodes are entities. |
| Search Around | Hop expansion from a seed entity along relationships, with optional kind / link / provenance / time filters and render caps. |
| Finding | A claim. Collected diagnostics, analyst conclusions, and accepted agent inferences are all findings, distinguished by provenance. |
| Proposal | A structured agent action waiting for the analyst to accept or reject it. |
| Batch | The `EvidenceBatch` / `CollectionOutput` a collector returns. Applied in one transaction. |
| Short ref | `S-12` style id. Sequence number scoped to one investigation. |
| Activity | `passive` or `active`. Built-in collectors are passive. Tools marked `active` are refused. |

## See also

- [User guide](guide.md)
- [Examples](examples.md)
- [Database tables](schema.md)
- [Ontology](ontology.md)
- [Ontology Exchange Format](ontology-exchange.md)
- [Vision](../VISION.md)
- [Phased roadmap](../PHASED_ROADMAP.md)
- [Contributing](../CONTRIBUTING.md)
- [Security policy](../SECURITY.md)
