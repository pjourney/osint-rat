# Community collector template

Copy this directory when you want to publish an external collector. The program prints [Ontology Exchange](../../../docs/ontology-exchange.md) Evidence Batch JSON on stdout. os-int applies that JSON through the same `storage::apply` path as a built-in collector. You do not fork the Rust crate.

Publishing steps, review bar, and playbook templates: [docs/plugins.md](../../../docs/plugins.md). Protocol and caps: [docs/connectors.md](../../../docs/connectors.md).

## Rules that will not be waived

- `activity = "passive"`. Active tools are refused.
- Provenance on sidecar output is `external`. Do not claim `collected` or `agent`.
- AI-shaped findings stay proposals. A sidecar must not write findings as if an analyst accepted them.
- Stderr is for logs. Stdout is JSON only.

## Try it

```bash
python3 collect.py --target example.com --kind domain
```

Wire it in `config.toml`:

```toml
[[collectors.external]]
id = "community-template"
label = "Community template"
description = "Copy-paste sidecar that emits an Evidence Batch"
program = "python3"
args = [
  "examples/community/template-collector/collect.py",
  "--target", "{target}",
  "--kind", "{kind}",
  "--id", "{id}",
]
accepts = ["domain"]
timeout_seconds = 30
activity = "passive"
runs_by_default = false
```

Then:

```bash
os-int collect domain example.com --collector community-template --investigation "Example Corp"
```
