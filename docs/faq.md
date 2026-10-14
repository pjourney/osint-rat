# FAQ

Short answers to the questions that come up while installing and using os-int. The [user guide](guide.md) has the full behavior, and [examples](examples.md) has copy-paste sessions.

## What is a page, and what is an entity?

A **page** is a subject you chose to work on: "look at example.com". An **entity** is a normalized identifier that showed up, including ones you did not create a page for (a nameserver, an address in a certificate, an email in a page). Several pages can mention the same entity. Collectors link them with relationships such as `resolves_to`.

## What is the difference between a note, a finding, and a source?

| Record | What it is |
| --- | --- |
| Source | One observation: the DNS answer, the HTTP response, the tool output. It has a content hash. |
| Finding | A claim. It should cite sources by short id (`S-1`). A finding with no sources is a hypothesis. |
| Note | Your writing, or text you accepted from an agent. Page notes live on the page. Standalone notes have their own id (`N-1`). |

Press `e` on Overview to edit the current page's notes in your editor. Add a finding from the command palette (`p`, then "finding").

## How do I merge two entities that are the same thing?

On **Entities**, mark each row with `Space`, press `m`, and confirm. The highlighted marked row is the canonical. Object View lists constituents. Graph shows one node. `u` on the canonical unresolves. Suggestions never auto-merge. See [Entity resolution](guide.md#entity-resolution).

## What do the badges mean?

| Badge | Provenance | Meaning |
| --- | --- | --- |
| `[COL]` | collected | A collector or external tool recorded this |
| `[EXT]` | external | Original material you supplied |
| `[ANL]` | analyst | You wrote this |
| `[AI]` | agent | An agent inferred this. Accepting the proposal keeps this badge |

## Does os-int call an LLM API or need an API key?

Built-in collectors use public DNS, RDAP, HTTP, TLS, and the public crt.sh search. They do not use API keys. Optional URLScan / OTX wrappers stay off until you enable them with `api_key_env` (preferred) or `api_key`; keys are redacted in logs. An agent, when you want one, is a program already on your `PATH` (`claude`, `codex`, or a command you configure). os-int runs that program as a subprocess.

## The welcome screen says Claude Code is missing. Is collection broken?

No. The welcome screen and `os-int doctor` report optional programs as `missing`. DNS, RDAP, HTTP, TLS, certificate transparency, IP intelligence, and the email pack run without them. Username checks and subdomain DNS need the network. Repository collection needs `git`. File metadata from ExifTool needs `exiftool`. `subfinder` / `amass` / `whois` are optional extras.

`doctor` exits non-zero when a check is `warn` (usually the database integrity check), not when an optional program is absent.

## Where are my investigations?

Run `os-int config`. It prints the config file, the data directory, the database, and the log directory.

| Platform | Config | Data |
| --- | --- | --- |
| Linux | `~/.config/os-int/config.toml` | `~/.local/share/os-int/` |
| macOS | `~/Library/Application Support/os-int/config.toml` | `~/Library/Application Support/os-int/` |
| Windows | `%APPDATA%\os-int\config.toml` | `%APPDATA%\os-int\` |

`--data-dir` and `OS_INT_DATA_DIR` override the data directory. `--config` and `OS_INT_CONFIG` override the config file.

## How do I start over?

Point os-int at an empty directory. Your existing cases stay where they are.

```bash
os-int --data-dir /tmp/os-int-fresh --config /tmp/os-int-fresh/config.toml
```

Deleting the platform data directory deletes every investigation stored there. Export first if you want a copy.

## Why did `r` not run IP intelligence?

`r` runs the default collectors for the **current page's** kind. A domain runs `dns`, `rdap`, `http`, `tls`, `ct`, and `subdomain`. An email page runs `dns`, `ct`, and `email`. A username page runs `username`. IP intelligence runs on an IP page, or with `os-int collect ip 192.0.2.15`. The full matrix is in the [user guide](guide.md#collectors).

## Why was my custom tool refused?

On the Tools screen, read the problem on that row. os-int refuses the tool when:

- `activity` is `active`
- `name` or `program` is empty
- `accepts` lists no known target kind
- an argument template is invalid

A tool whose program is not on `PATH` is listed and skipped. It is not a configuration error.

## Can I collect without a network?

DNS, RDAP, HTTP, TLS, certificate transparency, IP intelligence, username profile checks, and subdomain expansion need the network. The file collector, the email identity pack, and anything already stored in the case do not. The workbench opens and searches local data with no network. File ingest (`os-int ingest`) is local.

HTTP(S) can use `network.proxy` (for example `socks5h://127.0.0.1:9050`). DNS and TLS collectors connect directly and do not use that proxy.

## What happens if I collect the same target twice?

The same content hash is treated as the same observation: the source is marked seen again and is not duplicated. A new hash writes an "observation changed" timeline event. A diagnostic that is now `present` clears the earlier gap finding.

## I accepted an agent proposal. Why is the finding still marked AI?

That is the record of who asserted it. Accepting means you allowed it into the case. It does not turn the inference into a collected fact. Cite sources in the proposal (`"sources":["S-1"]`) when the claim rests on evidence you already have.

## The agent answered and nothing was added.

os-int applies a change only from an `os-int-actions` JSON block, and only after you accept the proposal. An answer with no such block is reading, not a write. The exact context pack that was sent is under the investigation's `agent/` directory and on the Artifacts screen.

## `os-int new` did not return to the shell.

`new` creates the investigation and opens the workbench until you press `q`. To create a case and keep the shell, use:

```bash
os-int collect domain example.com --investigation "Example Corp"
```

## Import says the investigation already exists.

The archive's id matches one in this data directory. Import a copy with a new id:

```bash
os-int import ./example-corp-export --rename
```

If the name also matches an existing case, os-int adjusts the imported name so the unique name constraint holds.

## Where are the logs?

`<data_dir>/logs/os-int.log.YYYY-MM-DD`. They are never printed into the workbench. `os-int --debug` raises the file log to debug level. `RUST_LOG` overrides the filter. Redact secrets before you attach a log to a bug report.

## Which Rust version do I need?

Rust **1.88** or newer. The crate uses the 2024 edition. SQLite is bundled; you do not install a database separately.

## See also

- [Examples](examples.md)
- [User guide](guide.md)
- [Database tables](schema.md)
- [Security policy](../SECURITY.md)
