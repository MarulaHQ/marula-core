#!/usr/bin/env python3
"""Measure EmbeddingGemma 2 on the reference deployment profiles.

Why this exists: the product runs embeddings locally by default, and "efficient
enough" has to be a number. This harness answers four questions at once, on the
corpus the vector benchmark already built:

  throughput   how many chunks per second a backfill batch gets, and whether one
               batch of EMBEDDING_BATCH inputs fits the client timeout
  memory       how much resident memory the loaded model costs, peak and steady
  cold start   what the first request after an idle unload costs
  quality      which variant ranks a fixed query set better, and how far apart
               the two variants' answers are

Reuse, don't rebuild: the corpus comes from `00-build-real-corpus.py` — the same
ticket archetypes, the same repository prose, the same seed — so this lane and
the vector-ANN lane measure the same text rather than two invented corpora.

Profiles are expressed as a core count, because the reference boxes differ in
vCPU and nothing else this harness can control. CPU-only execution is
`num_gpu: 0`; the Metal path is recorded separately as this machine's
accelerator, never as a profile.

Examples:
  ./30-embedding-profiles.py corpus --out /tmp/r13-corpus
  ./30-embedding-profiles.py throughput --model embeddinggemma-2:270m-q8_0-text --threads 4
  ./30-embedding-profiles.py coldstart --model embeddinggemma-2:270m-q8_0-text --threads 4
  ./30-embedding-profiles.py quality --model embeddinggemma-2:270m-q8_0-text --threads 8
  ./30-embedding-profiles.py compare --a a.json --b b.json

Requires a running Ollama (`ollama serve`) holding the models. No third-party
packages: the client is urllib, the maths is a dot product.
"""

import argparse
import importlib.util
import json
import os
import random
import statistics
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

OLLAMA = os.environ.get("OLLAMA_URL", "http://localhost:11434")
HERE = os.path.dirname(os.path.abspath(__file__))
CORPUS_BUILDER = os.path.join(HERE, "00-build-real-corpus.py")

DOC_PREFIX = "title: {title} | text: {content}"
QUERY_PREFIX = "task: search result | query: {query}"


def load_corpus_builder():
    """Import the P10 corpus builder so both lanes share one vocabulary."""
    spec = importlib.util.spec_from_file_location("p52_corpus", CORPUS_BUILDER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# --- ollama client -----------------------------------------------------------


def embed(model, texts, threads, gpu, keep_alive=None, timeout=1800):
    """One /api/embed call. Returns (payload, wall seconds)."""
    options = {"num_thread": threads}
    if gpu is not None:
        options["num_gpu"] = gpu
    body = {"model": model, "input": texts, "options": options}
    if keep_alive is not None:
        body["keep_alive"] = str(keep_alive)
    payload = json.dumps(body).encode()
    request = urllib.request.Request(
        OLLAMA + "/api/embed", data=payload, headers={"Content-Type": "application/json"}
    )
    started = time.time()
    with urllib.request.urlopen(request, timeout=timeout) as response:
        data = json.load(response)
    return data, time.time() - started


def loaded_models():
    with urllib.request.urlopen(OLLAMA + "/api/ps", timeout=30) as response:
        return json.load(response).get("models", [])


def load_average():
    """1-minute load average, recorded with every result.

    The measurement box is a workstation that also builds this repository, so a
    number without its load context cannot be compared to the next one.
    """
    try:
        return round(os.getloadavg()[0], 2)
    except (AttributeError, OSError):
        return None


def resident_now():
    """RSS of the inference process right now, in MB."""
    return round(sum(rss_kb(pid) for pid in runner_pids()) / 1024, 1)


def same_model(a, b):
    """Compare model names across the two shapes Ollama reports.

    A request names `nomic-embed-text`; `/api/ps` answers with
    `nomic-embed-text:latest`. Treating those as different models is how a
    memory reading silently becomes null.
    """
    def norm(value):
        value = (value or "").strip()
        return value[:-len(":latest")] if value.endswith(":latest") else value
    return norm(a) == norm(b)


def unload(model):
    """Drop every loaded model so a cold start is actually cold."""
    if not any(same_model(m.get("model"), model) for m in loaded_models()):
        # Load it explicitly with keep_alive 0: the request both proves the
        # model is servable and schedules its own eviction.
        embed(model, ["unload"], threads=1, gpu=None, keep_alive=0, timeout=600)
    for entry in loaded_models():
        for name in (entry.get("model"), entry.get("name")):
            if name:
                embed(name, ["unload"], threads=1, gpu=None, keep_alive=0, timeout=600)
                break
    for _ in range(240):
        if not loaded_models():
            break
        # keep asking: a model with a long keep_alive needs the second nudge
        for entry in loaded_models():
            name = entry.get("model") or entry.get("name")
            if name:
                embed(name, ["unload"], threads=1, gpu=None, keep_alive=0, timeout=600)
        time.sleep(0.5)
    for _ in range(40):
        if not runner_pids():
            break
        time.sleep(0.25)


# Ollama 0.40 runs a model in one of two processes: `llama-server` from its
# libexec directory, or `ollama runner --model …`. Which one depends on the
# model's quantization, so a pattern that matches only the first silently
# reports zero resident memory for a bf16 model.
RUNNER_PATTERNS = os.environ.get(
    "OLLAMA_RUNNER_PATTERNS", "lib/ollama/llama-server,ollama runner --model"
).split(",")


def runner_pids():
    """PIDs of the inference processes currently holding a model."""
    try:
        out = ""
        for pattern in RUNNER_PATTERNS:
            out += subprocess.run(
                ["pgrep", "-f", pattern.strip()], capture_output=True, text=True, check=False
            ).stdout
    except FileNotFoundError:
        return []
    return [int(p) for p in out.split() if p.strip().isdigit()]


def rss_kb(pid):
    try:
        out = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True, check=False
        ).stdout.strip()
        return int(out) if out else 0
    except (ValueError, FileNotFoundError):
        return 0


class RssSampler(threading.Thread):
    """Samples the runner's resident set while a measurement runs."""

    def __init__(self, interval=0.2):
        super().__init__(daemon=True)
        self.interval = interval
        self.samples = []
        self._stop = threading.Event()

    def run(self):
        while not self._stop.is_set():
            pids = runner_pids()
            total = sum(rss_kb(pid) for pid in pids)
            if total:
                self.samples.append(total)
            self._stop.wait(self.interval)

    def stop(self):
        self._stop.set()
        self.join(timeout=5)

    def summary(self):
        if not self.samples:
            return {"peak_mb": None, "median_mb": None, "samples": 0}
        return {
            "peak_mb": round(max(self.samples) / 1024, 1),
            "median_mb": round(statistics.median(self.samples) / 1024, 1),
            "samples": len(self.samples),
        }


# --- corpus ------------------------------------------------------------------


def build_docs(tickets, prose, seed=42):
    """Ticket-shaped chunks plus repository prose, each labelled with its archetype.

    The archetype label is what makes a quality judgment possible without a human
    in the loop: a query taken from a backup-failure ticket should retrieve the
    other backup-failure tickets, and nothing about the two embedding variants
    changes that judgment.
    """
    builder = load_corpus_builder()
    rng = random.Random(seed)
    docs = []
    for i in range(tickets):
        arch = rng.randrange(len(builder.ARCHETYPES))
        title_t, body_t = builder.ARCHETYPES[arch]
        subs = {
            "client": rng.choice(builder.CLIENTS),
            "site": rng.choice(builder.SITES),
            "device": rng.choice(builder.DEVICES),
            "error": rng.choice(builder.ERRORS),
            "time": rng.choice(builder.TIMES),
        }
        title = title_t.format(**subs)
        body = body_t.format(**subs) + " " + rng.choice(builder.DETAILS)
        docs.append({"id": f"t{i}", "arch": arch, "title": title, "text": DOC_PREFIX.format(title=title, content=body)})
    repo = os.path.dirname(os.path.dirname(HERE))
    for j, (heading, para) in enumerate(builder.doc_chunks(repo, prose)):
        docs.append({"id": f"k{j}", "arch": None, "title": heading, "text": DOC_PREFIX.format(title=heading, content=para)})
    return docs


def write_corpus(path, tickets, prose, seed):
    docs = build_docs(tickets, prose, seed)
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        for d in docs:
            fh.write(json.dumps(d, ensure_ascii=False) + "\n")
    words = [len(d["text"].split()) for d in docs]
    print(f"corpus: {len(docs)} documents, median {statistics.median(words):.0f} words, max {max(words)}")
    return docs


def read_corpus(path):
    with open(path, encoding="utf-8") as fh:
        return [json.loads(line) for line in fh if line.strip()]


# --- subcommands -------------------------------------------------------------


def cmd_corpus(args):
    write_corpus(args.out, args.tickets, args.prose, args.seed)
    return 0


def cmd_throughput(args):
    docs = read_corpus(args.corpus)[: args.chunks]
    texts = [d["text"] for d in docs]
    batch = args.batch
    batches = [texts[i:i + batch] for i in range(0, len(texts), batch)]
    if len(batches) < 2:
        print("corpus too small for a warmup batch plus a measured one", file=sys.stderr)
        return 2

    unload(args.model)
    sampler = RssSampler()
    sampler.start()

    warm, warm_wall = embed(args.model, batches[0], args.threads, args.gpu)
    load_ns = warm.get("load_duration", 0)
    print(f"warmup batch of {len(batches[0])}: {warm_wall:.2f}s (load {load_ns / 1e9:.2f}s)")

    # What the host holds while the model is loaded and idle: the number an
    # operator sizes a box against, taken before any inference is in flight.
    time.sleep(2.0)
    idle_loaded_mb = resident_now()
    loaded = next((m for m in loaded_models() if same_model(m.get("model"), args.model)), None)
    ollama_size = loaded.get("size") if loaded else None
    ollama_size_vram = loaded.get("size_vram") if loaded else None
    sampler.samples.clear()

    measured = []
    for index, chunk in enumerate(batches[1:]):
        payload, wall = embed(args.model, chunk, args.threads, args.gpu)
        check = len(payload["embeddings"])
        if check != len(chunk):
            print(f"arity mismatch: asked {len(chunk)}, got {check}", file=sys.stderr)
            return 3
        rate = len(chunk) / wall
        measured.append({"batch": index + 1, "inputs": len(chunk), "seconds": round(wall, 3), "chunks_per_s": round(rate, 2)})
        print(f"  batch {index + 1}: {len(chunk)} in {wall:.2f}s = {rate:.2f} chunks/s")

    sampler.stop()
    rates = [m["chunks_per_s"] for m in measured]
    slowest = max(m["seconds"] for m in measured)
    result = {
        "model": args.model,
        "profile": args.profile,
        "threads": args.threads,
        "gpu": args.gpu,
        "batch": batch,
        "median_chunks_per_s": round(statistics.median(rates), 2),
        "slowest_batch_s": round(slowest, 3),
        "fits_client_timeout": slowest <= args.client_timeout,
        "idle_loaded_rss_mb": idle_loaded_mb,
        "load_1m": load_average(),
        "ollama_reported_bytes": ollama_size,
        "ollama_reported_vram_bytes": ollama_size_vram,
        "rss": sampler.summary(),
        "batches": measured,
    }
    print(json.dumps(result, indent=2))
    if args.out:
        with open(args.out, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(result) + "\n")
    return 0


def cmd_latency(args):
    docs = read_corpus(args.corpus)
    text = docs[0]["text"]
    embed(args.model, [text], args.threads, args.gpu)
    walls = []
    for _ in range(args.requests):
        _payload, wall = embed(args.model, [text], args.threads, args.gpu)
        walls.append(wall)
    walls.sort()
    result = {
        "model": args.model,
        "profile": args.profile,
        "threads": args.threads,
        "gpu": args.gpu,
        "requests": len(walls),
        "p50_ms": round(statistics.median(walls) * 1000, 1),
        "load_1m": load_average(),
        "p95_ms": round(walls[min(len(walls) - 1, int(len(walls) * 0.95))] * 1000, 1),
    }
    print(json.dumps(result, indent=2))
    if args.out:
        with open(args.out, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(result) + "\n")
    return 0


def cmd_coldstart(args):
    docs = read_corpus(args.corpus)
    text = docs[0]["text"]
    unload(args.model)
    baseline = sum(rss_kb(pid) for pid in runner_pids()) or 0
    idle = loaded_models()
    if idle:
        print(f"expected an unloaded server, /api/ps reports {len(idle)} model(s)", file=sys.stderr)
        return 4

    payload, wall = embed(args.model, [text], args.threads, args.gpu, keep_alive=args.keep_alive)
    load_ns = payload.get("load_duration", 0)
    total_ns = payload.get("total_duration", 0)
    rss_after = sum(rss_kb(pid) for pid in runner_pids())

    # let the idle window expire, then confirm the runner has gone
    wait = args.keep_alive_seconds
    time.sleep(wait)
    for _ in range(40):
        if not runner_pids():
            break
        time.sleep(0.25)
    gone = not runner_pids()
    rss_idle = sum(rss_kb(pid) for pid in runner_pids()) or 0

    result = {
        "model": args.model,
        "profile": args.profile,
        "threads": args.threads,
        "gpu": args.gpu,
        "keep_alive": args.keep_alive,
        "cold_wall_ms": round(wall * 1000, 1),
        "load_1m": load_average(),
        "load_ms": round(load_ns / 1e6, 1),
        "total_server_ms": round(total_ns / 1e6, 1),
        "rss_before_mb": round(baseline / 1024, 1),
        "rss_loaded_mb": round(rss_after / 1024, 1),
        "unloaded_after_idle": gone,
        "rss_after_idle_mb": round(rss_idle / 1024, 1),
    }
    print(json.dumps(result, indent=2))
    if args.out:
        with open(args.out, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(result) + "\n")
    return 0


def cosine_rank(query, matrix, ids, depth=10):
    scored = []
    for doc_id, vec in zip(ids, matrix):
        score = sum(a * b for a, b in zip(query, vec))
        scored.append((score, doc_id))
    scored.sort(reverse=True)
    return [doc_id for _score, doc_id in scored[:depth]]


def cmd_quality(args):
    docs = read_corpus(args.corpus)
    started = time.time()
    vectors = []
    for start in range(0, len(docs), args.batch):
        chunk = docs[start:start + args.batch]
        payload, _wall = embed(args.model, [d["text"] for d in chunk], args.threads, args.gpu)
        vectors.extend(payload["embeddings"])
        done = min(start + args.batch, len(docs))
        if done % 512 == 0 or done == len(docs):
            print(f"  embedded {done}/{len(docs)} in {time.time() - started:.0f}s", flush=True)

    ticket_docs = [d for d in docs if d["arch"] is not None]
    stride = max(1, len(ticket_docs) // args.queries)
    query_docs = ticket_docs[::stride][: args.queries]
    query_texts = [QUERY_PREFIX.format(query=d["title"]) for d in query_docs]
    qpayload, _wall = embed(args.model, query_texts, args.threads, args.gpu)
    query_vectors = qpayload["embeddings"]

    ids = [d["id"] for d in docs]
    arch = {d["id"]: d["arch"] for d in docs}
    per_query = []
    for doc, qvec in zip(query_docs, query_vectors):
        gold = {d["id"] for d in ticket_docs if arch[d["id"]] == doc["arch"]}
        top = cosine_rank(qvec, vectors, ids)
        hits = sum(1 for doc_id in top if doc_id in gold)
        first = next((rank for rank, doc_id in enumerate(top, start=1) if doc_id in gold), None)
        per_query.append(
            {
                "query": doc["title"],
                "recall": round(hits / 10.0, 3),
                "hit_at_1": 1 if first == 1 else 0,
                "mrr": round(1.0 / first, 3) if first else 0.0,
                "gold": len(gold),
                "top10": top,
            }
        )

    result = {
        "model": args.model,
        "threads": args.threads,
        "gpu": args.gpu,
        "documents": len(docs),
        "queries": len(per_query),
        "load_1m": load_average(),
        "recall_at_10": round(statistics.mean(q["recall"] for q in per_query), 3),
        "hit_at_1": round(statistics.mean(q["hit_at_1"] for q in per_query), 3),
        "mrr_at_10": round(statistics.mean(q["mrr"] for q in per_query), 3),
        "top10": {q["query"]: q["top10"] for q in per_query},
        "detail": per_query,
    }
    print(json.dumps({k: v for k, v in result.items() if k not in ("top10", "detail")}, indent=2))
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(result, fh, indent=2)
    return 0


def cmd_compare(args):
    with open(args.a, encoding="utf-8") as fh:
        a = json.load(fh)
    with open(args.b, encoding="utf-8") as fh:
        b = json.load(fh)
    shared = sorted(set(a["top10"]) & set(b["top10"]))
    same = sum(1 for q in shared if a["top10"][q] == b["top10"][q])
    overlap = []
    for q in shared:
        left, right = set(a["top10"][q]), set(b["top10"][q])
        overlap.append(len(left & right) / 10.0)
    result = {
        "a": {"model": a["model"], "recall_at_10": a["recall_at_10"], "hit_at_1": a["hit_at_1"], "mrr_at_10": a["mrr_at_10"]},
        "b": {"model": b["model"], "recall_at_10": b["recall_at_10"], "hit_at_1": b["hit_at_1"], "mrr_at_10": b["mrr_at_10"]},
        "queries": len(shared),
        "identical_top10": same,
        "mean_top10_overlap": round(statistics.mean(overlap), 3) if overlap else None,
        "recall_gap": round(a["recall_at_10"] - b["recall_at_10"], 3),
        "mrr_gap": round(a["mrr_at_10"] - b["mrr_at_10"], 3),
    }
    print(json.dumps(result, indent=2))
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(result, fh, indent=2)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--corpus", default="/tmp/r13-corpus/documents.jsonl")
    sub = ap.add_subparsers(dest="command", required=True)

    corpus = sub.add_parser("corpus", help="build the deterministic corpus")
    corpus.add_argument("--out", default="/tmp/r13-corpus/documents.jsonl")
    corpus.add_argument("--tickets", type=int, default=1800)
    corpus.add_argument("--prose", type=int, default=200)
    corpus.add_argument("--seed", type=int, default=42)
    corpus.set_defaults(func=cmd_corpus)

    def profile_args(p):
        p.add_argument("--model", required=True)
        p.add_argument("--threads", type=int, required=True, help="core count of the profile")
        p.add_argument("--gpu", type=int, default=0, help="layers on the GPU; 0 is CPU-only")
        p.add_argument("--profile", default="unstated", help="profile name for the record")
        p.add_argument("--out", default=None)

    throughput = sub.add_parser("throughput", help="chunks/s for backfill batches")
    profile_args(throughput)
    throughput.add_argument("--chunks", type=int, default=1200)
    throughput.add_argument("--batch", type=int, default=128)
    throughput.add_argument("--client-timeout", type=float, default=30.0)
    throughput.set_defaults(func=cmd_throughput)

    latency = sub.add_parser("latency", help="single-query latency")
    profile_args(latency)
    latency.add_argument("--requests", type=int, default=40)
    latency.set_defaults(func=cmd_latency)

    cold = sub.add_parser("coldstart", help="first request after an idle unload")
    profile_args(cold)
    cold.add_argument("--keep-alive", default="5s")
    cold.add_argument("--keep-alive-seconds", type=float, default=8.0)
    cold.set_defaults(func=cmd_coldstart)

    quality = sub.add_parser("quality", help="recall on a fixed query set")
    quality.add_argument("--model", required=True)
    quality.add_argument("--threads", type=int, default=8)
    quality.add_argument("--gpu", type=int, default=0)
    quality.add_argument("--queries", type=int, default=24)
    quality.add_argument("--batch", type=int, default=128)
    quality.add_argument("--out", required=True)
    quality.set_defaults(func=cmd_quality)

    compare = sub.add_parser("compare", help="two quality runs, side by side")
    compare.add_argument("--a", required=True)
    compare.add_argument("--b", required=True)
    compare.add_argument("--out", default=None)
    compare.set_defaults(func=cmd_compare)

    args = ap.parse_args()
    if getattr(args, "corpus", None) and args.command != "corpus" and not os.path.exists(args.corpus):
        print(f"missing corpus at {args.corpus}; run the corpus subcommand first", file=sys.stderr)
        return 2
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
