#!/usr/bin/env python3
"""Build a real-embedding corpus for the vector ANN benchmark.

Why this exists: the synthetic fixture answers latency and index-size questions
honestly, but a recall number taken on vectors drawn from random centres plus
jitter is a number about the generator. Real embeddings cluster, and they are
anisotropic in ways a random generator does not reproduce. So recall is
measured here, on vectors that EmbeddingGemma 2 actually produced for text the
product would plausibly store, and the two results are reported separately.

Model: google/embeddinggemma-2 (740M parameters, Apache 2.0, 768 dimensions
native), the product's default. Task prefixes are the ones the model card
specifies for retrieval:
  document  title: {title} | text: {content}
  query     task: search result | query: {query}

Corpus: 18,000 ticket-shaped documents built from PSA issue archetypes with
variable substitution, plus 2,000 prose chunks taken from this repository's
documentation. Tickets are the bulk because that is what a PSA stores; the
prose chunks keep the corpus from being 30 archetypes repeated.

Tenants get documents at random, independent of archetype, so a tenant occupies
no region of the embedding space. Shares match the synthetic run: one small
tenant at 0.66% of the corpus and one large tenant at a third.

Outputs, all tab separated, into --out:
  documents.tsv  tenant, entity_kind, entity_id, text
  vectors.tsv    tenant, entity_kind, entity_id, [vector literal]
  queries.tsv    tenant, q, [vector literal]

Requires an Ollama server holding the model:
  docker run -d --name p52-ollama -p 11434:11434 \\
    -v p52-ollama-models:/root/.ollama ollama/ollama:latest
  docker exec p52-ollama ollama pull embeddinggemma-2:740m-q8_0
"""

import argparse
import json
import os
import random
import re
import sys
import time
import urllib.request

MODEL = "embeddinggemma-2:740m-q8_0"
OLLAMA = os.environ.get("OLLAMA_URL", "http://localhost:11434")


def embed_batch(texts):
    payload = json.dumps({"model": MODEL, "input": texts}).encode()
    req = urllib.request.Request(
        OLLAMA + "/api/embed", data=payload, headers={"Content-Type": "application/json"}
    )
    for attempt in range(5):
        try:
            with urllib.request.urlopen(req, timeout=900) as resp:
                return json.load(resp)["embeddings"]
        except Exception as exc:  # noqa: BLE001 - retry the transport, not the model
            if attempt == 4:
                raise
            print(f"  embed retry {attempt + 1}: {exc}", file=sys.stderr)
            time.sleep(3 * (attempt + 1))


# --- ticket text ------------------------------------------------------------

CLIENTS = [
    "Northwind Dental", "Cascade Legal", "Harbor Physio", "Beacon Logistics",
    "Redwood Clinic", "Summit Accounting", "Lakeside Manufacturing", "Orchard Foods",
    "Granite Insurance", "Bluefin Marine", "Cedar Ridge Council", "Fairview Schools",
    "Ironwood Fabrication", "Willowbrook Care", "Parkline Properties", "Delta Print Works",
]

SITES = ["Head Office", "Warehouse B", "Clinic North", "Plant 2", "Remote Branch",
         "Distribution Centre", "Store 14", "Annex", "Data Room", "Loading Bay"]

DEVICES = ["SRV-DC01", "SRV-FILE02", "FW-EDGE01", "AP-LOBBY3", "NAS-BACKUP1",
           "PRN-RECEPTION", "LT-FIN-014", "WS-DESIGN-07", "SW-CORE01", "UPS-RACK2"]

ERRORS = ["VSS_E_WRITERERROR_RETRYABLE", "0x80070005", "EVENT 4625", "0x800706BE",
          "HTTP 503", "STATUS_ACCESS_DENIED", "0xC000021A", "SMTP 550 5.7.1",
          "0x0000007B", "CERT_E_EXPIRED"]

TIMES = ["02:15", "03:40", "01:05", "23:50", "04:20", "06:10"]

ARCHETYPES = [
    ("Backup job failed for {client}",
     "The nightly backup for {client} at {site} failed at {time} with {error} while writing the snapshot for {device}."),
    ("Printer offline at {site}",
     "Users at {site} report {device} shows offline in the queue. The print spooler restarts but the device drops again within ten minutes."),
    ("MFA reset request for {client}",
     "A user at {client} replaced their phone and cannot complete sign-in. They need the authenticator registration cleared before the next shift."),
    ("Disk space low on {device}",
     "{device} at {client} is at 96 percent on the system volume. The log directory is the largest consumer and needs a retention change."),
    ("VPN drops for {client} remote staff",
     "Remote staff at {client} lose the tunnel every twenty minutes. The client logs {error} at reconnect and the gateway shows a rekey timeout."),
    ("Email delivery failing for {client}",
     "Outbound mail from {client} is deferred with {error}. The relay at {site} reports the receiving host throttling the connection."),
    ("Patch window overran at {site}",
     "The monthly patch window at {site} ran past the maintenance window because {device} rebooted twice and stalled on update rollback."),
    ("Onboarding for new starter at {client}",
     "A new starter at {client} needs the standard build, mailbox, and access to the shared drive before Monday. Hardware is staged at {site}."),
    ("Slow login on {device}",
     "Sign-in on {device} takes over two minutes at {client}. The profile service log shows {error} and a stalled group policy client."),
    ("Certificate expiry warning for {client}",
     "The wildcard certificate used by {client} expires in nine days. Renewal needs the DNS challenge completed by the domain owner."),
    ("File server latency at {site}",
     "Opening documents from {device} at {site} stalls for several seconds. Disk queue length stays above four during the morning peak."),
    ("Antivirus quarantine on {device}",
     "A quarantined item on {device} at {client} keeps returning after release. The hash is flagged by the definition set from this morning."),
    ("New site setup for {client}",
     "{client} is opening {site}. Needs firewall, wireless, and two printers configured, with the circuit due to activate next week."),
    ("Teams call quality for {client}",
     "Calls from {site} break up during the afternoon. The access point at {device} shows high channel utilisation and retries."),
    ("Server reboot loop on {device}",
     "{device} at {client} restarts every few minutes. The last boot log records {error} before the crash dump is written."),
    ("Password expiry complaints at {client}",
     "Several users at {client} were prompted to change passwords mid-session. The domain policy is fourteen days and the notice is not reaching them."),
    ("Scanner not detected on {device}",
     "The document scanner attached to {device} is missing from the capture tool at {client}. The driver reinstalls but the device disappears again."),
    ("Backup restore request for {client}",
     "A user at {client} needs a folder restored from the copy taken before the weekend. The path is on the file share at {site}."),
    ("Switch port flapping at {site}",
     "Port status on {device} at {site} alternates up and down. The event log records {error} on the uplink module."),
    ("Mailbox quota reached for {client}",
     "A mailbox at {client} bounced with {error}. The archive policy has not applied to this account since the migration."),
    ("Wi-Fi coverage gap at {site}",
     "Staff at {site} lose wireless in the far corridor. Signal drops below the roaming threshold with no neighbour to hand over to."),
    ("Application crash in {client} line of business app",
     "The practice management application at {client} closes without a message when opening a client record. Event log shows {error}."),
    ("Ransomware canary triggered at {client}",
     "A canary file on {device} was modified outside the expected process. The host is isolated and awaiting triage at {site}."),
    ("Scheduled task not running on {device}",
     "The reporting task on {device} at {client} has not run since the last reboot. The task account password was rotated."),
    ("Printer driver deployment failed at {site}",
     "The driver package pushed to {site} failed on three machines with {error}. The package is signed but the catalog is stale."),
    ("Voicemail to email stopped for {client}",
     "Voicemail messages are no longer arriving as attachments at {client}. The relay accepts the message and then defers with {error}."),
    ("Laptop battery swelling reported at {client}",
     "A user at {client} reports the chassis on {device} is lifting at the keyboard. The device is out of warranty and needs replacement."),
    ("Group policy not applying to {device}",
     "{device} at {client} keeps the old drive mappings. The policy log records {error} and the machine account is in the wrong container."),
    ("Time drift on {device}",
     "Clocks on {device} at {client} run four minutes slow. The domain hierarchy shows the source as unreachable from {site}."),
    ("Large mailbox migration for {client}",
     "Moving {client} to the new tenant next month. Two mailboxes exceed forty gigabytes and need a staged cutover."),
]

DETAILS = [
    "The user has a deadline this afternoon and asked for a workaround.",
    "This is the third occurrence this month and the second since the last change.",
    "A screenshot is attached to the ticket showing the dialog.",
    "The site contact is available after ten and prefers a call before any change.",
    "Last successful run was four days ago according to the job history.",
    "The vendor support case is open but has not produced a fix.",
    "The device was reimaged three weeks ago and the issue started afterwards.",
    "No configuration change was recorded on the affected systems.",
    "The monitoring alert cleared on its own after twenty minutes.",
    "Two other users at the same site reported the same symptom today.",
    "Access is needed before the audit on Friday.",
    "The previous engineer left a note that this was intermittent.",
]


def ticket(rng):
    title, body = rng.choice(ARCHETYPES)
    subs = {
        "client": rng.choice(CLIENTS),
        "site": rng.choice(SITES),
        "device": rng.choice(DEVICES),
        "error": rng.choice(ERRORS),
        "time": rng.choice(TIMES),
    }
    return title.format(**subs), (body.format(**subs) + " " + rng.choice(DETAILS))


def doc_chunks(repo, wanted):
    """Prose paragraphs from the repository, for topical variety."""
    paras = []
    for root, dirs, files in os.walk(os.path.join(repo, "docs")):
        dirs[:] = [d for d in dirs if d not in (".git", "node_modules")]
        for name in sorted(files):
            if not name.endswith(".md"):
                continue
            path = os.path.join(root, name)
            heading = "none"
            try:
                with open(path, encoding="utf-8", errors="replace") as fh:
                    text = fh.read()
            except OSError:
                continue
            for line in text.splitlines():
                if line.startswith("#"):
                    heading = re.sub(r"^#+\s*", "", line).strip()
                stripped = line.strip()
                if len(stripped.split()) >= 45:
                    paras.append((heading, stripped))
    rng = random.Random(7)
    rng.shuffle(paras)
    return [(h, p) for h, p in paras[:wanted]]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default=os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
    ap.add_argument("--out", default="/tmp/p52-corpus")
    ap.add_argument("--documents", type=int, default=20000)
    ap.add_argument("--docs-chunks", type=int, default=2000)
    ap.add_argument("--small-pct", type=float, default=0.66)
    ap.add_argument("--large-pct", type=float, default=33.3)
    ap.add_argument("--queries", type=int, default=20)
    ap.add_argument("--batch", type=int, default=32)
    args = ap.parse_args()

    os.makedirs(args.out, exist_ok=True)
    rng = random.Random(42)

    # --- corpus -------------------------------------------------------------
    n_tickets = args.documents - args.docs_chunks
    docs = []
    for i in range(n_tickets):
        title, body = ticket(rng)
        docs.append(("ticket", title, body))
    for heading, para in doc_chunks(args.repo, args.docs_chunks):
        docs.append(("kb_article", heading, para))

    # --- tenants ------------------------------------------------------------
    n_small = max(20, round(args.documents * args.small_pct / 100.0))
    n_large = max(100, round(args.documents * args.large_pct / 100.0))
    n_fill = args.documents - n_small - n_large
    if n_fill < 0:
        print("corpus too small for the requested shares", file=sys.stderr)
        return 2
    labels = ["S"] * n_small + ["L"] * n_large
    fillers = 13
    per_fill = n_fill // fillers
    for k in range(fillers):
        labels += [f"f{k + 1}"] * per_fill
    labels += ["f1"] * (args.documents - len(labels))
    rng.shuffle(labels)

    rows = []
    for i, ((kind, title, body), tenant) in enumerate(zip(docs, labels)):
        entity_id = "%08x-0000-4000-8000-%012x" % (i, i)
        text = f"title: {title} | text: {body}"
        rows.append((tenant, kind, entity_id, text, title))

    with open(os.path.join(args.out, "documents.tsv"), "w", encoding="utf-8") as fh:
        for tenant, kind, entity_id, text, _title in rows:
            fh.write(f"{tenant}\t{kind}\t{entity_id}\t{text}\n")

    print(f"documents: {len(rows)}  tenants: S={n_small} L={n_large} fillers={fillers}x{per_fill}")

    # --- embed documents ----------------------------------------------------
    started = time.time()
    with open(os.path.join(args.out, "vectors.tsv"), "w", encoding="utf-8") as fh:
        for start in range(0, len(rows), args.batch):
            chunk = rows[start:start + args.batch]
            vectors = embed_batch([r[3] for r in chunk])
            for (tenant, kind, entity_id, _text, _title), vec in zip(chunk, vectors):
                fh.write("%s\t%s\t%s\t%s\n" % (tenant, kind, entity_id, json.dumps(vec)))
            done = min(start + args.batch, len(rows))
            if done % 640 == 0 or done == len(rows):
                rate = done / max(time.time() - started, 1e-9)
                print(f"  embedded {done}/{len(rows)}  {rate:.1f}/s", flush=True)

    # --- queries ------------------------------------------------------------
    # A query is the subject line of a stored document, embedded with the
    # query prefix: what a user types when looking for that ticket.
    by_tenant = {"S": [], "L": []}
    for tenant, _kind, entity_id, _text, title in rows:
        if tenant in by_tenant and len(by_tenant[tenant]) < args.queries:
            by_tenant[tenant].append((entity_id, title))
    with open(os.path.join(args.out, "queries.tsv"), "w", encoding="utf-8") as fh:
        for tenant, items in by_tenant.items():
            texts = [f"task: search result | query: {title}" for _eid, title in items]
            vectors = embed_batch(texts)
            for q, ((_eid, _title), vec) in enumerate(zip(items, vectors), start=1):
                fh.write("%s\t%d\t%s\n" % (tenant, q, json.dumps(vec)))
    print(f"queries: {sum(len(v) for v in by_tenant.values())}")
    print(f"wrote {args.out}/vectors.tsv and {args.out}/queries.tsv in {time.time() - started:.0f}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
