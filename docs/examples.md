# Examples

Copy-paste sessions for the workflows people repeat. Each one assumes `os-int` is on your `PATH`. The [user guide](guide.md) explains every flag these commands use. The [FAQ](faq.md) answers the questions that usually come up halfway through.

Use a throwaway directory while you learn, so these examples stay out of your real cases:

```bash
mkdir -p /tmp/os-int-learn
alias os-int-learn='os-int --data-dir /tmp/os-int-learn --config /tmp/os-int-learn/config.toml'
```

Drop the alias when you want the platform data directory again. `os-int-learn config` prints the paths this alias will use.

## Collect a domain, then read it

`collect` creates the investigation and the page, runs the default collectors, and returns to the shell. For a domain those collectors are DNS, RDAP, HTTP, TLS, and certificate transparency.

```bash
os-int-learn collect domain example.com --investigation "Example Corp"
os-int-learn list
os-int-learn report "Example Corp"
os-int-learn open "Example Corp"
```

Inside the workbench:

| Key | What you are looking at |
| --- | --- |
| `1` | The `example.com` page: target, entities, findings |
| `3` | Sources. `S-1` onward are the raw observations |
| `4` | Findings. `[COL]` marks a collected diagnostic |
| `5` | Entities such as the domain, its addresses, and its nameservers |
| `6` | Timeline, including registration dates and later observation changes |
| `8` | Jobs, if a collector is still running or a previous one failed |
| `/` | Search. Try `example.com` or a short id such as `E-1` |

Press `q` to leave the workbench. The case is already saved.

## Run one collector

Use `--collector` when you want a narrower pass. Ids are `dns`, `rdap`, `http`, `tls`, `ct`, `ip`, `repository`, and `file`.

```bash
os-int-learn collect example.com --investigation "Example Corp" --collector dns
os-int-learn collect url https://example.com/ --investigation "Example Corp" --collector http
```

The first command guesses the kind from `example.com`. The second names the kind as `url` because a bare URL is unambiguous only when you pass it as the value.

In the workbench, press `p` and type `dns`, `rdap`, `http`, `tls`, `ct`, or `ip` to run that one collector against the current page.

## Follow an address the domain just revealed

After the domain collection, open Entities (`5`), note an address, and collect it. An IP runs RDAP and IP intelligence (reverse DNS, origin ASN, prefix, registry country).

```bash
os-int-learn collect ip 192.0.2.15 --investigation "Example Corp"
```

Or, in the workbench, add a page (`n`, type **IP address**, enter the address) and press `r`.

The new page shares the investigation with the domain, so relationships such as `resolves_to` and `announced_by` show up on both.

## Inspect a local file

The file collector hashes the file, records its type and timestamps, and adds ExifTool metadata when `exiftool` is on `PATH`. The file is copied into the investigation, up to `storage.max_file_import_bytes` (512 MiB by default).

```bash
os-int-learn collect file ./evidence.pdf --investigation "Example Corp"
```

The page target is the path you passed. The stored artifact lives under the investigation's `files/` directory, content-addressed, so the original path is not required later.

## Look at a public repository

This collector needs `git`. `gh` is optional and adds a GitHub summary when the remote is on GitHub. The clone is shallow, stored inside the investigation, and run with an empty credential helper.

```bash
os-int-learn collect repository https://github.com/owner/project --investigation "Example Corp"
```

## Add a passive tool

Built-in tools (`whois`, `dig`, `curl`, …) appear on the Tools screen when their program is on `PATH`. A program os-int does not know about is a `[[tools.custom]]` entry. Arguments are an argv list. `{domain}` and the other placeholders are replaced inside one argument.

Edit the config file (`os-int-learn config` prints its path):

```toml
[[tools.custom]]
name = "subfinder"
program = "subfinder"
args = ["-silent", "-d", "{domain}"]
accepts = ["domain"]
description = "Passive subdomain discovery"
timeout_seconds = 120
parser = "hostnames"
activity = "passive"
```

Restart os-int. The Tools screen lists the tool, or the reason it will not run (missing program, empty `accepts`, `activity = "active"`). Active tools are refused.

Hide a built-in you do not want offered:

```toml
[tools]
disabled = ["dig"]
```

## Ask the local agent

Collection does not need an agent. When you want one, install Claude Code or Codex and confirm `os-int-learn doctor` sees it. The default profile is `claude`.

Switch to Codex in the config file:

```toml
[agent]
active = "codex"
```

Or press `0` for Settings, then `e`, and set the active profile. From any investigation screen, press `a`, ask a question, and press `Enter`. If the reply contains an `os-int-actions` block, os-int shows each proposed change. `Enter` accepts the selected one. `R` rejects it. An accepted finding keeps the `[AI]` badge.

A generic CLI, with the prompt passed as an argument:

```toml
[agents.myagent]
name = "My agent"
program = "my-agent"
args = ["run", "{prompt}"]
adapter = "text"
input = "argument"

[agent]
active = "myagent"
```

## Write a report and hand the case over

```bash
os-int-learn report "Example Corp" -o /tmp/example-corp.md
os-int-learn report "Example Corp" --format json -o /tmp/example-corp.json
os-int-learn export "Example Corp" -o /tmp/example-corp-export
```

The export directory contains `investigation.osint.json` and a `files/` tree. On another machine, or in another data directory:

```bash
os-int --data-dir /tmp/os-int-other import /tmp/example-corp-export
```

If that data directory already has an investigation with the same id, add `--rename` to import a copy.

## See a case from the shell only

`list` prints a short id, plus counts of pages, sources, findings, and entities. `open` and `report` accept the name, the full UUID, or a unique prefix of it.

```bash
os-int-learn list
os-int-learn report "Example Corp" --format markdown
```

`os-int new "Example Corp"` also creates a case, then opens the workbench and stays there until you quit. Use `collect --investigation` when you want the shell back.

## See also

- [User guide](guide.md)
- [FAQ](faq.md)
- [Database tables](schema.md)
- [Ontology Exchange Format](ontology-exchange.md)
- [Configuration reference](guide.md#configuration)
