#!/usr/bin/env python3
"""Assert the rendered chart's Ingress routes exactly the three seams.

Split out of check-edge.sh so the assertion parses the Ingress document
instead of grepping the whole render: the worker carries its own
`path: /healthz` readinessProbe, and a grep over the render cannot tell
that apart from an ingress route (it once reported a pass for a healthz
route that had already been removed).

Usage: check-ingress.py <rendered-manifests.yaml>
"""
import sys

import yaml


def main(path: str) -> int:
    docs = [d for d in yaml.safe_load_all(open(path)) if d]
    ingresses = [d for d in docs if d.get("kind") == "Ingress"]
    if not ingresses:
        print("  FAIL no Ingress rendered")
        return 1

    ing = ingresses[0]
    rules = ing["spec"]["rules"]
    paths = [
        (p["path"], p["backend"]["service"]["name"])
        for p in rules[0]["http"]["paths"]
    ]
    got = {path for path, _ in paths}
    want = {"/api", "/openapi.json", "/"}
    services = {name for _, name in paths}

    problems = []
    if got != want:
        problems.append(f"ingress paths are {sorted(got)}, expected {sorted(want)}")
    if "/healthz" in got:
        problems.append("ingress routes /healthz, which core-api answers 404")
    if not any(name.endswith("-core-api") for name in services):
        problems.append("no ingress path reaches core-api")
    if not any(name.endswith("-web") for name in services):
        problems.append("no ingress path reaches the web console")
    if not ing["spec"].get("tls"):
        problems.append("no tls block: the public origin would speak plaintext")

    for problem in problems:
        print(f"  FAIL {problem}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
