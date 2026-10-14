---
name: Community plugin
about: Propose an external collector or playbook (passive-only)
title: "community-plugin: "
labels: ["enhancement"]
---

## Plugin type

- [ ] External collector (Evidence Batch sidecar)
- [ ] YAML playbook

## Id

Unique snake-case id.

## What it adds to the ontology

Which objects, links, or playbook steps, and which sources they cite.

## Passive-only

Confirm the plugin does not scan, exploit, or otherwise probe beyond a single user-specified public fetch the analyst already asked for.

- [ ] `activity = "passive"`
- [ ] No silent finding writes
- [ ] Provenance is `external` (collector) or pending proposals (playbook templates)

## Tests

How CI will run without the public internet.

## See also

[docs/plugins.md](https://github.com/pjourney/osint-rat/blob/main/docs/plugins.md)
