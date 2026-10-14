# Example Corp demo case

A portable investigation archive for the first ten minutes with os-int. It is a **documentation-range fixture** (`example.com`, `192.0.2.0/24`, `2001:db8::/32`). Nothing in this folder was collected from a live network for this snapshot.

The case already contains:

- A cited graph (domain, addresses, nameservers, ASN, certificate, organization)
- A resolved identity (hostname `example.com` merged into the domain)
- Playbook output (`domain-infra` on the timeline as a `playbook_run` event)
- Three findings, each citing at least one source (`COL`)

## Import this first

From the repository root, after `os-int` is on your `PATH`:

```bash
mkdir -p /tmp/os-int-demo
alias os-int-demo='os-int --data-dir /tmp/os-int-demo --config /tmp/os-int-demo/config.toml'

os-int-demo import examples/demo-case
os-int-demo open "Example Corp"
```

Inside the workbench (no need to read `docs/ARCHITECTURE.md`):

1. Press `g` — Graph of `example.com` and its infra.
2. Press `s` — Search Around two hops (addresses, then ASN / organization).
3. Press `Enter` on the canonical domain — Object View with properties, catalogued links, constituents, and cited findings.
4. Press `6` — Timeline, including **Playbook domain-infra**.
5. Press `4` — Findings. Every row has a source citation.

The same walkthrough from the shell:

```bash
os-int-demo objects "Example Corp"
os-int-demo search-around "Example Corp" E-1 --hops 2
os-int-demo get "Example Corp" E-1
```

Rebuild the checked-in JSON from the typed snapshot (maintainers):

```bash
cargo test --lib demo::tests::write_checked_in_demo_archive -- --ignored --nocapture
```
