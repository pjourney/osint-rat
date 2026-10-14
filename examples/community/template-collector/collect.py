#!/usr/bin/env python3
"""Community collector template: print an os-int Evidence Batch on stdout.

Copy this directory when opening a community plugin pull request. os-int
spawns the program through exec/ with an explicit argv (no shell). Stdout
must be Ontology Exchange Evidence Batch JSON. Logs belong on stderr.

Stay passive: no port scans, no credential stuffing, no active probes.
Built-in collectors already cover DNS/RDAP/HTTP/TLS. A community sidecar
should add a source you already have (a local file, a documented public API
the analyst configured, or a deterministic fixture like this template).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys

BATCH_FORMAT = "os-int-evidence-batch"
BATCH_FORMAT_VERSION = 1


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Emit an os-int Evidence Batch JSON document on stdout."
    )
    parser.add_argument("--target", required=True, help="Normalized investigation target")
    parser.add_argument("--kind", default="domain", help="Target kind")
    parser.add_argument("--id", default="community-template", help="Connector id")
    return parser.parse_args(argv)


def sha256_hex(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def build_batch(kind: str, target: str, connector_id: str) -> dict:
    kind = (kind or "domain").strip().lower()
    target = (target or "").strip()
    if not target:
        raise SystemExit("community-template: --target is empty")
    if len(target) > 2048:
        raise SystemExit("community-template: --target is longer than 2048 characters")
    body = f"connector={connector_id}\nkind={kind}\ntarget={target}\n"
    source = {
        "kind": "document",
        "title": f"{connector_id} observation of {target}",
        "collector": f"connector:{connector_id}",
        "content": body,
        "content_hash": sha256_hex(body),
        "provenance": "external",
        "fields": [["connector", connector_id], ["origin", "community-template"]],
    }
    host = target.lower().rstrip(".")
    entities = [
        {"kind": "domain", "value": host, "sources": [0], "link_to_page": True},
        {"kind": "hostname", "value": f"www.{host}", "sources": [0], "link_to_page": True},
    ]
    relationships = [
        {
            "from": {"kind": "hostname", "value": f"www.{host}"},
            "kind": "subdomain_of",
            "to": {"kind": "domain", "value": host},
            "sources": [0],
            "confidence": "high",
        }
    ]
    return {
        "sources": [source],
        "entities": entities,
        "relationships": relationships,
        "summary": f"{connector_id} cited {len(entities)} object(s) for {target}",
    }


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    document = {
        "format": BATCH_FORMAT,
        "format_version": BATCH_FORMAT_VERSION,
        "batch": build_batch(args.kind, args.target, args.id),
    }
    sys.stdout.write(json.dumps(document, indent=2))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except BrokenPipeError:
        raise SystemExit(0)
