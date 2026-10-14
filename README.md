# os-int

**A local-first investigation ontology workbench for OSINT and cyber cases.**

Fuse public sources into objects and links, resolve identities, Search Around the graph, run playbooks, and keep humans in control of every Action (including AI). Collectors are plugins. The graph and the resolution engine are the product.

**New here?** [Install](#install) (sixty-second one-liner on Linux and macOS), then [import the demo case](#import-the-demo-case-first). You should see Graph, a resolved entity, Search Around, and cited findings in a few minutes without reading the architecture notes.

![Graph](docs/images/graph.svg)

![Object View](docs/images/object-view.svg)

![Search Around](docs/images/search-around.svg)

```text
┌ os-int · Example Corp ───────────────────────────────────────── Claude ● ┐
│ Navigate │ P-1  example.com                         │ example.com        │
│ Overview │ Domain                                   │                    │
│ Pages    │                                          │ Entities           │
│ Sources  │ Target   example.com                     │  E-1  Domain       │
│ Findings │                                          │  E-4  IP           │
│ Entities │ Entities on this page                    │                    │
│ Graph    │   E-1  Domain   example.com              │ Findings           │
│ Timeline │   E-4  IP       192.0.2.1                │  F-1  [COL] HTTPS  │
│ Notes    │                                          │                    │
│ Agent    │ Findings                                 │                    │
│ Jobs     │   F-1  [COL]  HTTPS redirects to apex    │                    │
│ Settings │                                          │                    │
├──────────┴──────────────────────────────────────────┴────────────────────┤
│ [a] Ask   [/] Search   [p] Palette   [g] Graph   [?] Help                 │
└───────────────────────────────────────────────────────────────────────────┘
```

*Sketch of a wide terminal. The live layout drops the right-hand detail pane below 96 columns. Press `?` inside the app for the keyboard overlay.*

| | |
| --- | --- |
| **New here** | [Install](#install), then [import the demo](#import-the-demo-case-first) |
| **Day to day** | [How a case is organized](#how-a-case-is-organized), [keyboard](#keyboard), [command line](#command-line) |
| **Going further** | [Vision](VISION.md) · [Roadmap](PHASED_ROADMAP.md) · [User guide](docs/guide.md) · [Install](docs/install.md) · [Examples](docs/examples.md) · [FAQ](docs/faq.md) · [Plugins](docs/plugins.md) · [Connectors](docs/connectors.md) · [Playbooks](docs/playbooks.md) · [Metrics](docs/metrics.md) · [Collaboration](docs/collaboration.md) · [HTTP API](docs/api.md) · [Scale](docs/scale.md) · [Ontology exchange](docs/ontology-exchange.md) |
| **Contributing** | [Architecture](docs/ARCHITECTURE.md) · [Ontology](docs/ontology.md) · [Database tables](docs/schema.md) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) |

---

## What this is / is not

**This is** a local-first investigation ontology for OSINT and cyber cases. You fuse public sources into objects and links, keep provenance on every record, and stay in control of every Action—including those a local AI agent proposes. Collectors are plugins. The graph and the resolution engine are the product. The [vision](VISION.md) and [phased roadmap](PHASED_ROADMAP.md) are complete.

**This is not** a recon CLI, a collector-count contest, or an active-probing toolkit. Built-in collectors stay passive. There is no silent AI writeback, no automatic identity merge, and no cloud requirement. The bar is an enterprise-class investigative loop (ontology, fusion, resolution, Search Around, playbooks, governed Actions)—not module-count parity. Deferred follow-ups after v1 live in [docs/deferred.md](docs/deferred.md).

The [Boundaries](#boundaries) section is the operational rule for what built-in collectors may do. It is not relaxed by the vision.

---

## What you use it for

- Keep one investigation per case, with pages for each subject (a domain, an IP, a URL, a repository, a file, a person, an organization).
- Collect public DNS, RDAP, HTTP, TLS, certificate-transparency, IP, repository, file, email-identity, username-profile, and subdomain metadata, and store the raw response.
- Ingest STIX 2.1, MISP, CSV/JSON observables, or an Evidence Batch from a file or a sidecar connector.
- Read diagnostics of published records: mail policy, response headers, certificate validity, DNSSEC, and `security.txt`.
- Attach provenance to every record so collected data, original material, your notes, and AI inference stay distinct.
- Cite anything with a short id (`P-1`, `S-12`, `E-4`, `F-1`) that the interface, the agent, and reports all share.
- Run passive external tools (`whois`, `dig`, `gh`, or your own) with an explicit argument list. There is no shell.
- Run a YAML playbook (`domain-infra`, `mail-policy`, `cert-estate`, `repo-surface`) or watch collect steps. Templates stay pending until you accept them.
- Ask Claude Code, Codex, or another local agent CLI a question about the case. Proposed changes are previewed before anything is written.
- Export a Markdown or JSON report, or a portable directory you can import on another machine.
- Mark a case with TLP, keep an append-only audit log, and optionally share a data directory with `owner` / `analyst` / `reader` roles.

> [!TIP]
> Collection works with no AI agent installed. If `claude` or `codex` is on your `PATH`, press `a` from any investigation screen.

## Boundaries

os-int is an observation, monitoring, and diagnostic notebook for public information.

Built-in collectors perform ordinary DNS queries, RDAP lookups, a single HTTP(S) fetch of a URL you specify, a TLS handshake, a query to the public [crt.sh](https://crt.sh) certificate transparency search, inspection of public git metadata, inspection of a local file you point at, public profile URL checks, and passive subdomain expansion. Diagnostics describe what a name publishes. A gap is stored as a collected finding and cleared when a later collection shows the record is in place.

Custom tools and sidecars marked `active` are refused. You are responsible for complying with applicable law, terms of service, and your organization's rules.

## Install

Linux and macOS, from a **published** tagged GitHub Release (checksum-verified into `~/.local/bin`). Pin both the script URL and `OS_INT_VERSION` to the same tag, not `main`:

```bash
OS_INT_VERSION=v1.0.0 curl -fsSL https://raw.githubusercontent.com/pjourney/osint-rat/v1.0.0/scripts/install.sh | bash
```

Until the crate is on crates.io:

```bash
cargo binstall --git https://github.com/pjourney/osint-rat
```

Windows: download the `os-int-x86_64-pc-windows-msvc.zip` asset from [Releases](https://github.com/pjourney/osint-rat/releases) and verify the `.sha256` file. Details, Homebrew tap notes, and source builds: [docs/install.md](docs/install.md).

From source (Rust **1.88+**, edition 2024; SQLite is bundled):

```bash
cargo install --git https://github.com/pjourney/osint-rat
```

The binary is named `os-int`. Check the install:

```bash
os-int --version
os-int doctor
os-int config
```

`doctor` reports the database, the config file, and whether common programs (`claude`, `codex`, `dig`, `whois`, `git`, `gh`, `exiftool`, `curl`, `openssl`, `subfinder`, `amass`) are on `PATH`. Missing programs are optional. A database warning makes `doctor` exit non-zero, which is useful in bug reports.

### Where your files live

The first launch writes a commented `config.toml` and a SQLite database. Override either location with `--config`, `--data-dir`, `OS_INT_CONFIG`, or `OS_INT_DATA_DIR`.

| Platform | Config | Data |
| --- | --- | --- |
| Linux | `~/.config/os-int/config.toml` | `~/.local/share/os-int/` |
| macOS | `~/Library/Application Support/os-int/config.toml` | `~/Library/Application Support/os-int/` |
| Windows | `%APPDATA%\os-int\config.toml` | `%APPDATA%\os-int\` |

The data directory holds `os-int.sqlite3`, daily log files, a small cache, and one folder per investigation. Details are in the [user guide](docs/guide.md#where-files-live).

## Import the demo case first

The archive at [`examples/demo-case/`](examples/demo-case/) is a documentation-range fixture (`example.com`, `192.0.2.0/24`). It already has a cited graph, a resolved hostname, playbook output, and findings that cite sources. You do not need the network.

```bash
git clone https://github.com/pjourney/osint-rat.git
cd osint-rat
mkdir -p /tmp/os-int-demo
alias os-int-demo='os-int --data-dir /tmp/os-int-demo --config /tmp/os-int-demo/config.toml'
os-int-demo import examples/demo-case
os-int-demo open "Example Corp"
```

Then, without reading ARCHITECTURE:

1. Press `g` — Graph of `example.com` and connected infra.
2. Press `s` — Search Around two hops (addresses, then ASN / organization).
3. Press `Enter` — Object View: properties, catalogued links, constituent hostname, cited findings.
4. Press `6` — Timeline, including **Playbook domain-infra**.
5. Press `4` — Findings. Every row cites a source (`[COL]`).

Shell equivalent: `os-int-demo search-around "Example Corp" E-1 --hops 2`.

To collect a live case instead, press `n` on the welcome screen, add a domain page, and press `r` (or `os-int collect domain example.com --investigation "Example Corp"`). `collect` creates the investigation and the page when they do not already exist.

## Success metrics

Stars and collector counts are not success. Methodology and the demo-case numbers: [docs/metrics.md](docs/metrics.md).

| Metric | Demo-case result |
| --- | --- |
| Time to a useful graph (≥20 linked entities) | Import the demo; Search Around 2 hops from `E-1` (20 visible objects) |
| Findings with ≥1 cited source | 100% (3 / 3), provenance `collected` |
| Search Around hops | Walkthrough uses 2; no session telemetry is uploaded |
| Playbooks | One `domain-infra` run on the demo timeline |
| External connectors | In-tree `hello-batch` plus the [community template](examples/community/) |

## Open platform

Anything an analyst already has can become cited objects without forking Rust. Sidecars print Evidence Batch JSON; playbooks are YAML. How to publish, and the review bar (passive-only, provenance, tests): [docs/plugins.md](docs/plugins.md). Copy [`examples/community/`](examples/community/) when you open a plugin pull request. The Phase 4 `hello-batch` sidecar still applies against a release binary.

## How a case is organized

```text
Investigation
  └── Pages          one subject each (domain, IP, URL, file, …)
        ├── Sources        raw observations, with provenance
        ├── Entities       normalized identifiers (example.com, 192.0.2.1)
        ├── Relationships  typed links from the link-type catalog (resolves_to, redirects_to, …)
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
| `i` | Ingest a STIX / MISP / CSV / JSON / Evidence Batch file |
| `b` | Run playbook (bundled YAML or a file) |
| `v` | Object View: highlighted entity, or the current page |
| `g` | Graph: entities as nodes, relationships as edges |
| `s` | Search Around from the current selection |
| `+` / `-` | Graph: more / fewer hops (re-query from the original seed) |
| `[` / `]` | Graph: move the facet cursor |
| `f` | Graph: apply or toggle the highlighted facet |
| `x` | Graph: clear facet filters |
| `c` | Cancel the selected job (Jobs screen) |
| `?` | Help |
| `1`–`6` | Overview, Pages, Sources, Findings, Entities, Timeline |
| `7` `8` `9` `0` | Agent, Jobs, Tools, Settings |

Graph, Notes, and Artifacts are in the navigation column. On the welcome screen, `o` opens an existing investigation and `d` runs doctor. While an agent proposal is open: `A`, `y`, or `Enter` accepts it; `R` or `n` rejects it; `Esc` dismisses the list.

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
os-int collect email alice@example.com --collector email
os-int ingest ./bundle.json --format stix --investigation "Example Corp"
os-int playbook run domain-infra --investigation "Example Corp" --target example.com
os-int watch --playbook domain-infra --investigation "Example Corp" --dry-run
os-int report "Example Corp" --format markdown -o ./example.md
os-int export "Example Corp" -o ./export
os-int export "Example Corp" --format stix -o ./bundle.json
os-int get "Example Corp" E-1
os-int search-around "Example Corp" E-1 --hops 2
os-int serve --token "$OS_INT_API_TOKEN"
os-int import examples/demo-case            # first-run fixture (Graph + Search Around)
os-int import ./export --rename             # --rename only if the id already exists
os-int marking set "Example Corp" TLP:AMBER
os-int team add "Example Corp" bob --role analyst
os-int audit "Example Corp"
os-int doctor
os-int config
```

`report --format json` writes JSON. Any other format value writes Markdown. Without `-o`, the file is saved inside the investigation directory and the path is printed.

Collector ids: `dns`, `rdap`, `http`, `tls`, `ct`, `ip`, `repository`, `file`, `email`, `username`, `subdomain`. Omit `--collector` to run the defaults for that target. A domain runs DNS, RDAP, HTTP, TLS, certificate transparency, and subdomain expansion. An IP runs RDAP and IP intelligence. An email runs DNS, CT, and the email identity pack. A username runs the username pack. See the [collector matrix](docs/guide.md#collectors) and [connectors](docs/connectors.md).

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

[identity]
# user_id = "analyst"      # actor on audit rows; defaults to $USER
# machine_name = "workstation"
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
