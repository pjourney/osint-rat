# Vision

os-int is a **local-first investigation ontology** for OSINT and cyber cases. The product is the graph and the resolution engine: fuse sources into objects and links, resolve identities, Search Around the graph, run playbooks, and keep humans in control of every Action—including those proposed by AI.

The bar is an enterprise-class investigative loop, not recon CLI parity and not collector-count vanity.

## The loop

| Pillar | What it means |
| --- | --- |
| **Objects and links** | First-class types, properties, and verbs—not only freeform entities and ad hoc relationship strings. |
| **Entity resolution** | Merge records that are the same real-world thing without destroying history; unresolve cleanly. |
| **Search Around** | Expand from a selection by hops, link types, and kinds; explore the ontology, not only tables. |
| **Playbooks** | Capture analyst logic as reusable, reviewable workflows. Automations propose; they do not silently write. |
| **Governed Actions** | Accept, reject, and audit every mutation. Provenance stays attached to every observation. |
| **AI proposes only** | Local agent CLIs may suggest Actions. Nothing is written until an analyst accepts. Accepted AI findings keep `agent` provenance. |

Collectors are **plugins**. Built-in collectors stay **passive-only**: public DNS, RDAP, a single HTTP(S) fetch of a URL you specify, TLS, certificate transparency, public git metadata, and local file inspection. Tools marked `active` are refused. Everything else plugs in as an external program or an exchange-format sidecar.

Data lives on the machine that runs os-int. There is no cloud requirement.

## Non-goals

- Active probing, exploitation, credential attacks, or remote access.
- Silent AI writeback or promoting inference to collected fact.
- Chasing raw module or collector count as a success metric.
- Replacing analyst judgment with automatic merges or unmarked findings.

These boundaries match [SECURITY.md](SECURITY.md). This document does not relax them.

## Success metrics (from Phase 1 onward)

- Time from a seed domain to a useful graph (≥20 linked entities) on a standard fixture.
- Percentage of findings with at least one cited source.
- Search Around hops explored per session.
- Playbooks run successfully (local counter is enough).

## How we get there

Delivery is gated by phase. Do not start phase *N+1* on `main` until phase *N* merge criteria are met.

The plan, status table, and per-phase checklists live in **[PHASED_ROADMAP.md](PHASED_ROADMAP.md)**.
