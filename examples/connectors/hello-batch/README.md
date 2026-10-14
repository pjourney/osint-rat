# hello-batch

A sidecar that prints [Ontology Exchange](../../../docs/ontology-exchange.md) **Evidence Batch** JSON on stdout. os-int applies that JSON through the same `storage::apply` path as a built-in collector. You do not fork the Rust crate.

Protocol, config, and caps: [docs/connectors.md](../../../docs/connectors.md).

## Run it by hand

```bash
python3 hello_batch.py --target example.com --kind domain
```

Stdout is an envelope:

```json
{
  "format": "os-int-evidence-batch",
  "format_version": 1,
  "batch": { "sources": [], "entities": [], "relationships": [] }
}
```

Logs belong on stderr. Active probing is refused: keep `activity = "passive"`.

## Wire it in `config.toml`

```toml
[[collectors.external]]
id = "hello-batch"
label = "Hello batch"
description = "Example sidecar that emits an Evidence Batch"
program = "python3"
args = [
  "examples/connectors/hello-batch/hello_batch.py",
  "--target", "{target}",
  "--kind", "{kind}",
  "--id", "{id}",
]
accepts = ["domain", "email", "username"]
timeout_seconds = 30
activity = "passive"
runs_by_default = false
```

Use an absolute path for `program` / the script if the workbench's working directory is not the repository root. Placeholders `{target}`, `{kind}`, `{id}`, and `{value}` are substituted inside a single argument; nothing is passed through a shell.

Then either:

```bash
os-int collect domain example.com --collector hello-batch --investigation "Example Corp"
```

or open the command palette and run **Hello batch** (the collector id). Tools or sidecars marked `active` are refused.

## Ingest the JSON as a file

```bash
python3 hello_batch.py --target example.com --kind domain > /tmp/hello.json
os-int ingest /tmp/hello.json --format batch --investigation "Example Corp"
```

Re-ingest of identical content is idempotent: the source is marked seen again instead of duplicated.
