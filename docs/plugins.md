# Plugin marketplace (open platform)

os-int treats collectors as plugins. The “marketplace” is this repository plus pull requests: there is no separate store, account, or paid listing. You publish an **external collector** (Evidence Batch sidecar) or a **YAML playbook**. Built-in collectors stay in Rust and stay passive.

## What you can publish

| Plugin | Contract | In-tree starting point |
| --- | --- | --- |
| External collector | Sidecar prints [Evidence Batch](ontology-exchange.md#evidence-batch-collector--future-connector-payload) JSON on stdout | [`examples/community/template-collector/`](../examples/community/template-collector/) and [`examples/connectors/hello-batch/`](../examples/connectors/hello-batch/) |
| Playbook | `format: os-int-playbook`, `format_version: 1` | [`examples/community/template-playbook.yaml`](../examples/community/template-playbook.yaml) and [`playbooks/`](../playbooks/) |

Protocol, caps, and `[[collectors.external]]`: [connectors.md](connectors.md). Playbook schema: [playbooks.md](playbooks.md).

## Review bar

A plugin pull request is accepted only when all of these hold:

1. **Passive-only.** `activity = "passive"`. Active tools and sidecars are refused and will not be merged.
2. **AI only proposes.** Playbook finding templates stay pending. A sidecar must not impersonate analyst or agent provenance.
3. **Provenance is preserved.** Sidecar sources and findings are `external`. Do not relabel collected rows as something else.
4. **Tests.** Parser/apply coverage for a new playbook; for a sidecar, a test that the script emits a batch which `storage::apply` accepts (see `hello_batch_example_applies_without_forking_rust` and `community_plugin_templates_parse`).
5. **No network in CI.** Live collectors stay out of GitHub Actions. Fixtures and documentation-range values (`example.com`, `192.0.2.0/24`) only.

Title the pull request `community-plugin: <id> — <one-line description>` and copy the community templates rather than editing `hello-batch` in place.

## How to publish a collector

1. Copy `examples/community/template-collector/` to `examples/connectors/<id>/`.
2. Keep stdout JSON-only. Logs go to stderr.
3. Document `[[collectors.external]]` in that folder’s README (absolute paths if the workbench cwd is not the repo root).
4. Add a test that runs the program (or a checked-in stdout fixture) and applies the batch.
5. Open the pull request. Maintainers review the bar above; they do not run active probes to “try it”.

`os-int collect … --collector <id>` against a **release binary** is the compatibility target. The sidecar contract is the Evidence Batch envelope (`format_version` 1), not a Rust trait in your process.

## How to publish a playbook

1. Copy `examples/community/template-playbook.yaml`.
2. Use a unique `id` (snake-case). Nested playbooks, `resolved_findings`, and provenance overrides are refused by the parser.
3. Bundled playbooks live under `playbooks/` and are listed by `os-int playbook`. Community experiments can stay under `examples/community/` until they are reviewed into the bundle.
4. Finding templates and `propose_resolve` stay pending. Automations may only propose.

## What will be declined

- Active scanning, exploitation, credential stuffing, or anything `activity = "active"`.
- Silent finding writes, auto-merge, or relabelling AI inference as `collected`.
- Collector-count vanity (“50 new modules”) without a cited graph.
- Network calls from CI.

## See also

- [Connectors](connectors.md)
- [Playbooks](playbooks.md)
- [Contributing](../CONTRIBUTING.md)
- [Vision](../VISION.md)
