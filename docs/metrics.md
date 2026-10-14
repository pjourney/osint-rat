# Success metrics

os-int does not treat GitHub stars, download counts, or collector module counts as success. The numbers below are **methodology plus a fixture measurement**. They are not telemetry uploaded anywhere. Search Around hops per session and playbooks-per-week stay on the machine that ran them (optional local counting); this document does not enable phoning home.

## What we measure

| Metric | Method | Demo-case result |
| --- | --- | --- |
| Time from seed to a useful graph (≥20 linked entities) | Import `examples/demo-case/` (no live collection). `os-int search-around "Example Corp" E-1 --hops 2`. | Import is local SQLite + JSON; on the Phase 9 development VM this is well under a second. The snapshot has **21 entities** (20 visible after resolving the hostname into the domain) and **30 relationships**. Search Around 2 hops from `E-1` reaches the ASN. |
| % of findings with ≥1 cited source | Count `findings` whose `source_ids` is non-empty after import. | **100%** (3 / 3). Provenance on those rows is `collected`. |
| Search Around hops used | Default in the workbench is 2 (`s`). Caps are documented in [guide.md](guide.md#graph-and-search-around). | Demo walkthrough uses 2 hops. No session telemetry is recorded. |
| Playbooks run | Timeline kind `playbook_run` and local `playbook_runs` rows. | Demo snapshot includes one `domain-infra` run on the timeline. Weekly counts are whatever you run locally. |
| External connectors in-tree vs out-of-tree | In-tree: `examples/connectors/` plus `examples/community/`. Out-of-tree: your `[[collectors.external]]` entries. | In-tree: `hello-batch` plus the community template. Out-of-tree is unbounded; CI does not scrape the internet for them. |

Reproduce the demo-case column:

```bash
cargo test --test demo_case
os-int import examples/demo-case --data-dir /tmp/os-int-metrics --config /tmp/os-int-metrics/config.toml
os-int search-around "Example Corp" E-1 --hops 2 --data-dir /tmp/os-int-metrics --config /tmp/os-int-metrics/config.toml
```

A live `os-int collect domain example.com` is a different measurement (network, CT, RDAP). It is valid for your environment; it is not the number in the table.

## What we refuse to count

- Number of built-in collectors.
- Number of GitHub stars or forks.
- “Modules” or vendor feature-matrix rows.

The [vision](../VISION.md) and [roadmap](../PHASED_ROADMAP.md) use the table above. Phase 9 ships the methodology in the README; it does not add a public analytics backend.
