# os-int user guide

This is the complete guide to using os-int: the workbench, the collectors, the agent, the command line, and the configuration file. For a shorter introduction, start with the [README](../README.md). For copy-paste sessions, see [examples](examples.md). For how the code is put together, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Contents

- [Concepts](#concepts)
- [Where files live](#where-files-live)
- [The workbench](#the-workbench)
- [Building a case](#building-a-case)
- [Graph and Search Around](#graph-and-search-around)
- [Timeline and geospatial](#timeline-and-geospatial)
- [Entity resolution](#entity-resolution)
- [Collectors](#collectors)
- [Ingest](#ingest)
- [Playbooks](#playbooks)
- [Diagnostics](#diagnostics)
- [External tools](#external-tools)
- [Asking an agent](#asking-an-agent)
- [Reports, export, and import](#reports-export-and-import)
- [Command-line reference](#command-line-reference)
- [Configuration](#configuration)
- [Keyboard](#keyboard)
- [Troubleshooting](#troubleshooting)
- [Using it responsibly](#using-it-responsibly)

Worked sessions are in [examples](examples.md). Common questions are in the [FAQ](faq.md). The SQLite table map is in [schema](schema.md).

## Concepts

An **investigation** is one case. It has a name, an optional description, and everything collected or written for that case.

A **page** is one subject inside the case. Page types:

| Type | Typical target | Collectors can run |
| --- | --- | --- |
| Domain | `example.com` | yes |
| IP address | `192.0.2.15` | yes |
| URL | `https://example.com/about` | yes |
| Email | `alice@example.com` | yes |
| ASN | `AS13335` | yes |
| Repository | `https://github.com/owner/project` | yes, when `git` is installed |
| File | `/path/to/evidence.pdf` | yes |
| Username | `octocat` | yes |
| Person, Organization, Event, Generic | a name or a note | no |

Identifiers are normalized on the way in, so `Example.COM.` and `example.com` are the same entity.

A **source** is one observation: a DNS answer set, an HTTP response, an RDAP document, a tool's stdout. Sources keep a content hash. When a later collection of the same record hashes differently, os-int writes an "observation changed" timeline event.

An **entity** is a normalized identifier (domain, IP, URL, email, ASN, repository, file, username, person, organization, certificate). A **relationship** is a directed link between two entities, such as `resolves_to` or `redirects_to`, optionally supported by sources.

A **finding** is a claim. It should cite the sources that support it. Confidence is `unknown`, `low`, `medium`, or `high`.

**Provenance** is stored on every record and shown as a badge:

| Provenance | Badge | Meaning |
| --- | --- | --- |
| `collected` | `COL` | Gathered by a collector or external tool |
| `external` | `EXT` | Original material you supplied |
| `analyst` | `ANL` | Written by you |
| `agent` | `AI` | Inference from an agent. Stays labelled this way after you accept it |

**Short references** are how you cite records in notes, findings, agent questions, and reports:

| Prefix | Record | Prefix | Record |
| --- | --- | --- | --- |
| `P` | Page | `N` | Note |
| `S` | Source | `A` | Artifact |
| `F` | Finding | `T` | Timeline event |
| `E` | Entity | `J` | Job |
| `R` | Relationship | `Q` | Agent run |
| `V` | Resolution | `X` | Proposal |
| `C` | Event (instant or interval) | | |

`S-12` means "source number 12 in this investigation." The UUID remains the primary key.

Collectors record relationships with a small set of verbs. Any snake_case phrase is allowed when you link entities yourself. Ones you will see from built-in collectors include:

| Relationship | Typical meaning |
| --- | --- |
| `resolves_to` | A name resolved to an address |
| `alias_of` | Two names for the same subject, including `www` |
| `reverse_dns` | A pointer record for an address |
| `uses_nameserver` | A domain's nameserver |
| `mail_handled_by` | An MX target |
| `spf_includes` | A domain named by SPF |
| `redirects_to` | An HTTP redirect |
| `links_to` / `mentions` | A link or name found in a page |
| `presents_certificate` / `issued_by` / `covers` | A host's certificate, its issuer, and the names that certificate lists |
| `subdomain_of` | A discovered hostname beneath the name you collected |
| `announced_by` / `operated_by` | Origin ASN, network operator, or repository organization |
| `registered_with` / `registered_to` | RDAP registrar or registrant |
| `hosted_on` | An email address at a domain |
| `contributed_to` / `references` | A repository contributor, or a host a repository mentions |

## Where files live

Run `os-int config` to print the paths this process will use. Resolution order:

1. `--config` / `--data-dir` on the command line
2. `OS_INT_CONFIG` / `OS_INT_DATA_DIR`
3. `storage.data_dir` in the config file (data directory only)
4. The platform default

| Platform | Config file | Data directory |
| --- | --- | --- |
| Linux | `~/.config/os-int/config.toml` | `~/.local/share/os-int/` |
| macOS | `~/Library/Application Support/os-int/config.toml` | `~/Library/Application Support/os-int/` |
| Windows | `%APPDATA%\os-int\config.toml` | `%APPDATA%\os-int\` |

```text
<data_dir>/
  os-int.sqlite3                     structured records
  logs/os-int.log.YYYY-MM-DD         tracing output (never the TUI)
  cache/rdap/*.json                  IANA RDAP bootstrap cache
  investigations/<uuid>/
    snapshots/ab/abcdef….bin         content-addressed HTTP bodies, RDAP JSON, …
    files/                           imported evidence files
    agent/                           exact context packs sent to agents
    reports/                         generated reports
    repos/                           partial clones of public repositories
```

Database rows store paths relative to the investigation directory, so an export can be imported on another machine.

## The workbench

```bash
os-int
```

With no subcommand, os-int opens the terminal interface on your last investigation, or on the welcome screen when you have none.

```text
┌ Welcome ──────────────────────┐  ┌ Environment ──────────────┐
│ os-int                        │  │ Database        OK        │
│                               │  │ Claude Code     missing   │
│ [N] New Investigation         │  │ dig             OK        │
│ [O] Open Investigation        │  │ …                         │
│ [D] Doctor                    │  └───────────────────────────┘
│ [?] Help                      │
│                               │
│ Recent investigations         │
└───────────────────────────────┘
```

A wide investigation screen has three columns: **Navigate**, the current section, and a detail pane. Below 96 columns the detail pane is hidden. Below 72 columns the navigation labels tighten. 120×40 is a comfortable size.

The footer is always:

```text
[a] Ask   [/] Search   [p] Palette   [r] Collect   [g] Graph   [s] Around   [?] Help
```

Background work (HTTP, DNS, TLS, subprocesses, agent calls, report rendering) runs on a job queue. The interface never waits on that I/O. Open **Jobs** (`8`) to watch or cancel a job. Open **Tools** (`9`) to see which external programs were found. Open **Settings** (`0`) or press `e` there to change the active agent, an optional program override, the agent timeout, and the theme (`default`, `high-contrast`, `monochrome`).

os-int remembers the last view, page, and agent thread for each investigation.

### Command palette

Press `p` or `Ctrl+P`, then type to filter.

| Command | What it does |
| --- | --- |
| New Investigation | Create a case |
| Open Investigation | Switch case |
| New Page | Add a subject |
| Run Collectors | Default collectors for the current page |
| Run DNS / RDAP / HTTP / TLS / Certificate Transparency / IP Intelligence | One collector |
| Run Email Identity Pack / Username Pack / Subdomain Expansion | One collector |
| Ingest file | STIX, MISP, CSV/JSON, or Evidence Batch (`i`) |
| Run playbook | Bundled YAML or a file (`b`). Mode `run` / `dry-run` |
| Playbook prompt | Seed the agent question with suggested Actions |
| Watch playbook | Re-run collect steps; mode `dry-run` / `once` |
| Ask Agent | Question the active agent |
| Generate Report | Markdown or JSON |
| Add Finding / Add Note | Analyst-authored records |
| Open Entities, Sources, Findings, Timeline, Jobs, Tools, Artifacts, Agent History | Jump to that screen |
| Nearby (country / prefix) | Timeline / Entities: list co-located entities (`l`) |
| Object View | Typed properties, catalogued links, sources, and findings for the selected entity or page (`v`) |
| Graph | Link-analysis canvas of entities and relationships (`g`) |
| Search Around | Expand 1–N hops from the selection and merge into Graph (`s`) |
| Configure Agent | Agent profile and theme |
| Doctor | Environment checks |
| Help | Keyboard overlay |
| Cancel Running Job | Cancel the selected job |

## Building a case

### From the workbench

1. `n` on the welcome screen. Enter a name and an optional description. The description is used as the executive summary of a Markdown report.
2. `n` again. Pick a page type with `Left` / `Right` and enter the target. The form shows an example for that type (`example.com`, `192.0.2.15`, `AS13335`, …).
3. `r` on that page to collect.
4. Move through Sources, Findings, and Entities. The detail pane shows the selected record. Press `Enter` on an entity (or `v` from Pages) to open **Object View**. Press `g` to open **Graph**, then `s` to Search Around.
5. `e` on Overview edits the current page's notes in your editor (`ui.editor`, then `$VISUAL`, then `$EDITOR`, then `vi`). os-int suspends the terminal UI, runs the editor, and reloads the note when the editor exits.
6. Add a finding from the palette when you want to record a conclusion. Cite sources by short id (`S-1, S-4`). A finding you add this way is provenance `analyst`.

### From the shell

```bash
os-int new "Example Corp" --description "Public website and mail"
os-int collect domain example.com --investigation "Example Corp"
os-int open "Example Corp"
```

`open` and most investigation arguments accept the name, the UUID, or a unique UUID prefix. `os-int list` prints name, id prefix, and counts of pages, sources, findings, and entities.

Search (`/` or `Ctrl+F`) matches pages (title, target, notes), entities, findings, sources, artifacts, and agent runs. `Enter` jumps to the hit.

### Object View

Object View is the typed reading of one entity or page: property groups from the [ontology](ontology.md), linked objects grouped by catalog verb, supporting sources, and findings.

- From **Entities**, press `Enter` or `v` to open the highlighted row.
- From **Pages** or Overview, press `v` to open the current page (its primary entity when it has one).
- Palette: **Object View** uses the same rules; a leftover entity is never reused.
- `Esc` returns to the screen you came from.

Identity is always shown, so every object has at least one property group. Known relationship verbs use the link-type catalog (`resolves_to`, `subdomain_of`, …). Unknown snake_case verbs still store and appear under **Other**. Accepting an agent Action records who and when. `PROPOSE_RESOLVE` / `PROPOSE_UNRESOLVE` / `RUN_PLAYBOOK` / `ASSERT_FINDING_TEMPLATE` apply only after accept. `RUN_PLAYBOOK` may only name a bundled id.

When the object is a resolved canonical, Object View lists **Constituents** (the absorbed records). Sources and links from both sides stay visible.

## Graph and Search Around

Graph is a terminal link-analysis canvas (TUI-first; there is no embedded webview). Nodes are entities. Edges are relationships. The detail pane reuses Object View data for the selected node.

- Press `g`, pick **Graph** in Navigate, or run the palette command **Graph**. The seed is the current entity (Entities highlight, Object View, or the page's primary entity). If none is focused, the first entity is used. Opening Graph expands **1 hop**.
- Press `s` or the palette command **Search Around** to expand **2 hops** from the current selection. On an open Graph, those hops **merge** into the canvas. From any other screen, Search Around opens Graph at 2 hops.
- `+` / `-` change hop depth and re-query from the original seed (0–8). `hops` follows the last result's `hops_used`.
- The **facet strip** counts the unfiltered set by entity kind, top link types, and provenance. `[` / `]` move the cursor. `f` applies (or toggles) that facet; the visible set updates immediately. `x` clears filters.
- Render caps are **200 nodes** and **400 edges**. Crossing either threshold warns and asks for confirm before the first 200 / 400 are drawn. A capped result is labelled in the status line.
- `Enter` opens Object View for the selected node. `Esc` returns to the screen that opened Graph. The Entities table is unchanged: `Enter` there still opens Object View.

Empty investigations do not crash: Graph shows an empty neighborhood and a collect hint.

Resolved constituents are hidden: the graph shows **one node** for the canonical. Search Around from a constituent seed starts at the canonical.

## Timeline and geospatial

Time and place are first-class. The Timeline screen overlays **collection log rows** (`T-n`) and **Event objects** (`C-n`). An Event is an instant when `ended_at` is absent, otherwise an interval. STIX `first_observed` / `last_observed` and Evidence Batch `events` import as Events. Observation-hash changes and diagnostic flips also promote Events (`change_intelligence` on the field bag). Agent-suggested findings still require accept.

- Press `6` or the palette command **Open Timeline**.
- `[` / `]` move the entity-filter cursor. `f` filters the overlay to that entity (resolution bags included). `x` clears the filter.
- `Enter` on a row with a linked entity opens Object View. `Esc` returns to the same Timeline selection.
- `l` on Timeline or Entities lists **nearby** entities that share a `country` or `prefix`. `Esc` restores the previous list and selection. Empty data does not crash.

Co-occurrence (`os-int cooccur`) prints pairs that share an Event, a cited source, or a time window (default 24 hours).

Geo fields (`country`, `latitude`, `longitude`, `prefix`, `geo_source`) come from RDAP, Team Cymru, an optional local geo database, or a country centroid. `geo_source` records how coordinates were derived (`collected`, `geo_db`, `country_centroid`). Built-ins never fetch a geo service. Set `collectors.geo_db_path` to a JSON records file or an MMDB. A missing file is reported and skipped.

```bash
os-int geojson "Example Corp" -o ./example.geojson
os-int export "Example Corp" --format geojson -o ./example.geojson
os-int cooccur "Example Corp" --window 24
```

GeoJSON is a FeatureCollection of Points and envelope Polygons plus properties (`kind`, `value`, `country`, `prefix`, `geo_source`, `canonical_id`). It is not the investigation archive.

## Entity resolution

Merge records that are the same real-world thing without destroying history. Unresolve is always available.

### Safe merge practices

- Merge only when you would treat the records as one thing in a report. Same host (`example.com` domain + hostname) is a typical yes. A domain and an unrelated URL that happens to share a string is a no.
- Prefer the more specific or already-cited object as the winner (the highlighted marked row). The winner keeps its kind and value.
- Read the confirm text. Esc cancels. Nothing merges until you confirm.
- After merge, open Object View and check constituents, sources, and links. Both sides' citations must still be there.
- Unresolve (`u` on the canonical) if the bag was wrong. Constituents return; property writes from that bag are undone. SQLite rows are not deleted, so `os-int doctor` should stay clean.
- Do not accept an agent `PROPOSE_RESOLVE` without checking the entity ids. AI only proposes.
- Suggestions (palette **Resolution suggestions**) are scored hints: same canonical email/domain/IP/repo URL, username↔profile URL, certificate SAN clusters. They never auto-merge.

### Keys

- **Entities:** `Space` marks or unmarks. `m` proposes a merge of the marked set (the highlighted marked row is the canonical). Enter confirms; Esc cancels.
- **Object View / Entities:** `u` proposes unresolve of the active bag on the canonical.
- Palette: **Merge selected entities**, **Unresolve entity**, **Resolution suggestions**.

Timeline events `entities_resolved` and `entities_unresolved` record the actor and constituent ids.

## Collectors

A collector takes one target and returns sources, entities, relationships, timeline events, and artifacts. It does not write the database itself. The workbench or the `collect` command applies the batch in one transaction.

Press `r` or run `os-int collect` without `--collector` to run the **default** set. Ask for one collector by id when you want a narrower pass.

| Id | Accepts | In the default set | What it records |
| --- | --- | --- | --- |
| `dns` | domain, email | yes | Address, mail, nameserver, TXT, CAA, and related records. Also resolves `www.<domain>` when `check_www` is true. Diagnostics for mail policy and DNSSEC. |
| `rdap` | domain, ip, asn | yes | Registrar, dates, nameservers, and network owner, via the IANA RDAP bootstrap (cached for 7 days). |
| `http` | domain, url | yes | Status, redirect chain, headers, page metadata, links, header diagnostics, `robots.txt` and sitemap discovery, and `security.txt`. |
| `tls` | domain, url | yes | Certificate chain, subject alternative names, issuer, protocol, cipher, and validity diagnostics. Validated against the Mozilla root store. |
| `ct` | domain, email | yes | Names, issuers, and validity dates from the public crt.sh certificate transparency search. |
| `ip` | ip | yes | Reverse DNS, origin ASN, prefix, and registry country, using Team Cymru's public DNS interface. |
| `repository` | repository | yes, if `git` is on `PATH` | Remotes, commits, contributors, and referenced hosts. `gh` is used for GitHub metadata when it is installed. Clones are shallow and stored inside the investigation. |
| `file` | file | yes | SHA-256, MIME type, and timestamps. ExifTool metadata when `exiftool` is installed. |
| `email` | email | yes | Hosted domain, local-part username, `hosted_on` / `associated_with`. No invented website URL. No network. |
| `username` | username | yes | Public profile URL checks (GitHub, GitLab, Codeberg, Reddit, Hacker News, Keybase). 2xx responses become `associated_with`; 3xx is not a hit. |
| `subdomain` | domain | yes | Passive expansion (common prefixes plus optional `subfinder` / `amass -passive`) with `subdomain_of` / `resolves_to` under `max_subdomains`. |

When RDAP is missing or thin, the RDAP collector merges a structured WHOIS parse if `whois` is installed.

So:

| Target | Default collectors |
| --- | --- |
| Domain | `dns`, `rdap`, `http`, `tls`, `ct`, `subdomain` |
| Email | `dns`, `ct`, `email` |
| IP address | `rdap`, `ip` |
| URL | `http`, `tls` |
| ASN | `rdap` |
| Repository | `repository` |
| File | `file` |
| Username | `username` |

```bash
os-int collect example.com                         # guesses "domain"
os-int collect domain example.com --collector dns
os-int collect url https://example.com/about --collector http
os-int collect ip 192.0.2.15
os-int collect repository https://github.com/owner/project
os-int collect file ./evidence.pdf
os-int collect email alice@example.com --collector email
os-int collect username octocat
```

When `--investigation` names a case that does not exist, `collect` creates it. When the name is a UUID that does not exist, `collect` stops and tells you to run `os-int list`. With no `--investigation`, `collect` writes into the most recently updated case, or creates one named after the target when you have none.

HTTP follows redirects itself so the chain is recorded. A redirect from a public address to a private or loopback address is refused unless `network.allow_private_redirects` is true. Response bodies are capped (`network.max_body_bytes`, default 5 MiB).

The repository collector runs `git` with an empty credential helper. It does not try to reach private repositories by borrowing your credentials.

Warnings (a single DNS query timed out, `robots.txt` returned an error) are attached to the job and printed by `collect`. They do not discard the rest of the batch.

Sidecars that print Evidence Batch JSON are `[[collectors.external]]`. Optional URLScan / OTX wrappers are `[[collectors.api]]`; they stay off until enabled and are never in the default collector set. See [connectors.md](connectors.md).

## Ingest

`os-int ingest` (workbench: `i`) parses a STIX 2.1 bundle, a MISP event, a CSV/JSON observable list, or an Evidence Batch file and applies it through the same transaction as a collector. Every imported source and finding is provenance `external`. A batch cannot claim `agent` provenance or clear collected diagnostics with `resolved_findings`. Identical re-ingest is idempotent.

```bash
os-int ingest tests/fixtures/ingest/stix-bundle.json --format stix --investigation "Example Corp"
os-int ingest tests/fixtures/ingest/misp-event.json --format misp
os-int ingest tests/fixtures/ingest/observables.csv --format csv
os-int ingest tests/fixtures/ingest/events-batch.json --format batch
```

`--format` is guessed from the file name when omitted. `--map kind=type,value=indicator` and `--kind domain` cover bulk CSV/JSON. Esc on the ingest form restores the entity selection; Enter applies and keeps that selection (canonical id after a merge). An empty path or a parse error leaves the form open. Caps, encoding, and the sidecar protocol: [connectors.md](connectors.md).

The in-repo sidecar example is [`examples/connectors/hello-batch/`](../examples/connectors/hello-batch/).

## Playbooks

A playbook is a YAML document: a seed object type plus ordered steps (`collect`, `search_around`, `run_tool`, `assert_finding_template`, `propose_resolve`). Collect and tool steps run; Search Around is a graph query; finding templates and merges become **pending proposals**. Nothing from a template is written until you accept it.

From the workbench, press `b` (or palette **Run playbook**). Left/Right cycles bundled ids (`domain-infra`, `mail-policy`, `cert-estate`, `repo-surface`). Mode `run` / `dry-run`. Esc closes the form and leaves the entity selection in place. Enter on dry-run shows the plan as text; Enter on run queues a job you can cancel from Jobs with `c`. Palette **Playbook prompt** seeds the agent question with suggested Actions — the agent still only proposes. Palette **Watch playbook** re-runs collect steps (`dry-run` / `once`) and opens proposals on changes.

From the shell:

```bash
os-int playbook list
os-int playbook validate domain-infra
os-int playbook run domain-infra --investigation "Example Corp" --target example.com
os-int playbook run domain-infra --investigation "Example Corp" --target example.com --dry-run
os-int watch --playbook domain-infra --investigation "Example Corp" --target example.com --dry-run
os-int watch --playbook domain-infra --investigation "Example Corp" --target example.com --once
os-int watch --playbook domain-infra --investigation "Example Corp" --interval 3600
```

`--dry-run` prints planned Actions and applies nothing. It does not create an investigation or investigation directories; the case must already exist. `--once` runs one watch pass. `--interval` loops until Ctrl+C (cancellation is observed during a pass). Collect batches apply in **one SQLite transaction** after the steps finish; a cancelled or failed collect applies nothing.

Built-in collectors stay **passive-only**. Tools marked `active` are refused. Nested playbooks are refused. Untrusted YAML cannot set provenance or clear collected findings. Caps, the YAML schema, and the bundled files: [playbooks.md](playbooks.md).

## Diagnostics

Diagnostics read published records. Each reading is `present`, `missing`, or `review`.

- `present` clears an earlier gap finding for that check.
- `missing` and `review` are stored as collected findings (`COL`), with high confidence for a missing record and medium confidence for one that needs review.

| Area | Always recorded | Recorded when the record is published |
| --- | --- | --- |
| DNS | MX, SPF, DMARC, and DNSSEC (a missing DS is `review`) | MTA-STS, TLS-RPT, BIMI, and CAA |
| HTTP headers | HTTPS, HSTS, Content-Security-Policy, `X-Content-Type-Options`, and framing (`X-Frame-Options` or CSP `frame-ancestors`) | |
| `security.txt` | A missing file on an HTTPS URL (HTML responses count as absent) | Contact and Expires. A missing Contact, a missing Expires, or an Expires date in the past is `review` |
| TLS | Trust against the Mozilla root store, and expiry. Expiry is `review` when the certificate is expired or inside 14 days of `notAfter` | |

`security.txt` is fetched from `/.well-known/security.txt` and then `/security.txt` (RFC 9116).

These readings describe what the name publishes. Gap and review readings are collected findings, with the source that produced them. A later collection that finds the check `present` clears the earlier gap.

## External tools

Built-in programs are offered only when they are on `PATH`. Open the Tools screen or run `os-int doctor` to see what was found.

| Tool | Program | Used on |
| --- | --- | --- |
| `whois` | whois | domain, ip, asn |
| `dig` | dig | domain |
| `dig-ptr` | dig | ip |
| `host` | host | domain, ip |
| `nslookup` | nslookup | domain, ip |
| `curl-headers` | curl | domain, url |
| `openssl-cert` | openssl | domain, url |
| `exiftool` | exiftool | file |
| `git-ls-remote` | git | repository |
| `gh-repo` | gh | repository |
| `subfinder` | subfinder | domain |
| `amass` | amass | domain (`enum -passive`) |

The repository and file collectors call `git`, `gh`, and `exiftool` directly when they need them. Subdomain expansion calls `subfinder` and `amass` when they are installed. The Tools screen also lists utilities that are useful to know about (`jq`, `rg`, `fd`, `wget`, `curl`, `openssl`).

Hide a built-in tool with `tools.disabled`. Add your own under `[[tools.custom]]`. Arguments are an argv template, never a shell command. Placeholders are substituted inside a single argument:

`{target}` `{domain}` `{host}` `{ip}` `{url}` `{email}` `{username}` `{repo}` `{file}` `{asn}`

```toml
[[tools.custom]]
name = "subfinder"
program = "subfinder"
args = ["-silent", "-d", "{domain}"]
accepts = ["domain"]
description = "Passive subdomain discovery"
timeout_seconds = 120
parser = "hostnames"     # text, json, lines, hostnames, ips, or urls
activity = "passive"
```

`activity` must be `passive`. An entry marked `active` is refused and shown as a configuration problem on the Tools screen. Values that look like common secrets are redacted in job output and logs. Redaction is best-effort: prefer not to put tokens in argument lists.

Output parsers:

| Parser | Result |
| --- | --- |
| `text` | The whole output, kept as text |
| `json` | Parsed as JSON when it is valid |
| `lines` | One record per line |
| `hostnames` | Hostnames extracted from the output |
| `ips` | IP addresses extracted from the output |
| `urls` | URLs extracted from the output |

## Asking an agent

os-int does not ship a model and does not call a provider API. It runs a local CLI you install yourself.

| Profile id | Program | How it is launched |
| --- | --- | --- |
| `claude` (default) | `claude` | `claude -p` with stream-json output, `--tools ""`, and the context pack on stdin |
| `codex` | `codex` | `codex exec --json` in a read-only sandbox, prompt on stdin |
| anything else | whatever you set | A generic command, with adapter `text`, `claude`, or `codex` |

`--tools ""` on Claude Code disables that CLI's built-in tools, so text inside a web page cannot make the agent run commands or read files. Codex is launched with `--sandbox read-only`.

Press `a`, type a question, and press `Enter`. os-int writes a context pack that includes the investigation, the page you are on, related entities, findings, and sources. Untrusted material (HTTP bodies, DNS TXT, tool output, repository text) is sanitized for the terminal and wrapped like this:

```text
<<<UNTRUSTED source=S-12 nonce=…>>>
…collected text…
<<<END UNTRUSTED nonce=…>>>
```

The nonce is random per request, and fence-like text inside the material is neutralized, so the collected text cannot close the fence early. Trusted instructions travel separately, as the system prompt, when the agent profile supports it. The exact pack is saved under `investigations/<uuid>/agent/` before the process starts.

Earlier turns of the same thread are included, up to `agent.history_turns` (default 4). The pack is truncated to `agent.max_context_bytes` (default 200 KiB).

If the agent wants os-int to change the case, it appends a fenced JSON block. os-int stores those as proposals and shows them to you. Nothing is written until you accept.

````text
```os-int-actions
[
  {"action":"ADD_FINDING","title":"Apex redirects to www","details":"…","confidence":"high","sources":["S-1"]},
  {"action":"ADD_NOTE","body":"…"},
  {"action":"CREATE_PAGE","title":"www.example.com","page_type":"domain","target":"www.example.com"},
  {"action":"LINK_ENTITY","from":"E-1","to":"E-4","relation":"resolves_to","sources":["S-1"]},
  {"action":"RUN_COLLECTOR","collector":"dns","target":"www.example.com"},
  {"action":"GENERATE_REPORT","format":"markdown"},
  {"action":"RUN_PLAYBOOK","playbook":"domain-infra"},
  {"action":"ASSERT_FINDING_TEMPLATE","title":"Review domain infrastructure","details":"…"}
]
```
````

Accepted findings keep provenance `agent` and the `AI` badge.

Switch profiles in Settings, or in `config.toml`:

```toml
[agent]
active = "codex"
timeout_seconds = 600

[agents.opencode]
name = "opencode"
program = "opencode"
args = ["run", "{prompt}"]
adapter = "text"          # text, claude, or codex
input = "argument"        # stdin, argument, or file
working_directory = "investigation"
```

Placeholders inside agent arguments: `{prompt}`, `{prompt_file}`, `{instructions}`, `{question}`, `{workdir}`. They are substituted inside individual arguments and are never re-split through a shell.

If the program is missing, the Agent screen explains that, and collection continues to work. Point `active` at a program that is installed, or set `program` under `[agent]` to override the selected profile.

## Reports, export, and import

From the palette, choose **Generate Report**. From the shell:

```bash
os-int report "Example Corp"                      # Markdown, path printed
os-int report "Example Corp" --format json
os-int report "Example Corp" -o ./example-corp.md
```

A Markdown report includes the description as an executive summary, scope (pages), diagnostics, findings with provenance and source ids, entities, infrastructure relationships, observation changes, the timeline, and an evidence section. The report states that AI-written analysis is unverified inference.

JSON is the same investigation rendered for another program to read.

Export writes a directory you can copy, archive, or hand to someone else:

```bash
os-int export "Example Corp" -o ./example-corp
os-int import ./example-corp
os-int import ./example-corp --rename
```

The directory contains `investigation.osint.json` (format `os-int-investigation`, version 3) plus a `files/` tree of the investigation's snapshots, notes artifacts, and reports. Paths inside the manifest are relative. Version-1 and version-2 archives still import (empty `data.resolutions` / `data.events`). Import refuses an archive whose investigation id already exists, unless you pass `--rename`, which assigns a new investigation id, remaps every record UUID (including resolution bags and Events), and copies the files under the new id. `--format geojson` writes a FeatureCollection instead of an archive. The JSON fields, ingest batch, and version-bump rules are in [Ontology Exchange Format](ontology-exchange.md).

Without `-o`, export uses `<sanitized-name>-<uuid>` in the current directory.

## Command-line reference

```text
os-int [OPTIONS] [COMMAND]
```

| Option | Environment | Effect |
| --- | --- | --- |
| `--config <file>` | `OS_INT_CONFIG` | Config file to read or create |
| `--data-dir <dir>` | `OS_INT_DATA_DIR` | Data directory |
| `--debug` | `RUST_LOG` | Debug-level tracing in the log file |
| `--version`, `--help` | | Version and command help |

| Command | Purpose |
| --- | --- |
| *(none)* | Open the workbench on the last investigation, or the welcome screen |
| `open <name>` | Open that investigation in the workbench |
| `new <name> [--description <text>]` | Create an investigation and open it |
| `list` | Print investigations and counts |
| `collect <kind> [target]` | Run collectors. `<kind>` may be a target value to guess from |
| `collect --investigation <name>` | Case to write into. Created when the name is new |
| `collect --collector <id>` | Run one collector instead of the defaults |
| `ingest <path> [--format stix\|misp\|csv\|json\|batch]` | Import a file into a case |
| `playbook list` | Print bundled playbooks |
| `playbook validate <id-or-file>` | Parse YAML without writing |
| `playbook run <id-or-file> --investigation <name> --target <seed>` | Run a playbook (`--dry-run` plans only) |
| `watch --playbook <id-or-file> --investigation <name>` | Re-run collect steps (`--dry-run`, `--once`, `--interval`) |
| `report <name> [--format markdown\|json] [-o <file>]` | Write a report |
| `export <name> [-o <dir>] [--format archive\|geojson]` | Write a portable directory, or GeoJSON |
| `geojson <name> [-o <file>]` | Write a GeoJSON FeatureCollection |
| `cooccur <name> [--window <hours>]` | Print pairs that share an Event, source, or time window |
| `import <path> [--rename]` | Import an export directory |
| `doctor` | Check the database, config, network, logs, and common programs |
| `config` | Print the resolved config, data, database, and log paths |

`collect` target kinds: `domain`, `ip`, `url`, `email`, `repository`, `file`, `username`, `asn`. Aliases such as `host`, `hostname`, `repo`, and `as` are accepted.

## Configuration

A commented file is written the first time os-int runs. Every key is optional. Settings changed from the Settings screen are saved back to the same file; comments in that file are not preserved on save.

```toml
[ui]
# "default", "high-contrast", or "monochrome". Unknown values fall back to default.
theme = "default"
# Redraw interval in milliseconds while jobs are running. The UI is otherwise event-driven.
tick_rate_ms = 250
# editor = "nvim"

[agent]
active = "claude"
timeout_seconds = 600
max_context_bytes = 204800
history_turns = 4

# Inline profile. When program is set, it overrides `active`.
# name = "Claude Code"
# program = "claude"
# args = ["-p", "{prompt}"]
# adapter = "claude"
# input = "stdin"
# working_directory = "investigation"

[network]
# user_agent defaults to: os-int/<version> (+https://github.com/pjourney/osint-rat)
request_timeout_seconds = 20
connect_timeout_seconds = 10
max_body_bytes = 5242880
max_redirects = 10
# proxy = "socks5h://127.0.0.1:9050"
allow_private_redirects = false

[storage]
# data_dir = "~/osint-data"
max_file_import_bytes = 536870912

[collectors]
fetch_robots = true
check_www = true
max_concurrent_jobs = 4
repository_history_depth = 200
max_subdomains = 80
max_username_profiles = 8
max_ingest_bytes = 20971520
# geo_db_path = "/var/lib/os-int/city.json"

[tools]
# disabled = ["dig"]
```

| Key | Default | Meaning |
| --- | --- | --- |
| `ui.theme` | `default` | `default`, `high-contrast`, or `monochrome` |
| `ui.tick_rate_ms` | `250` | Redraw interval while jobs are running |
| `ui.editor` | unset | Long-form editor. Then `$VISUAL`, `$EDITOR`, `vi` |
| `agent.active` | `claude` | Profile id: `claude`, `codex`, or a key under `[agents]` |
| `agent.timeout_seconds` | `600` | Agent process timeout. `0` is warned about and would never time out |
| `agent.max_context_bytes` | `204800` | Largest context pack sent with a question |
| `agent.history_turns` | `4` | Earlier turns of the same thread included with a follow-up |
| `agent.program` | unset | When set, this inline profile replaces `active` |
| `network.user_agent` | `os-int/<version> (+repo)` | HTTP User-Agent |
| `network.request_timeout_seconds` | `20` | HTTP request timeout |
| `network.connect_timeout_seconds` | `10` | HTTP connect timeout |
| `network.max_body_bytes` | `5242880` | Bytes kept from one HTTP body |
| `network.max_redirects` | `10` | Redirects followed for one HTTP fetch |
| `network.proxy` | unset | Proxy for HTTP(S) only. Example: `socks5h://127.0.0.1:9050`. DNS and TLS connect directly |
| `network.allow_private_redirects` | `false` | Allow a public URL to redirect to a private or loopback address |
| `storage.data_dir` | platform dir | Data directory, after flags and environment variables |
| `storage.max_file_import_bytes` | `536870912` | Largest evidence file that will be copied in (512 MiB) |
| `collectors.fetch_robots` | `true` | Fetch `/robots.txt` and discover sitemaps during HTTP collection |
| `collectors.check_www` | `true` | Also resolve `www.<domain>` during DNS collection |
| `collectors.max_concurrent_jobs` | `4` | How many background jobs run at once |
| `collectors.repository_history_depth` | `200` | Commits inspected for a repository |
| `collectors.max_subdomains` | `80` | Hostnames promoted during subdomain expansion |
| `collectors.max_username_profiles` | `8` | Public profile URL checks in the username pack |
| `collectors.max_ingest_bytes` | `20971520` | Largest STIX / MISP / CSV / batch file `os-int ingest` will read (20 MiB) |
| `collectors.geo_db_path` | unset | Optional local JSON or MMDB for IP coordinates. Missing files degrade; no network |
| `collectors.external` | empty | Sidecars that print Evidence Batch JSON. See [connectors.md](connectors.md) |
| `collectors.api` | empty | Optional URLScan / OTX wrappers. Off until enabled |
| `tools.disabled` | empty | Built-in tool names to hide |
| `tools.custom` | empty | Extra passive programs. See [External tools](#external-tools) |

Agent profile keys under `[agents.<id>]` or the inline `[agent]` profile:

| Key | Meaning |
| --- | --- |
| `name` | Label shown in the UI |
| `program` | Executable name or path |
| `args` | Argument vector with `{prompt}`-style placeholders |
| `adapter` | `claude` (stream-json), `codex` (`exec --json`), or `text` |
| `input` | `stdin` (default), `argument`, or `file` |
| `working_directory` | `investigation` (default), `current`, or an absolute path |
| `timeout_seconds` | Overrides the global agent timeout for this profile |
| `env` | Extra environment variables for the process |

os-int logs a warning at startup for an unknown theme, a timeout of `0`, or a custom tool missing `name` or `program`.

## Keyboard

| Key | Where | Action |
| --- | --- | --- |
| `q` | normal, overlays | Quit, or close the overlay |
| `Ctrl+C` | anywhere | Quit |
| `Esc` | normal | Back |
| `Esc` | overlays, editing, proposals | Close or cancel |
| `Tab` / `Shift+Tab` | normal | Next / previous pane |
| `j` `k` or arrows | normal, lists | Move the selection |
| `PgUp` `PgDn` | normal | Move by 10 |
| `Enter` | normal | Open or select |
| `/` `Ctrl+F` | normal | Search |
| `p` `Ctrl+P` | normal | Command palette |
| `a` | normal | Ask the agent |
| `n` | normal | New investigation, page, or finding |
| `e` | normal | Edit notes, or open settings |
| `r` | normal | Run collectors for this page |
| `i` | normal | Ingest a STIX / MISP / CSV / JSON / Evidence Batch file |
| `b` | normal | Run playbook (bundled YAML or a file) |
| `v` | normal | Object View: highlighted entity, or the current page |
| `g` | normal | Graph for the current entity (1 hop) |
| `s` | normal | Search Around (default 2 hops; merges on Graph) |
| `+` `-` | Graph | More / fewer hops |
| `[` `]` | Graph | Move the facet cursor |
| `f` | Graph | Apply or toggle the highlighted facet |
| `x` | Graph | Clear facet filters |
| `[` `]` | Timeline | Move the entity-filter cursor |
| `f` | Timeline | Filter overlay by the highlighted entity |
| `x` | Timeline | Clear the entity filter |
| `l` | Timeline, Entities | Nearby (same country or prefix) |
| `Space` | Entities | Mark or unmark for merge |
| `m` | Entities | Propose merge of marked entities |
| `u` | Object View, Entities | Propose unresolve |
| `Enter` | Entities | Open Object View |
| `Enter` | Graph | Open Object View for the selected node |
| `c` | Jobs | Cancel the selected job |
| `?` | normal, overlays | Help |
| `1` … `6` | normal | Overview, Pages, Sources, Findings, Entities, Timeline |
| `7` `8` `9` `0` | normal | Agent, Jobs, Tools, Settings |
| `o` | welcome | Open investigation |
| `d` | welcome | Doctor |
| `A` `y` `Enter` | proposals | Accept the selected proposal |
| `R` `n` | proposals | Reject the selected proposal |
| `Enter` | editing | Submit the form or the question |
| `Left` `Right` | forms | Cycle a choice (page type, and similar fields) |

Inside a text field, letters insert text. They are not treated as commands until you leave the field.

## Troubleshooting

| What you see | What to do |
| --- | --- |
| `os-int: command not found` | `cargo install --path .` puts the binary in `~/.cargo/bin`. Add that directory to `PATH`, or run `~/.cargo/bin/os-int`. |
| The welcome screen says Claude Code or Codex is missing | Collection still works. Install the CLI, or set `[agent] active` to a program that is on `PATH`. Confirm with `os-int doctor`. |
| `doctor` exits with an error | A check landed in `warn`, usually the database integrity check. The path is in the doctor output. Missing optional programs are reported as `missing` and do not by themselves fail doctor. |
| You cannot find the database or logs | `os-int config` prints every resolved path. |
| A collector fails for one target and the others succeed | Open Jobs, or re-run with `--collector`. The log file has the underlying error. Pass `--debug` and try again. |
| HTTP collection stops on a redirect to a private address | That refusal is intentional. Set `network.allow_private_redirects = true` only when you mean to follow that redirect. |
| `unknown collector` | Use `dns`, `rdap`, `http`, `tls`, `ct`, `ip`, `repository`, `file`, `email`, `username`, `subdomain`, or a configured external id. |
| Repository collection is unavailable | Install `git`. `gh` is optional and only adds GitHub metadata. |
| The agent runs but proposes nothing | You can still use the answer as reading. Proposals appear only when the reply contains an `os-int-actions` block. Accepting is a separate step. |
| A custom tool never runs | On the Tools screen, read the problem line. Typical causes: the program is not on `PATH`, `accepts` lists no known target kind, the argument template is invalid, or `activity` is `active`. |
| The interface looks cramped | Widen the terminal. The detail pane appears at 96 columns or more. |

Logs are daily files under the data directory. Redact anything sensitive before you attach a log to a bug report. The [bug report template](../.github/ISSUE_TEMPLATE/bug_report.md) asks for `os-int doctor` output.

## Using it responsibly

os-int stores data on the machine that runs it. Built-in collectors fetch public records you ask for. Subprocesses are started with an explicit argument list, a timeout, a bounded output buffer, and a process-group kill on cancel. They are not started through a shell.

Do not use os-int to access systems you are not authorized to investigate, to harass people, or to collect personal data without a lawful basis. You are responsible for the law, the terms of service, and the organizational rules that apply to your work.

To report a vulnerability in os-int itself, follow [SECURITY.md](../SECURITY.md). Please use a private report rather than a public issue.

## See also

- [README](../README.md) — install and a first session
- [Examples](examples.md) — copy-paste investigations, tools, agents, and export
- [FAQ](faq.md) — pages versus entities, badges, paths, and the cases that look like failures
- [Ontology](ontology.md) — object types, link verbs, Actions, Object View
- [Database tables](schema.md) — what each SQLite table stores
- [Connectors](connectors.md) — Evidence Batch sidecars, STIX/MISP/CSV ingest, API keys
- [Playbooks](playbooks.md) — YAML schema, bundled files, watch sets, caps
- [Ontology Exchange Format](ontology-exchange.md) — export JSON, EvidenceBatch, and version bumps
- [ARCHITECTURE.md](ARCHITECTURE.md) — module map and design decisions
- [CONTRIBUTING.md](../CONTRIBUTING.md) — development setup and pull requests
- [SECURITY.md](../SECURITY.md) — vulnerability reports
