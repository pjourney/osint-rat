# os-int

An open-source intelligence investigation workbench for the terminal.

os-int lets you create an investigation, collect public information, organize evidence with provenance, inspect entities and relationships, run passive OSINT tools, take notes, ask a locally installed AI agent CLI (Claude Code, Codex, or any other command), and generate reports — without leaving the terminal.

It orchestrates information, tools, sources, and agents. It does not try to reimplement every OSINT utility.

```text
┌ os-int ─ Investigation: Example Corp ───────────── Claude ● ┐
│ Navigate     │ P-1  example.com  Domain                     │
│              │ Target  example.com                          │
│ Overview     │                                              │
│ Pages        │ Entities on this page                        │
│ Sources      │   E-1  Domain      example.com               │
│ Findings     │   E-4  IP          93.184.216.34             │
│ Entities     │                                              │
│ Timeline     │ Findings                                     │
│ Agent        │   F-1  [COL]  HTTPS redirects to apex        │
│ Jobs         │                                              │
├──────────────┴──────────────────────────────────────────────┤
│ [a] Ask  [/] Search  [p] Palette  [r] Collect  [?] Help     │
└─────────────────────────────────────────────────────────────┘
```

*(Screenshot placeholder — run `os-int` in a 120×40 terminal for the live layout.)*

## Installation

Requires Rust 1.88 or newer (the crate edition is 2024).

```bash
git clone https://github.com/pjourney/osint-rat.git
cd osint-rat
cargo install --path .
```

The binary is named `os-int`. On first launch it writes a commented `config.toml` and a SQLite database under the platform data directory (`~/.local/share/os-int` on Linux).

Override locations with `--config`, `--data-dir`, `OS_INT_CONFIG`, or `OS_INT_DATA_DIR`.

## First-run workflow

```text
$ os-int
  n          New Investigation → Example Corp
  n          New Page → example.com  (type: Domain)
  r          Run collectors (DNS, RDAP, HTTP, TLS)
  5          Open Entities
  a          Ask the agent about a highlighted IP
  p          Command palette → Generate Report
```

Collection works without an AI agent. If Claude Code or Codex is on `PATH`, press `a` from any investigation screen.

## Features

- Investigations, pages, sources, findings, entities, relationships, notes, artifacts, timeline
- Provenance on every record: collected, external, analyst, or AI inference (never silently promoted to fact)
- Short references (`S-12`, `E-4`, `F-1`) shared by the UI, agents, and reports
- Built-in passive collectors: DNS, RDAP, HTTP, TLS, certificate transparency, IP intelligence, public repositories, files
- Diagnostics on public records: SPF, DMARC, MX, MTA-STS, BIMI, HTTP response headers, and certificate validity
- Re-collection records a timeline event when a public observation changes
- External tool registry (`whois`, `dig`, `gh`, custom passive TOML tools) — no shell, explicit argv. Active probes are refused
- Background jobs with cancellation; the UI never blocks on I/O
- Local agent CLIs: Claude Code (`claude -p --output-format stream-json --tools ""`), Codex, or a generic command
- Agent context packs fence untrusted evidence; proposed actions are previewed before they mutate state
- Markdown and JSON reports; portable investigation import/export
- `os-int doctor` for environment checks useful in bug reports

## CLI

```text
os-int                         # TUI
os-int new "Example Corp"
os-int open "Example Corp"
os-int list
os-int collect domain example.com
os-int collect domain example.com --investigation "Example Corp"
os-int collect example.com --collector dns
os-int collect example.com --collector ct
os-int report "Example Corp" --format markdown
os-int export "Example Corp" -o ./export
os-int import ./export
os-int doctor
os-int --debug                 # file logs at debug level
```

## AI agent integration

os-int never talks to an LLM API. It runs the agent you already have installed as a subprocess.

Default profile: Claude Code. `claude -p` is invoked non-interactively with `--tools ""` so prompt-injected web content cannot make the agent run commands. The investigation context is written to disk (audit trail) and passed on stdin.

Configure another agent in `config.toml`:

```toml
[agent]
active = "codex"

# or a generic CLI:
# [agents.opencode]
# name = "opencode"
# program = "opencode"
# args = ["run", "{prompt}"]
# adapter = "text"
# input = "argument"
```

Untrusted material (HTTP bodies, DNS TXT, tool output) is sanitized for the terminal and fenced as `<<<UNTRUSTED …>>>` before it is shown to an agent.

## Configuration

See the commented file written on first launch, or `src/config.rs` (`DEFAULT_CONFIG_TOML`). Notable sections:

- `[ui]` theme (`default`, `high-contrast`, `monochrome`), tick rate, editor
- `[agent]` active profile, timeouts, context size
- `[network]` timeouts, max body size, optional SOCKS/HTTP proxy (DNS and TLS collectors connect directly)
- `[collectors]` robots.txt, www check, concurrency, git history depth
- `[[tools.custom]]` extra programs with argument templates (`{domain}`, `{ip}`, `{url}`, …)

Logs go to `<data_dir>/logs/os-int.log.YYYY-MM-DD`, never to the TUI. Pass `--debug` or set `RUST_LOG`.

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `q` | Quit (or close an overlay) |
| `Esc` | Back / close overlay |
| `Tab` / `Shift+Tab` | Next / previous pane |
| `j` `k` | Move selection |
| `Enter` | Open / select |
| `/` | Search |
| `p` or `Ctrl+P` | Command palette |
| `a` | Ask agent |
| `n` | New investigation / page / finding |
| `e` | Edit page notes in `$EDITOR` (Overview/Notes), or agent/theme settings |
| `r` | Run collectors for the current page |
| `c` | Cancel the selected job |
| `?` | Help |
| `1`–`0` | Jump to Overview … Settings |

Agent proposals: `A` accept, `R` reject.

## OSINT, legal, and ethical use

os-int is an observation, monitoring, and diagnostic notebook for public information. Built-in collectors perform ordinary DNS queries, RDAP lookups, a single HTTP(S) fetch of a user-specified URL, a TLS handshake, a query to the public crt.sh certificate transparency search, and inspection of public git metadata. It does not include remote access, exploitation, credential attacks, or active network probes. Custom tools marked `active` are refused.

You are responsible for complying with applicable law, terms of service, and your organization's rules. Do not use this software to access systems you are not authorized to investigate, to harass people, or to collect personal data without a lawful basis.

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). In short:

- `model/` — domain types, no UI
- `storage/` — SQLite (rusqlite, bundled) plus a content-addressed file store
- `collectors/`, `tools/`, `agent/`, `jobs/` — work that must not block the UI
- `app/` — state and actions, testable without a terminal
- `tui/` — Ratatui event loop and rendering

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

MSRV is 1.88. Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
