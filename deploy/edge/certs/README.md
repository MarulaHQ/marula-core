# Cloudflare Origin CA certificate material — EDGE MODE (cf-origin) only.
#
# Drop the two files Cloudflare's dashboard gives you here:
#
#   origin.crt   the Origin CA certificate (PEM)
#   origin.key   the matching private key (PEM)
#
# then set MARULA_TLS_MODE=cf-origin and start the edge profile. Caddy
# reads them from the read-only bind mount at /etc/caddy/certs and does
# nothing else — no ACME account, no renewal, no port 80.
#
# The dashboard path is SSL/TLS -> Origin Server -> Create Certificate.
# Choose "Let Cloudflare generate a private key and a CSR", list the
# hostnames (a wildcard *.example.com covers every subdomain), RSA 2048 or
# ECDSA P-256, and a long expiry. Cloudflare keeps its own copy, so a lost
# key is a re-issue, not an outage.
#
# The private key must be readable by the uid the edge container runs as:
# the container runs as root (see the capability exemption in
# deploy/edge/compose.edge.yml), so a root-owned 0600 file works. If you
# drop the key through a userland tool running as a different uid, check
# that Caddy can actually open it — a permission failure here shows up as
# Caddy refusing to start with "reading certificate" in the log.
#
# Nothing in this directory is ever committed: the .gitignore below keeps
# certificate material out of the repository, and the compose overlay
# mounts the directory read-only.
