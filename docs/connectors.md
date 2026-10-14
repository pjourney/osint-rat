# Connectors

Anything an analyst already has can become cited ontology objects and links. Collectors still return an [`EvidenceBatch`](ontology-exchange.md#evidence-batch-collector--future-connector-payload). `storage::apply` writes it in one transaction. This page is the external / file ingest contract.

Built-in collectors stay **passive-only**. Sidecars and tools marked `active` are refused. AI only proposes Actions. File and sidecar ingest always stores sources and findings as provenance `external`. A batch cannot claim `agent`, `analyst`, or `collected`. `resolved_findings` is ignored on this path (built-in collectors still use it to clear collected diagnostics). Unsafe `snapshot_path` / artifact `path` values are rejected.

## Evidence Batch envelope

Sidecar stdout (and `os-int ingest --format batch`) is JSON:

```json
{
  "format": "os-int-evidence-batch",
  "format_version": 1,
  "batch": {
    "sources": [],
    "entities": [],
    "relationships": [],
    "timeline": [],
    "events": [],
    "artifacts": [],
    "findings": [],
    "resolved_findings": [],
    "summary": ""
  }
}
```

A raw batch object (no envelope) is accepted. `format_version` newer than **1** is refused. An `os-int-investigation` archive is refused here — use `os-int import`. The investigation archive `FORMAT_VERSION` (currently **4**) is a different constant and is not used on this envelope. Optional `batch.events` (instants or intervals) is additive and does not bump this envelope.

Field meanings, `SourceKey` indexes, upsert keys, and duplicate-source behaviour: [Ontology Exchange Format](ontology-exchange.md). Re-ingest of identical collector + title + URL + content hash is idempotent.

Caps (refused before a full decode when possible):

| Cap | Default |
| --- | --- |
| File / sidecar stdout | `collectors.max_ingest_bytes` (20 MiB) for files; 2 MiB stdout for sidecars |
| Objects / rows per file | 10_000 |
| Entity value length | 2_048 characters |

Malformed JSON, truncated UTF-16, a missing UTF-8 encoding, an empty file, or a directory is a structured error. Parsing does not panic. Skipped rows become warnings.

UTF-8 (BOM stripped) and UTF-16 (LE/BE BOM) are accepted.

## `[[collectors.external]]`

A sidecar program. os-int spawns it through `exec/` with an explicit argv (no shell). Stdout is parsed as an Evidence Batch.

```toml
[[collectors.external]]
id = "hello-batch"
label = "Hello batch"
description = "Example sidecar"
program = "python3"
args = [
  "examples/connectors/hello-batch/hello_batch.py",
  "--target", "{target}",
  "--kind", "{kind}",
  "--id", "{id}",
]
accepts = ["domain"]
timeout_seconds = 30
activity = "passive"
runs_by_default = false
# working_directory = "/absolute/path"
```

Placeholders inside a single argument: the usual tool set (`{target}`, `{domain}`, …) plus `{kind}`, `{value}`, and `{id}`. `activity` must be `passive`. Empty `id` / `program`, unknown `accepts`, or `activity = "active"` is refused (startup warning; the collector is not registered).

In-repo example: [`examples/connectors/hello-batch/`](../examples/connectors/hello-batch/). It produces `subdomain_of` / `resolves_to` (or `hosted_on` / `associated_with`) without any Rust change. To publish a new sidecar or playbook, copy [`examples/community/`](../examples/community/) and follow [plugins.md](plugins.md).

## Built-in file ingest

```bash
os-int ingest path/to/bundle.json --format stix --investigation "Example Corp"
os-int ingest event.json --format misp
os-int ingest observables.csv --format csv --map kind=type,value=indicator
os-int ingest observables.json --format json --kind domain
os-int ingest batch.json --format batch
```

`--format` is `stix`, `misp`, `csv`, `json`, or `batch`. When omitted, the file name is guessed (`stix` / `misp` in the name, `.csv`, `batch` / `.osint.json`, otherwise `.json` as observables). `--kind` is the default entity kind for a single-column CSV. `--map` is `kind=COLUMN,value=COLUMN` (names or 0-based indexes).

In the workbench: `i` or palette **Ingest file**. Left/Right cycles the format. Esc closes the form and leaves the entity selection in place. Enter applies through `storage::apply` and restores that selection (canonical id after a merge).

| Format | What is imported |
| --- | --- |
| STIX 2.1 | Bundle (or a single SDO): `indicator`, `observed-data`, `malware`, `identity`, `relationship`, and top-level cyber observables (`domain-name`, `ipv4-addr`, …). Other types are skipped with a warning. |
| MISP | Event JSON (`{"Event": …}` or a bare event). Attributes become objects; email → domain gets `hosted_on`. Unsupported types are skipped. |
| CSV / JSON | Observables with column mapping. Email rows also emit `hosted_on`. |
| Batch | Envelope or raw `EvidenceBatch`. |

Every imported source and finding is cited on the objects and links it produced. Provenance is `external` (never `collected` or `agent`).

### STIX 2.1 export

`os-int export CASE --format stix -o bundle.json` writes a STIX 2.1 bundle (not the investigation archive; `FORMAT_VERSION` is unchanged). Re-ingest with `os-int ingest bundle.json --format stix` into a second empty case.

| os-int | STIX 2.1 | Notes |
| --- | --- | --- |
| Domain / hostname | `domain-name` | Hostname sets `x_osint_kind=hostname` so import can restore the kind |
| IP | `ipv4-addr` / `ipv6-addr` | Colon in the value selects IPv6 |
| Email / URL / username | `email-addr` / `url` / `user-account` | Repository also exports as `url` |
| ASN | `autonomous-system` | |
| File | `file` | |
| Certificate | `x509-certificate` | SHA-256 in `hashes` |
| Person / organization | `identity` | `identity_class` individual / organization |
| Unknown with `stix_type=malware` | `malware` | |
| Finding | `indicator` | Pattern from a cited entity, or `[file:name = 'title']` |
| Event with entity links | `observed-data` | `first_observed` / `last_observed` |
| Relationship | `relationship` | `resolves_to` → `resolves-to`; others `related-to` plus `x_osint_relationship` |
| TLP on the case or object | `marking-definition` + `object_marking_refs` | Official OASIS TLP UUIDs. Each SDO/SCO/relationship gets `max(case, entity)`. TLP:CLEAR maps to TLP:WHITE. TLP:AMBER+STRICT is a statement marking with `x_osint_marking`. Import restores object markings. |

**Documented gaps (acceptable fidelity):** pages, notes, artifacts, jobs, proposals, resolution bags, audit, members, and per-source collector provenance do not have STIX equivalents. Re-import stores one `SourceKind::Stix` source with provenance `external` (ingest rule). `x_osint_kind` on observables and identities, `x_osint_relationship` on links, and `x_osint_marking` / `object_marking_refs` are restored; other `x_osint_*` properties stay in the JSON only. Absorbed resolution constituents are omitted (canonical only). Case-level TLP is not written onto the destination investigation row (objects carry the restriction).

Sample fixtures: [`tests/fixtures/ingest/`](../tests/fixtures/ingest/).

## High-signal built-in collectors

Still passive. They return a batch; they do not write SQLite.

| Id | Accepts | Default | What it records |
| --- | --- | --- | --- |
| `email` | email | yes | Local-part username, hosted domain, `hosted_on` / `associated_with`. No invented website URL. No network. |
| `username` | username | yes | Public profile URL GETs (GitHub, GitLab, Codeberg, Reddit, Hacker News, Keybase). 2xx → `associated_with`. 3xx is not a hit. Cap: `collectors.max_username_profiles` (8). |
| `subdomain` | domain | yes | Common prefixes plus optional `subfinder` / `amass -passive` via the tool registry. DNS A/AAAA. Catalog verbs `subdomain_of` and `resolves_to` (not `mentions`). Cap: `collectors.max_subdomains` (80). |
| RDAP | domain, ip, asn | yes | Unchanged. When RDAP is missing or thin (no nameservers / contacts / registration edges), a structured `whois` parse is merged if `whois` is on `PATH`. |

Custom tool parsers (`hostnames`, `ips`, `urls`, `lines`) promote the same catalog verbs: a name under the apex is `subdomain_of`, an address on a domain/URL target is `resolves_to`, a URL is `references`. Other names stay `mentions`.

## Optional API connectors

Off until enabled, and never in the default “run all” set even when enabled. Fetches go through the shared `NetContext` client (cancel + public-to-private redirect policy). Keys from `api_key_env` (preferred) or `api_key`. Keys are redacted in source content and job command lines.

```toml
[[collectors.api]]
id = "urlscan"
enabled = true
api_key_env = "URLSCAN_API_KEY"
rate_limit_per_minute = 30

[[collectors.api]]
id = "otx"
enabled = true
api_key_env = "OTX_API_KEY"
```

Built-in ids: `urlscan` (search), `otx` (indicator general). Unknown ids are skipped with a warning. OTX without a key is skipped. Inline `api_key` in the config file is warned about at startup.

## Resolution bags

Ingest upserts entities on `(kind, canonical value)`. A merge bag is not rewritten. Sources attach to the existing row (the canonical when that value was the winner). Object View still unions constituent sources. Unresolve restores property/source writes; `os-int doctor` / `PRAGMA foreign_key_check` stay clean. Identical re-ingest does not duplicate the source.

## See also

- [Ontology Exchange Format](ontology-exchange.md)
- [Playbooks](playbooks.md)
- [User guide — collectors](guide.md#collectors)
- [User guide — ingest](guide.md#ingest)
- [Architecture](ARCHITECTURE.md)
- [hello-batch example](../examples/connectors/hello-batch/)
