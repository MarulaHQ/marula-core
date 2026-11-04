#!/usr/bin/env python3
"""Assert the rendered chart's Ingress routes exactly the three seams.

Split out of check-edge.sh so the assertion parses the Ingress document
instead of grepping the whole render: the worker carries its own
`path: /healthz` readinessProbe, and a grep over the render cannot tell
that apart from an ingress route (it once reported a pass for a healthz
route that had already been removed).

Why the parser is here and not PyYAML (F71): `import yaml` made this check
fail for an environment reason on any host without the package — 33/33 with
it, 32/1 without, on an unchanged tree. A gate that is red because of what is
installed beside the checkout is a gate nobody reads. So this file reads the
block-YAML subset `helm template` emits for this one document, and REFUSES
anything outside that subset (flow collections, anchors, tags, block scalars,
tabs) instead of guessing: a parser that quietly skipped what it understood
nothing about would report a pass for a document it never read.

Only documents carrying a `kind: Ingress` line are parsed at all. The rest of
the render is not this check's business, and the other objects legitimately
use the constructs above (`selector: {}`), so refusing on them would fail a
render that is fine.

Usage:  check-ingress.py <rendered-manifests.yaml>
        check-ingress.py --self-test
"""
import sys

WANT_PATHS = {"/api", "/openapi.json", "/"}


class Refused(Exception):
    """The document is outside the subset this parser reads."""


def _scalar(text: str) -> str:
    text = text.strip()
    if len(text) >= 2 and text[0] == text[-1] and text[0] in "\"'":
        return text[1:-1]
    if text[:1] in "{[":
        raise Refused(f"flow collection, not read: {text!r}")
    if text[:1] in "|>":
        raise Refused(f"block scalar, not read: {text!r}")
    if text[:1] in "&*!":
        raise Refused(f"anchor, alias or tag, not read: {text!r}")
    return text


def _split_key(line: str):
    """A `key: value` / `key:` mapping entry, quotes respected."""
    quote = None
    for index, char in enumerate(line):
        if quote is not None:
            if char == quote:
                quote = None
            continue
        if char in "\"'":
            quote = char
            continue
        if char == ":" and (index + 1 == len(line) or line[index + 1] == " "):
            return _scalar(line[:index]), line[index + 1 :].strip()
    return None


def _tokens(lines):
    tokens = []
    for raw in lines:
        if "\t" in raw:
            raise Refused("tab indentation, not read")
        stripped = raw.rstrip()
        body = stripped.lstrip(" ")
        if not body or body.startswith("#"):
            continue
        tokens.append((len(stripped) - len(body), body))
    return tokens


def _parse_block(tokens, index, indent):
    if index >= len(tokens):
        return None, index
    line_indent, body = tokens[index]
    if line_indent != indent:
        raise Refused(f"unexpected indentation at {body!r}")
    if body == "-" or body.startswith("- "):
        return _parse_sequence(tokens, index, indent)
    return _parse_mapping(tokens, index, indent)


def _parse_sequence(tokens, index, indent):
    items = []
    while index < len(tokens):
        line_indent, body = tokens[index]
        if line_indent < indent:
            break
        if line_indent > indent:
            raise Refused(f"over-indented sequence entry {body!r}")
        if not (body == "-" or body.startswith("- ")):
            break
        rest = body[1:].strip()
        index += 1
        if rest == "":
            value, index = _parse_block(tokens, index, indent + 2)
            items.append(value)
            continue
        entry = _split_key(rest)
        if entry is None:
            items.append(_scalar(rest))
            continue
        # `- key: value` opens a mapping whose remaining keys sit at indent + 2.
        item = {}
        key, tail = entry
        if tail:
            item[key] = _scalar(tail)
        elif index < len(tokens) and tokens[index][0] > indent:
            item[key], index = _parse_block(tokens, index, tokens[index][0])
        else:
            item[key] = None
        while index < len(tokens) and tokens[index][0] == indent + 2:
            entry = _split_key(tokens[index][1])
            if entry is None:
                raise Refused(f"expected a mapping entry at {tokens[index][1]!r}")
            key, tail = entry
            index += 1
            if tail:
                item[key] = _scalar(tail)
            elif index < len(tokens) and tokens[index][0] > indent + 2:
                item[key], index = _parse_block(tokens, index, tokens[index][0])
            else:
                item[key] = None
        items.append(item)
    return items, index


def _parse_mapping(tokens, index, indent):
    mapping = {}
    while index < len(tokens):
        line_indent, body = tokens[index]
        if line_indent < indent:
            break
        if line_indent > indent:
            raise Refused(f"over-indented mapping entry {body!r}")
        entry = _split_key(body)
        if entry is None:
            raise Refused(f"expected a mapping entry, found {body!r}")
        key, tail = entry
        index += 1
        if tail:
            mapping[key] = _scalar(tail)
        elif index < len(tokens) and tokens[index][0] > indent:
            mapping[key], index = _parse_block(tokens, index, tokens[index][0])
        else:
            mapping[key] = None
    return mapping, index


def documents(text):
    """The render's documents, as line lists. Helm separates them with `---`."""
    chunks, current = [], []
    for line in text.splitlines():
        if line.strip() == "---":
            chunks.append(current)
            current = []
        else:
            current.append(line)
    chunks.append(current)
    return [
        chunk
        for chunk in chunks
        if any(line.strip() and not line.strip().startswith("#") for line in chunk)
    ]


def is_ingress(lines) -> bool:
    return any(line.strip() == "kind: Ingress" for line in lines)


def parse_document(lines):
    tokens = _tokens(lines)
    if not tokens:
        return {}
    if tokens[0][1].startswith("%"):
        raise Refused("YAML directive, not read")
    value, index = _parse_block(tokens, 0, tokens[0][0])
    if index != len(tokens):
        raise Refused(f"unparsed remainder starting at {tokens[index][1]!r}")
    return value


def ingress_problems(document):
    """The three seams, checked on the parsed document. Empty = pass."""
    spec = document.get("spec")
    if not isinstance(spec, dict):
        return ["the Ingress has no spec map"]
    rules = spec.get("rules")
    if not isinstance(rules, list) or not rules:
        return ["the Ingress declares no rules"]
    first = rules[0]
    http = first.get("http") if isinstance(first, dict) else None
    entries = http.get("paths") if isinstance(http, dict) else None
    if not isinstance(entries, list) or not entries:
        return ["the first rule declares no http paths"]

    paths = []
    for entry in entries:
        if not isinstance(entry, dict):
            return [f"a path entry is not a mapping: {entry!r}"]
        path = entry.get("path")
        backend = entry.get("backend")
        service = backend.get("service") if isinstance(backend, dict) else None
        name = service.get("name") if isinstance(service, dict) else None
        if not isinstance(path, str) or not isinstance(name, str):
            return [f"a path entry has no path/service name: {entry!r}"]
        paths.append((path, name))

    got = {path for path, _ in paths}
    services = {name for _, name in paths}
    problems = []
    if got != WANT_PATHS:
        problems.append(f"ingress paths are {sorted(got)}, expected {sorted(WANT_PATHS)}")
    if "/healthz" in got:
        problems.append("ingress routes /healthz, which core-api answers 404")
    if not any(name.endswith("-core-api") for name in services):
        problems.append("no ingress path reaches core-api")
    if not any(name.endswith("-web") for name in services):
        problems.append("no ingress path reaches the web console")
    if not spec.get("tls"):
        problems.append("no tls block: the public origin would speak plaintext")
    return problems


def check(text: str) -> list:
    """Problems for one render. A refusal is a problem, never a pass."""
    ingresses, chunks = 0, documents(text)
    for lines in chunks:
        if not is_ingress(lines):
            continue
        document = parse_document(lines)
        if not isinstance(document, dict):
            return ["the Ingress document does not parse as a mapping"]
        ingresses += 1
        problems = ingress_problems(document)
        if problems:
            return problems
    if ingresses == 0:
        return ["no Ingress rendered"]
    return []


GOOD = """---
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: marula
  annotations:
    "nginx.ingress.kubernetes.io/proxy-body-size": 25Mi
spec:
  ingressClassName: nginx
  tls:
    - hosts:
        - marula.example.com
      secretName: marula-tls
  rules:
    - host: "marula.example.com"
      http:
        paths:
          - path: /api
            pathType: Prefix
            backend:
              service:
                name: marula-core-api
                port:
                  number: 8080
          - path: /openapi.json
            pathType: Prefix
            backend:
              service:
                name: marula-core-api
                port:
                  number: 8080
          - path: /
            pathType: Prefix
            backend:
              service:
                name: marula-web
                port:
                  number: 3000
"""

OTHER = """apiVersion: v1
kind: Service
metadata:
  name: other
spec:
  selector: {}
  ports:
    - port: 80
"""


def _self_test() -> int:
    """Prove the assertion still bites: a good render passes, a removed route
    fails, and a document outside the subset is refused rather than passed."""
    api_route = """          - path: /api
            pathType: Prefix
            backend:
              service:
                name: marula-core-api
                port:
                  number: 8080
"""
    cases = [
        ("the chart's own shape passes", GOOD, 0),
        ("a removed route fails", GOOD.replace(api_route, ""), 1),
        (
            "a /healthz route fails",
            GOOD.replace("          - path: /\n", "          - path: /healthz\n"),
            1,
        ),
        (
            "a missing tls block fails",
            GOOD.replace("  tls:\n    - hosts:", "  hosts:\n    - hosts:"),
            1,
        ),
        (
            "a path that reaches neither service fails",
            GOOD.replace("marula-web", "something-else"),
            1,
        ),
        (
            "another object using a flow mapping does not fail the check",
            OTHER + "---\n" + GOOD.split("---\n", 1)[1],
            0,
        ),
        (
            "a flow mapping inside the Ingress is refused",
            GOOD.replace("  ingressClassName: nginx\n", "  ingressClassName: {a: b}\n"),
            1,
        ),
        (
            "a tab is refused",
            GOOD.replace("  ingressClassName: nginx\n", "\tingressClassName: nginx\n"),
            1,
        ),
        ("a render with no Ingress fails", OTHER, 1),
    ]
    failures = 0
    for label, text, want in cases:
        try:
            problems = check(text)
        except Refused as refusal:
            problems = [f"refused: {refusal}"]
        got = 1 if problems else 0
        if got != want:
            print(f"  FAIL self-test {label!r}: exit {got}, wanted {want}")
            for problem in problems:
                print(f"        {problem}")
            failures += 1
        else:
            print(f"  ok   self-test {label}")
    return 1 if failures else 0


def main(argv) -> int:
    if len(argv) == 2 and argv[1] == "--self-test":
        return _self_test()
    if len(argv) != 2:
        print("usage: check-ingress.py <rendered-manifests.yaml> | --self-test", file=sys.stderr)
        return 2
    try:
        with open(argv[1], encoding="utf-8") as handle:
            text = handle.read()
        problems = check(text)
    except Refused as refusal:
        print(f"  FAIL cannot read the rendered Ingress: {refusal}")
        return 1
    except OSError as error:
        print(f"  FAIL cannot read {argv[1]}: {error}")
        return 1
    for problem in problems:
        print(f"  FAIL {problem}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
