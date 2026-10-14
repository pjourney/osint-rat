# Playbooks and automations

A **playbook** is a YAML document: a seed object type plus ordered steps (`collect`, `search_around`, `run_tool`, `assert_finding_template`, `propose_resolve`). os-int runs the collect and tool steps, Search Around is a graph query, and finding templates / merges become **pending proposals**. Nothing from a template is written until you accept it. Automations re-run collect steps and propose Actions on changes; they do not silently write findings.

Built-in collectors stay **passive-only**. Tools marked `active` are refused. Nested playbooks are refused. `RUN_PLAYBOOK` from an agent may only name a bundled id.

## YAML

```yaml
format: os-int-playbook
format_version: 1
id: domain-infra
name: Domain infrastructure
description: Map DNS, registration, HTTP, TLS, CT, and subdomains.
seed:
  type: domain
steps:
  - collect: dns
  - collect: rdap
  - search_around:
      hops: 2
  - run_tool: whois
  - assert_finding_template:
      title: Review domain infrastructure
      details: Accept only after checking the graph.
  - propose_resolve:
      reason: Same registrable domain or shared certificate SAN cluster
```

`format` must be `os-int-playbook`. `format_version` newer than **1** is refused. This version is **not** the investigation archive `FORMAT_VERSION` (currently **2**).

| Step | What happens |
| --- | --- |
| `collect` | Runs a passive collector. The batch is applied in one transaction with the rest of the run. |
| `run_tool` | Runs a registered **passive** tool. Missing tools are skipped with a warning. Active tools fail the playbook. |
| `search_around` | Query only. Caps match Graph / Search Around. |
| `assert_finding_template` | Pending `ASSERT_FINDING_TEMPLATE`. Accept writes the finding (`analyst` if the playbook proposed it, `agent` if an agent did). |
| `propose_resolve` | Pending `PROPOSE_RESOLVE` when suggestion rules fire. Never auto-merges. |

Forbidden in YAML (clear error, no database writes): `provenance`, `resolved_findings`, `include`, `run_playbook`, path traversal (`..`), custom YAML tags, merge keys, alias bombs. Files are capped at 64 KiB. Directories are refused. Parsing does not panic.

## Bundled playbooks

Checked in under [`playbooks/`](../playbooks/) and compiled into the binary:

| Id | Seed | Collects |
| --- | --- | --- |
| `domain-infra` | domain | dns, rdap, http, tls, ct, subdomain; optional `whois` |
| `mail-policy` | domain | dns, rdap |
| `cert-estate` | domain | ct, tls, dns |
| `repo-surface` | repository | repository |

## CLI

```bash
os-int playbook list
os-int playbook validate domain-infra
os-int playbook validate path/to/custom.yaml
os-int playbook run domain-infra --investigation "Example Corp" --target example.com
os-int playbook run domain-infra --investigation "Example Corp" --target example.com --dry-run
os-int watch --playbook domain-infra --investigation "Example Corp" --target example.com --dry-run
os-int watch --playbook domain-infra --investigation "Example Corp" --target example.com --once
os-int watch --playbook domain-infra --investigation "Example Corp" --interval 3600
```

`--dry-run` prints planned Actions and applies nothing. It does not create an investigation or investigation directories; the case must already exist (missing names are an error). `--once` runs one watch pass. `--interval` loops until Ctrl+C (cancellation is observed during a pass, not only between intervals). Collect batches apply in **one SQLite transaction** after the steps finish; a cancelled or failed collect applies nothing.

## Workbench

- `b` or palette **Run playbook**. Left/Right cycles bundled ids. Mode `run` / `dry-run`. Esc closes the form and leaves the entity selection in place. Enter on dry-run shows the plan as text; Enter on run queues a job (cancel from Jobs with `c`).
- Palette **Playbook prompt** seeds the agent question with the playbook's suggested Actions. The agent still only proposes.
- Palette **Watch playbook** is the same form with mode `dry-run` / `once`.

## Caps and loops

| Cap | Limit |
| --- | --- |
| File size | 64 KiB |
| Steps | 32 |
| Collect steps | 16 |
| Tool steps | 8 |
| Search Around hops | 8 (same as Graph) |
| Proposals per run | 16 |
| Nested playbooks | refused |
| Concurrent playbook jobs | one (workbench and CLI; further runs are refused) |

Untrusted playbooks cannot set provenance or delete collected findings (`resolved_findings` is refused at parse, and ingest-style honors rules still apply on collector batches). Resolution bags are unchanged: collect upserts by `(kind, value)` onto the canonical.

Playbook runs and watch rows live in SQLite (`playbook_runs`, `automations`) and are **not** exported. Timeline kind `playbook_run` is additive (no archive `FORMAT_VERSION` bump).

## See also

- [User guide — playbooks](guide.md#playbooks)
- [Ontology — Actions](ontology.md)
- [Architecture](ARCHITECTURE.md)
