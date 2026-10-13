# os-int architecture

This document describes how `os-int` is put together and records the design
decisions that shape it. It is written for contributors: read it before making
structural changes.

## Guiding principles

1. **os-int orchestrates information, tools, sources, and agents.** It does not
   reimplement every OSINT utility. Built-in collectors cover the passive basics;
   everything else plugs in through the external tool registry or an agent CLI.
2. **Provenance is a first-class concept.** Every observation is tied to a
   `Source`. Findings, entities, and relationships reference sources. Every
   record carries a `Provenance` (`collected`, `external`, `analyst`, `agent`) so
   AI inference is never silently promoted to fact.
3. **The UI thread never blocks on I/O.** HTTP, DNS, TLS, subprocesses, agent
   calls, HTML parsing, and report rendering run as background jobs on Tokio and
   report back through a bounded channel. SQLite writes stay on the UI task
   because they are small, transactional, and sub-millisecond for typical
   investigations (documented trade-off; see "Decisions").
4. **Untrusted content is data, never instructions.** Everything collected from
   the network, files, repositories, or tool output is sanitized before it is
   rendered and is fenced as untrusted evidence before it is shown to an agent.
5. **Passive by default.** Built-in collectors only perform ordinary DNS
   queries, RDAP lookups, single HTTP(S) fetches of user-specified targets, TLS
   handshakes, and public repository inspection. Tools marked `active` require
   explicit confirmation.

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
| `model/` | Domain types: `Investigation`, `Page`, `Source`, `Finding`, `Entity`, `Relationship`, `Note`, `Artifact`, `TimelineEvent`, `JobRecord`, `AgentRun`, `Proposal`. Short references (`S-12`, `E-4`, …). No UI or storage code. |
| `storage/` | `Store` wraps a SQLite connection: migrations, CRUD, applying collector output transactionally, loading a full `Workspace`. |
| `files.rs` | Investigation directory layout and content-addressed snapshot storage. |
| `collectors/` | `Collector` trait plus built-ins: DNS, RDAP, HTTP, TLS, IP intelligence, repository, file. Collectors return a `CollectorOutput` (sources, entities, relationships, timeline events, artifacts) and never touch the database. |
| `tools/` | External tool registry: PATH detection, built-in and user-defined tool definitions, argument templates. |
| `exec/` | The single place where subprocesses are spawned: no shell, explicit argv, timeouts, bounded output, process-group kill, cancellation, argument redaction for display/logs. |
| `agent/` | Agent profiles (Claude Code, Codex, generic), `AgentContext` and its prompt serialization, the streaming runner and output parsers, structured action proposals. |
| `jobs/` | `JobManager`: spawns tasks with cancellation tokens, a concurrency limit, and a bounded event channel. |
| `report/` | Markdown and JSON report generation. Renderers implement a small trait so HTML/PDF can be added later. |
| `exchange/` | Versioned, path-independent import/export of investigations. |
| `app/` | `App` owns config, store, workspace, job manager, and all UI state. Keys map to `Action`s; `App::dispatch` performs state transitions and side effects. Per-investigation `ui_state` restores the last view, page, and agent thread after restart. Long notes open `$EDITOR` / `$VISUAL` without a shell. |
| `tui/` | Terminal setup/restore, the async event loop, theme, keymap, widgets, screens, and overlays. Rendering is a pure function of `App`. |
| `cli/` | Clap definitions and non-TUI commands (`list`, `new`, `export`, `import`, `collect`, `report`, `doctor`, `config`). |

## Data flow

```text
key press ──▶ keymap ──▶ Action ──▶ App::dispatch ──┬─▶ Store (sync, small writes)
                                                   └─▶ JobManager::spawn ──▶ Tokio task
                                                                                │
   draw ◀── App (dirty) ◀── App::handle_job_event ◀── bounded mpsc ◀────────────┘
```

* The event loop (`tui::run`) `select!`s over terminal events, job events, an
  optional tick (only while jobs are running, for elapsed-time display), and
  SIGTERM/SIGHUP. It redraws only when state is marked dirty.
* Collector output is applied in one SQLite transaction: sources are inserted,
  entities are upserted by `(kind, canonical value)`, relationships are
  upserted by `(from, kind, to)`, and every link records its supporting source.

## Storage layout

```text
<data_dir>/                      platform data dir (e.g. ~/.local/share/os-int)
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

Database rows store **relative** paths inside the investigation directory, so
an exported investigation can be imported on another machine unchanged.

## Agent integration

* An `AgentProfile` is configuration: program, argv template, adapter (output
  parser), how the prompt is delivered (stdin, argument, or file), working
  directory, timeout. Profiles for Claude Code and Codex are built in; any
  other CLI can be added in `config.toml` without recompiling.
* Placeholders (`{prompt}`, `{instructions}`, `{prompt_file}`, `{question}`,
  `{workdir}`) are substituted *inside* individual arguments. Arguments are
  never re-split and never pass through a shell.
* Claude Code runs as
  `claude -p --output-format stream-json --verbose --include-partial-messages --tools "" --no-session-persistence --system-prompt {instructions}`
  with the context pack on stdin. `--tools ""` disables every built-in tool, so
  prompt-injected content cannot make the agent run commands or read files.
* The `AgentContext` is serialized to Markdown. Trusted os-int instructions are
  passed separately (system prompt) when the agent supports it. All collected
  material is wrapped in `<<<UNTRUSTED … nonce=…>>>` fences with a per-request
  random nonce, and any fence-like text inside the material is neutralized.
* The exact prompt is saved as an artifact for every run, so an analyst can
  audit what the agent saw.
* Agents may propose structured actions in an ```` ```os-int-actions ```` JSON
  block (`ADD_FINDING`, `ADD_NOTE`, `CREATE_PAGE`, `LINK_ENTITY`,
  `RUN_COLLECTOR`, `GENERATE_REPORT`). Proposals are stored as pending and are
  applied only after the analyst accepts them. Accepted AI findings keep
  `provenance = agent`.

## Decisions

| Decision | Rationale |
| --- | --- |
| `rusqlite` (bundled SQLite), synchronous, on the UI task | Simple, robust, portable; writes are tiny. A storage actor can be introduced later without changing callers. |
| `hickory-resolver` for DNS instead of shelling out to `dig` | Works on every platform without external tools; `dig` remains available as an external tool. |
| RDAP via the IANA bootstrap registry (cached 7 days) | Queries the authoritative registry directly instead of a third-party redirector. |
| ASN data via Team Cymru's public DNS interface | Passive, free, no API key. |
| Manual redirect following | Records the full redirect chain and refuses redirects from public to private/loopback addresses. |
| Own text input widgets instead of `tui-textarea` | `tui-textarea` targets an older Ratatui; a small internal widget avoids version lock. `$EDITOR` is supported for long notes. |
| `aws-lc-rs` as the single rustls crypto provider | It is the default for reqwest 0.13 and rustls 0.23; using one provider avoids compiling two. |
| Short IDs (`S-12`) are per-investigation sequence numbers; UUIDs are the primary keys | Humans and agents cite short IDs; UUIDs keep import/export collision-free. |
| Whole-workspace reload after mutations | Keeps the UI consistent with the database with very little code; fast for investigations with thousands of records. |
