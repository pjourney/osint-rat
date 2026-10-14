# Ontology HTTP API

`os-int serve` exposes a **localhost** ontology bus: read Object View / Search Around / suggestions, and **propose** Actions. Proposals stay pending until an analyst accepts them in the workbench. The API never silently writes findings, merges identities, or runs collectors.

Bind defaults to **`127.0.0.1:8743`**. Unauthorized requests to data routes return **401**.

## Start

```bash
os-int serve
os-int serve --bind 127.0.0.1:8743 --token "$OS_INT_API_TOKEN"
```

Configuration (`config.toml`):

```toml
[api]
bind = "127.0.0.1:8743"
# Empty token: serve mints an ephemeral token and prints it once.
token = ""
```

`--token` and `OS_INT_API_TOKEN` override `[api].token`. Send `Authorization: Bearer <token>` or `X-Osint-Token: <token>`.

`GET /health` and `GET /openapi.json` do not require a token. Every `/v1/…` route does.

## Routes

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/health` | Liveness |
| `GET` | `/openapi.json` | OpenAPI 3.0.3 snippet |
| `GET` | `/v1/investigations` | List cases |
| `GET` | `/v1/investigations/{id}` | One case |
| `GET` | `/v1/investigations/{id}/objects?kind=domain` | Visible entities |
| `GET` | `/v1/investigations/{id}/objects/{ref}` | **Object View** (`E-1` or `P-1`) |
| `GET` | `/v1/investigations/{id}/search-around?seed=E-1&hops=2` | Search Around |
| `GET` | `/v1/investigations/{id}/resolutions/suggest` | Scored merge suggestions (never applied) |
| `POST` | `/v1/investigations/{id}/actions` | Propose Actions; **202** with `status: pending` |
| `GET` | `/v1/investigations/{id}/proposals` | List proposals |

`{id}` is a name, UUID, or unique UUID prefix (same as the CLI).

POST body:

```json
{
  "actions": [
    { "action": "ADD_FINDING", "title": "Mail policy gap", "details": "…" }
  ]
}
```

Accepted Action names match the catalog (`ADD_FINDING`, `ADD_NOTE`, `CREATE_PAGE`, `LINK_ENTITY`, `RUN_COLLECTOR`, `GENERATE_REPORT`, `PROPOSE_RESOLVE`, `PROPOSE_UNRESOLVE`, `RUN_PLAYBOOK`, `ASSERT_FINDING_TEMPLATE`). Nothing in that list is applied by the API.

A `reader` on a shared-data-dir case receives **403** on POST.

## Scriptable CLI (same helpers)

```bash
os-int objects CASE
os-int get CASE E-1
os-int search-around CASE E-1 --hops 2
os-int resolve suggest CASE
os-int resolve propose CASE E-1 E-4 --reason "same host"
os-int resolve merge CASE E-1 E-4     # analyst action; not the API
os-int export CASE --format stix -o bundle.json
```

`resolve propose` stores a pending `PROPOSE_RESOLVE`. `resolve merge` is the same confirmation the workbench uses.

OpenAPI lives at [`src/api/openapi.json`](../src/api/openapi.json) and is served at `/openapi.json`.
