# Ontology

os-int treats an investigation as a typed graph: **objects** (entities and pages), **links** (relationships), and **Actions** (proposed mutations). Collectors still write the existing rows — `entities.fields` is the property bag, `relationships.kind` stays a snake_case string — and this registry is how those values are interpreted.

The product thesis is in [VISION.md](../VISION.md). Phase gates are in [PHASED_ROADMAP.md](../PHASED_ROADMAP.md). The on-disk archive is described in [ontology-exchange.md](ontology-exchange.md).

Built-in collectors stay **passive-only**. Tools marked `active` are refused. AI only proposes Actions; nothing is written until you accept. Provenance is never relabelled. File ingest and sidecars land through `EvidenceBatch` → `storage::apply` ([connectors.md](connectors.md)); ingested sources use provenance `external`.

## Object types

Each [`EntityKind`](../src/model/entity.rs) and [`PageKind`](../src/model/page.rs) has a registered [`ObjectType`](../src/model/ontology.rs) with documented property groups. Required fields describe identity (`kind`, `value`, `provenance`). Optional fields are the keys collectors already write into the JSON property bag (`entities.fields`). There is no extra column per property.

| Key | Also a page | Property groups |
| --- | --- | --- |
| `domain` | yes | Identity, Registration, DNS and diagnostics |
| `hostname` | | Identity, DNS and diagnostics |
| `ip` | yes | Identity, Network |
| `asn` | yes | Identity, Network |
| `url` | yes | Identity, HTTP |
| `email` | yes | Identity |
| `username` | yes | Identity |
| `repository` | yes | Identity, Repository |
| `file` | yes | Identity, File |
| `certificate` | | Identity, Certificate |
| `person` | yes | Identity, Page |
| `organization` | yes | Identity, Page |
| `location` | | Identity |
| `phone` | | Identity |
| `unknown` | | Identity |
| `generic` | page only | Identity, Page |
| `event` | page only | Identity, Page |

Unknown kind strings still store (`Other`). Object View falls back to the `unknown` or `generic` schema. Uncatalogued field keys appear in an **Other** group.

To add or change a type: edit `OBJECT_TYPES` in `src/model/ontology.rs` and this table. Prefer a new optional field on the bag over a new SQLite column.

## Link-type catalog

Built-in verbs live in `src/model/relationship.rs` (`kinds`) and are registered in `LINK_TYPES` with a label, description, inverse (when one exists), and allowed endpoint kinds. Collectors emit the catalog verbs. Any other snake_case verb is still stored.

| Verb | Inverse | Typical from → to |
| --- | --- | --- |
| `resolves_to` | `reverse_dns` | domain/hostname → ip |
| `reverse_dns` | `resolves_to` | ip → hostname/domain |
| `alias_of` | `alias_of` | hostname ↔ hostname |
| `uses_nameserver` | | domain → hostname |
| `mail_handled_by` | | domain → hostname |
| `spf_includes` | | domain → domain |
| `announced_by` | | ip → asn |
| `operated_by` | | * → organization |
| `registered_with` | | domain → organization |
| `registered_to` | | domain → organization/person |
| `redirects_to` | | url → url |
| `serves` | | domain → url |
| `links_to` | | url → hostname |
| `mentions` | | * → * |
| `presents_certificate` | | domain → certificate |
| `issued_by` | | certificate → organization |
| `covers` | | certificate → domain |
| `subdomain_of` | `contains` | hostname → domain |
| `contains` | `subdomain_of` | domain → hostname |
| `references` | | * → * |
| `contributed_to` | | email → repository |
| `hosted_on` | | email → domain |
| `associated_with` | `associated_with` | * → * |

Object View groups outgoing and incoming edges by the catalog **label**. Unknown verbs appear under **Other**. The first time a verb is used in an investigation it is written to `link_types` (built-in description and inverse are copied; custom verbs get an empty description). That table is not exported: it is reconstructed on import from `relationships.kind`.

Empty `from_kinds` / `to_kinds` means any entity kind. The catalog does not reject writes that use a surprising pair; it documents the intended endpoints.

## Actions

Implemented agent Actions stay the six kinds the workbench already applies. Reserved kinds are catalogued so later phases do not invent a second enum. They may be stored if an agent emits them; accept is refused.

| Kind | Implemented | Notes |
| --- | --- | --- |
| `ADD_FINDING` | yes | Accepted AI findings keep `agent` provenance |
| `ADD_NOTE` | yes | |
| `CREATE_PAGE` | yes | |
| `LINK_ENTITY` | yes | |
| `RUN_COLLECTOR` | yes | Passive collectors only |
| `GENERATE_REPORT` | yes | |
| `PROPOSE_RESOLVE` | yes | Accept required; never auto-merge |
| `PROPOSE_UNRESOLVE` | yes | Accept required |
| `RUN_PLAYBOOK` | no | Playbooks (later phase) |
| `ASSERT_FINDING_TEMPLATE` | no | Playbooks (later phase) |

Accept and reject record `decided_at` and `decided_by` (the local user name from `$USER` / `$USERNAME`, or `local`). Pending rows leave both empty. `decided_by` is an optional archive field. Resolution bags required a format bump: `FORMAT_VERSION` is **2**.

## Entity resolution

A **resolution bag** is a canonical entity plus the constituents absorbed into it. Constituent rows stay in SQLite. The bag records property writes (`entities.fields`, plus label/notes) and source citations copied onto the canonical so unresolve can restore them. Nothing is deleted. `canonical_id` is a SQLite foreign key; `constituent_ids` is a JSON array, so `PRAGMA foreign_key_check` cannot validate those members.

- Manual merge: on **Entities**, `Space` marks rows, `m` proposes, Enter confirms. Esc cancels. Winner is the highlighted marked row (else the first marked).
- Unresolve: from Object View (or Entities) on the canonical, `u` then confirm.
- Suggestions (rules v1): same canonical email/domain/IP/repo URL; username↔profile URL; shared certificate SAN clusters. Palette **Resolution suggestions**. Scored only — never auto-merge.
- Agent Actions `PROPOSE_RESOLVE` / `PROPOSE_UNRESOLVE` apply only after accept. Timeline: `entities_resolved` / `entities_unresolved` with actor and constituents.
- Graph and Search Around collapse constituents onto the canonical (one node). Object View lists constituents and the union of sources, links, and findings.

Safe merge practices: [user guide](guide.md#entity-resolution).

## Graph and Search Around

The Graph screen walks the same objects and catalogued links. Search Around is a pure hop expansion over `entities` + `relationships` (in-memory snapshot or `Store::search_around`). Filters: link types, entity kinds, relationship provenance, and an optional time window on `relationships.created_at`. Unknown verbs still traverse; they group under Other in Object View and appear by their snake_case key in the facet strip. Caps (200 nodes / 400 edges) are a render guardrail, not a storage limit. The archive format does not change.

## Object View

From **Entities**, press `Enter` (or `v`, or the palette command **Object View**) to open the highlighted row. From **Pages** or Overview, press `v` (or the palette) to open the current page — its primary entity when it has one. A leftover entity is never reused. Esc returns.

The screen shows:

1. At least one **property group** (Identity is always present).
2. **Linked objects** grouped by catalog link type (unknown verbs under Other), including edges from absorbed constituents remapped to the canonical.
3. **Supporting sources** cited on the entity (union with constituents after a merge).
4. **Findings** on pages that include the entity, or that share a cited source.
5. **Constituents** when the object is the canonical of an active resolution bag. `u` unresolves.

## See also

- [Connectors](connectors.md)
- [Ontology Exchange Format](ontology-exchange.md)
- [Database tables](schema.md)
- [Architecture — where to make a change](ARCHITECTURE.md#where-to-make-a-change)
- [User guide — Object View](guide.md#object-view)
- [User guide — Graph and Search Around](guide.md#graph-and-search-around)
- [User guide — Entity resolution](guide.md#entity-resolution)
