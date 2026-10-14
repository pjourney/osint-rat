#!/usr/bin/env python3
"""Hello-batch sidecar: print an os-int Evidence Batch on stdout.

os-int runs this as ``[[collectors.external]]``. The process is spawned with an
explicit argv (no shell). Stdout must be JSON. Logs belong on stderr.

This script does not touch the network. It turns the target into cited objects
and catalogued links so analysts can wire a connector without forking the
Rust crate.
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
    parser.add_argument(
        "--kind",
        default="domain",
        help="Target kind (domain, hostname, ip, url, email, username, …)",
    )
    parser.add_argument(
        "--id",
        default="hello-batch",
        help="Connector id, used as the source collector field",
    )
    return parser.parse_args(argv)


def sha256_hex(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def entity(kind: str, value: str, source: int = 0) -> dict:
    return {
        "kind": kind,
        "value": value,
        "sources": [source],
        "link_to_page": True,
    }


def relationship(kind: str, from_kind: str, from_value: str, to_kind: str, to_value: str) -> dict:
    return {
        "from": {"kind": from_kind, "value": from_value},
        "kind": kind,
        "to": {"kind": to_kind, "value": to_value},
        "sources": [0],
        "confidence": "high",
    }


def build_batch(kind: str, target: str, connector_id: str) -> dict:
    kind = (kind or "domain").strip().lower()
    target = (target or "").strip()
    if not target:
        raise SystemExit("hello-batch: --target is empty")
    if len(target) > 2048:
        raise SystemExit("hello-batch: --target is longer than 2048 characters")
    title = f"hello-batch observation of {target}"
    body = f"connector={connector_id}\nkind={kind}\ntarget={target}\n"
    source = {
        "kind": "document",
        "title": title,
        "collector": f"connector:{connector_id}",
        "content": body,
        "content_hash": sha256_hex(body),
        "provenance": "external",
        "fields": [["connector", connector_id], ["origin", "hello-batch"]],
    }
    entities = [entity(kind if kind != "hostname" else "hostname", target)]
    relationships = []
    if kind in {"domain", "hostname"} and "." in target:
        host = target.lower().rstrip(".")
        www = f"www.{host}"
        entities.append(entity("hostname", www))
        relationships.append(
            relationship("subdomain_of", "hostname", www, "domain", host)
        )
        relationships.append(
            relationship("resolves_to", "hostname", www, "ip", "192.0.2.80")
        )
        entities.append(entity("ip", "192.0.2.80"))
    elif kind == "email" and "@" in target:
        domain = target.rsplit("@", 1)[-1]
        entities.append(entity("domain", domain))
        relationships.append(
            relationship("hosted_on", "email", target, "domain", domain)
        )
    elif kind == "username":
        url = f"https://example.com/{target}"
        entities.append(entity("url", url))
        relationships.append(
            relationship("associated_with", "username", target, "url", url)
        )
    else:
        entities.append(entity("unknown", f"hello-{target}"))
        relationships.append(
            relationship(
                "associated_with",
                kind,
                target,
                "unknown",
                f"hello-{target}",
            )
        )
    return {
        "sources": [source],
        "entities": entities,
        "relationships": relationships,
        "summary": f"hello-batch cited {len(entities)} object(s) for {target}",
    }


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    batch = build_batch(args.kind, args.target, args.id)
    document = {
        "format": BATCH_FORMAT,
        "format_version": BATCH_FORMAT_VERSION,
        "batch": batch,
    }
    sys.stdout.write(json.dumps(document, indent=2))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except BrokenPipeError:
        raise SystemExit(0)
