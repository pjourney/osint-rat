# Community plugins

os-int is an open platform for **passive** ontology plugins: external collectors (Evidence Batch sidecars) and YAML playbooks. There is no separate storefront. You publish by opening a pull request that adds a connector under `examples/` or a playbook under `playbooks/`, following [docs/plugins.md](../../docs/plugins.md).

## Start here

| You want to publish | Copy |
| --- | --- |
| An external collector | [`template-collector/`](template-collector/) |
| A playbook | [`template-playbook.yaml`](template-playbook.yaml) |

The in-tree sidecar that already ships is [`examples/connectors/hello-batch/`](../connectors/hello-batch/). Use that when you need a working reference; use the templates in this folder when you are opening a new plugin pull request.

## Pull request title

`community-plugin: <id> — <one-line description>`

The reviewer checks the [plugin bar](../../docs/plugins.md#review-bar): passive-only, provenance preserved, tests, no silent AI writeback.
