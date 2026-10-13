# os-int

**A terminal workbench for open-source intelligence investigations.**

os-int is where you keep a case together. You create an investigation, add the domains, addresses, URLs, and files you care about, collect public records, and keep every observation tied to its source. You can take notes, ask a local AI agent CLI, and write a report — without leaving the terminal.

It orchestrates information, tools, sources, and agents. Built-in collectors cover the passive basics. Everything else plugs in as an external program you already have, or as the agent CLI you already use.

```text
┌ os-int · Example Corp ───────────────────────────────────────── Claude ● ┐
│ Navigate │ P-1  example.com                         │ example.com        │
│          │ Domain                                   │                    │
│ Overview │                                          │ Entities           │
│ Pages    │ Target   example.com                     │  E-1  Domain       │
│ Sources  │                                          │  E-4  IP           │
│ Findings │ Entities on this page                    │                    │
│ Entities │   E-1  Domain   example.com              │ Findings           │
│ Timeline │   E-4  IP       93.184.216.34            │  F-1  [COL] HTTPS  │
│ Notes    │                                          │                    │
│ Agent    │ Findings                                 │                    │
│ Jobs     │   F-1  [COL]  HTTPS redirects to apex    │                    │
│ Settings │                                          │                    │
├──────────┴──────────────────────────────────────────┴────────────────────┤
│ [a] Ask   [/] Search   [p] Palette   [r] Collect   [?] Help               │
└───────────────────────────────────────────────────────────────────────────┘
```

*Sketch of a wide terminal. The live layout drops the right-hand detail pane below 96 columns. Press `?` inside the app for the keyboard overlay.*

| | |
| --- | --- |
| **New here** | [Install](#install), then [your first investigation](#your-first-investigation) |
| **Day to day** | [How a case is organized](#how-a-case-is-organized), [keyboard](#keyboard), [command line](#command-line) |
| **Going further** | [User guide](docs/guide.md) · [Examples](docs/examples.md) · [FAQ](docs/faq.md) |
| **Contributing** | [Architecture](docs/ARCHITECTURE.md) · [Database tables](docs/schema.md) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) |

---

## What you use it for

- Keep one investigation per case, with pages for each subject (a domain, an IP, a URL, a repository, a file, a person, an organization).
- Collect public DNS, RDAP, HTTP, TLS, certificate-transparency, IP, repository, and file metadata, and store the raw response.
- Read diagnostics of published records: mail policy, response headers, certificate validity, DNSSEC, and `security.txt`.
- Attach provenance to every record so collected data, original material, your notes, and AI inference stay distinct.
- Cite anything with a short id (`P-1`, `S-12`, `E-4`, `F-1`) that the interface, the agent, and reports all share.
- Run passive external tools (`whois`, `dig`, `gh`, or your own) with an explicit argument list. There is no shell.
- Ask Claude Code, Codex, or another local agent CLI a question about the case. Proposed changes are previewed before anything is written.
- Export a Markdown or JSON report, or a portable directory you can import on another machine.

> [!TIP]
> Collection works with no AI agent installed. If `claude` or `codex` is on your `PATH`, press `a` from any investigation screen.

## Boundaries

os-int is an observation, monitoring, and diagnostic notebook for public information.

Built-in collectors perform ordinary DNS queries, RDAP lookups, a single HTTP(S) fetch of a URL you specify, a TLS handshake, a query to the public [crt.sh](https://crt.sh) certificate transparency search, inspection of public git metadata, and inspection of a local file you point at. Diagnostics describe what a name publishes. A gap is stored as a collected finding and cleared when a later collection shows the record is in place.

Custom tools marked `active` are refused. You are responsible for complying with applicable law, terms of service, and your organization's rules.

## Install

You need [Rust](https://rustup.rs) **1.88** or newer. The crate uses the 2024 edition. SQLite is bundled, so there is no separate database to install.

```bash
git clone https://github.com/pjourney/osint-rat.git
cd osint-rat
cargo install --path .
```

Or install straight from the repository:

```bash
cargo install --git https://github.com/pjourney/osint-rat
```

The binary is named `os-int`. Make sure `~/.cargo/bin` is on your `PATH`.

Check the install:

```bash
os-int --version
os-int doctor
os-int config
```

`doctor` reports the database, the config file, and whether common programs (`claude`, `codex`, `dig`, `whois`, `git`, `gh`, `exiftool`, `curl`, `openssl`) are on `PATH`. Missing programs are optional. A database warning makes `doctor` exit non-zero, which is useful in bug reports.

### Where your files live

The first launch writes a commented `config.toml` and a SQLite database. Override either location with `--config`, `--data-dir`, `OS_INT_CONFIG`, or `OS_INT_DATA_DIR`.

| Platform | Config | Data |
| --- | --- | --- |
| Linux | `~/.config/os-int/config.toml` | `~/.local/share/os-int/` |
| macOS | `~/Library/Application Support/os-int/config.toml` | `~/Library/Application Support/os-int/` |
| Windows | `%APPDATA%\os-int\config.toml` | `%APPDATA%\os-int\` |

The data directory holds `os-int.sqlite3`, daily log files, a small cache, and one folder per investigation. Details are in the [user guide](docs/guide.md#where-files-live).

## Your first investigation

Launch the workbench:

```bash
os-int
```

On the welcome screen:

1. Press `n` and name the investigation, for example `Example Corp`.
2. Press `n` again and add a page. Choose **Domain** and enter `example.com`.
3. Press `r`. os-int runs DNS, RDAP, HTTP, TLS, and certificate transparency in the background. The interface stays responsive. Watch progress under **Jobs** (`8`).
4. Press `3` for sources, `4` for findings, `5` for entities. Each row has a short id you can cite later.
5. Press `a`, ask a question about a highlighted record, and review any proposed changes before accepting them.
6. Press `p`, type `report`, and generate a Markdown report.

The same case from the shell, if you prefer to collect before opening the interface:

```bash
os-int collect domain example.com --investigation "Example Corp"
os-int report "Example Corp"
os-int open "Example Corp"
```

`collect` creates the investigation and the page when they do not already exist. `os-int new "Example Corp"` also creates a case, then opens it in the workbench and stays there until you quit.

## How a case is organized

```text
Investigation
  └── Pages          one subject each (domain, IP, URL, file, …)
        ├── Sources        raw observations, with provenance
        ├── Entities       normalized identifiers (example.com, 93.184.216.34)
        ├── Relationships  typed links between entities (resolves_to, redirects_to, …)
        ├── Findings       claims, each citing the sources that support them
        ├── Notes          your writing, or a note accepted from an agent
        ├── Artifacts      saved bodies, context packs, reports
        └── Timeline       what happened, including when a public record changed
```

| Provenance | Badge | Meaning |
| --- | --- | --- |
| Collected | `COL` | Gathered by a collector or external tool |
| External | `EXT` | Original material you supplied |
| Analyst | `ANL` | Written by you |
| Agent | `AI` | Inference from an agent. Accepting a proposal keeps this label |

A finding with no cited sources is a hypothesis. AI inference is never relabelled as collected fact.

Short references are per-investigation sequence numbers (`S-12`). UUIDs are the real primary keys, which keeps import and export collision-free. The prefixes are `P` page, `S` source, `F` finding, `E` entity, `R` relationship, `N` note, `A` artifact, `T` timeline, `J` job, `Q` agent run, `X` proposal.

## Moving around

The screen has a navigation column, a main pane, and (on a wide terminal) a detail pane. `Tab` and `Shift+Tab` move between panes. `j` / `k` or the arrow keys move the selection. `Enter` opens the selection.

| Key | Action |
| --- | --- |
| `q` | Quit, or close an overlay |
| `Esc` | Back, or close an overlay |
| `Ctrl+C` | Quit from anywhere |
| `/` or `Ctrl+F` | Search pages, entities, findings, sources, artifacts, and agent runs |
| `p` or `Ctrl+P` | Command palette |
| `a` | Ask the active agent |
| `n` | New investigation, page, or finding, depending on the screen |
| `e` | Edit the current page notes in `$EDITOR`, or open agent and theme settings |
| `r` | Run the default collectors for the current page |
| `c` | Cancel the selected job (Jobs screen) |
| `?` | Help |
| `1`–`6` | Overview, Pages, Sources, Findings, Entities, Timeline |
| `7` `8` `9` `0` | Agent, Jobs, Tools, Settings |

Notes and Artifacts are in the navigation column. On the welcome screen, `o` opens an existing investigation and `d` runs doctor. While an agent proposal is open: `A`, `y`, or `Enter` accepts it; `R` or `n` rejects it; `Esc` dismisses the list.

The full palette (new page, one collector, add a finding, generate a report, and the rest) is listed in the [user guide](docs/guide.md#command-palette).

## Command line

Global flags work with every command: `--config <file>`, `--data-dir <dir>`, and `--debug` (debug-level file logs).

```text
os-int                                      # open the workbench
os-int new "Example Corp" -d "Public site"
os-int open "Example Corp"                  # name, UUID, or unique prefix
os-int list
os-int collect domain example.com --investigation "Example Corp"
os-int collect example.com --collector dns  # kind is guessed from the value
os-int collect ip 192.0.2.15
os-int report "Example Corp" --format markdown -o ./example.md
os-int export "Example Corp" -o ./export
os-int import ./export --rename             # --rename only if the id already exists
os-int doctor
os-int config
```

`report --format json` writes JSON. Any other format value writes Markdown. Without `-o`, the file is saved inside the investigation directory and the path is printed.

Collector ids: `dns`, `rdap`, `http`, `tls`, `ct`, `ip`, `repository`, `file`. Omit `--collector` to run the defaults for that target. A domain runs DNS, RDAP, HTTP, TLS, and certificate transparency. An IP runs RDAP and IP intelligence. See the [collector matrix](docs/guide.md#collectors) for every target kind.

## Configuration

Every setting is optional. Delete a line to fall back to the built-in default. The file written on first launch is commented; the [configuration reference](docs/guide.md#configuration) explains each key.

```toml
[ui]
theme = "default"          # default, high-contrast, or monochrome
# editor = "nvim"          # otherwise $VISUAL, then $EDITOR, then vi

[agent]
active = "claude"          # claude, codex, or a profile under [agents]

[network]
# proxy = "socks5h://127.0.0.1:9050"   # HTTP(S) only; DNS and TLS connect directly

[tools]
# disabled = ["dig"]
```

To point os-int at another agent:

```toml
[agent]
active = "codex"

# [agents.opencode]
# name = "opencode"
# program = "opencode"
# args = ["run", "{prompt}"]
# adapter = "text"
# input = "argument"
```

os-int never calls an LLM API itself. It runs the program you configured as a subprocess. Claude Code is invoked with `--tools ""`, so prompt-injected page content cannot make that agent run commands. Collected material is wrapped in `<<<UNTRUSTED …>>>` fences before the agent sees it. The exact context pack is saved under the investigation for audit. Nothing an agent proposes is applied until you accept it.

Logs go to `<data_dir>/logs/os-int.log.YYYY-MM-DD`, never into the terminal UI. Pass `--debug` or set `RUST_LOG` when you need more detail.

## Develop

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

A throwaway data directory keeps experiments away from your real cases:

```bash
cargo run -- --data-dir /tmp/os-int-dev --config /tmp/os-int-dev/config.toml
```

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request, and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before changing structure.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
