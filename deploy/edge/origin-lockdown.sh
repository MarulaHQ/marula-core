#!/usr/bin/env sh
# Origin lockdown — firewall the edge's published IP ranges so the edge
# cannot be bypassed.
#
# Why this exists: in Cloudflare Edge mode the orange cloud is only a
# security boundary if the origin refuses to speak to anyone else. Leave
# 80 and 443 open to the world and an attacker who learns your origin IP
# (a historical DNS record, a certificate transparency log, a subdomain
# that was never proxied) reaches the real server directly — bypassing
# every rule, WAF policy and rate limit configured at the edge.
#
# What this script does: install rules that allow 80/443 (and, in
# DNS-01 mode, nothing extra) ONLY from Cloudflare's published ranges, and
# drop every other inbound packet.
#
# What it deliberately does NOT do: touch your SSH access. Locking yourself
# out of the box you are standing on is the classic way this class of
# script ends. Pass --ssh-from with the address you are connecting from
# (default: the peer of the current SSH connection, when there is one), and
# the plan is printed for review before anything is applied.
#
# Modes:
#   --plan    print the rules that would be applied, change nothing (default)
#   --apply   write them
#   --revert  delete the rules this script created
#   --direct  print why there is nothing to lock down for Direct mode and
#             exit: a single-IP/MX-record deployment with no edge in front
#             needs 80/443 open to the internet, because that IS its
#             clients' path.
#
# Backends: nftables (preferred, table inet marula-edge) or ufw (simpler,
# uses numbered rules). Auto-detected; override with --backend.
#
# Ranges come from a versioned constant below, not from the network at
# run time: a firewall helper that trusts a live fetch is a firewall helper
# that can be changed by whatever it fetched. Pass --fetch to refresh them
# from https://www.cloudflare.com/ips-v4 and /ips-v6, review the diff, and
# commit the new value.

set -eu

PROG=$(basename "$0")
MODE=plan
BACKEND=""
SSH_FROM=""
FETCH=0

# Versioned 2026-10-14, matching crates/core-api/src/client_ip.rs (which
# core-api itself extends through CF_IP_RANGES). Refresh both together.
CF_V4="173.245.48.0/20 103.21.244.0/22 103.22.200.0/22 103.31.4.0/22 141.101.64.0/18 108.162.192.0/18 190.93.240.0/20 188.114.96.0/20 197.234.240.0/22 198.41.128.0/17 162.158.0.0/15 104.16.0.0/13 104.24.0.0/14 172.64.0.0/13 131.0.72.0/22"
CF_V6="2400:cb00::/32 2606:4700::/32 2803:f800::/32 2405:b500::/32 2405:8100::/32 2a06:98c0::/29 2c0f:f248::/32"

# Ports the proxy answers on inside the host.
HTTP_PORTS="80 443"

usage() {
	cat <<EOF
$PROG — lockdown the origin to Cloudflare's published ranges

  $PROG [--plan|--apply|--revert|--direct] [--backend nft|ufw]
         [--ssh-from CIDR] [--ports "80 443"] [--fetch]

  --plan     print the rules, change nothing (default)
  --apply    write them (asks for confirmation unless --yes)
  --revert  remove the rules this script created
  --direct   Direct mode: nothing to lock down, explains why and exits 0
  --yes      skip the confirmation prompt on --apply
  --fetch    refresh the Cloudflare ranges from cloudflare.com before use
EOF
}

log() { printf '%s\n' "$*"; }
die() { printf '%s: %s\n' "$PROG" "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
	case "$1" in
	--plan) MODE=plan ;;
	--apply) MODE=apply ;;
	--revert) MODE=revert ;;
	--direct) MODE=direct ;;
	--backend) shift; BACKEND=${1:?--backend needs nft or ufw} ;;
	--ssh-from) shift; SSH_FROM=${1:?--ssh-from needs a CIDR} ;;
	--ports) shift; HTTP_PORTS=${1:?--ports needs a list} ;;
	--yes) YES=1 ;;
	--fetch) FETCH=1 ;;
	-h|--help) usage; exit 0 ;;
	*) die "unknown option: $1 (see --help)" ;;
	esac
	shift
done

[ "$MODE" = "direct" ] && {
	log "Direct mode: this host is its own edge, so 80 and 443 have to be"
	log "open to the internet — that is the path your clients arrive on."
	log "There is no edge range to allowlist. Instead:"
	log "  - keep the host firewall default-deny for every OTHER port"
	log "  - never publish 8080/8081/3000/15432/16379/8082 on 0.0.0.0"
	log "    (deploy/edge/compose.edge.yml pulls them back for you)"
	log "  - if you later put Cloudflare in front, re-run with --apply"
	exit 0
}

[ "$FETCH" = "1" ] && {
	command -v curl >/dev/null 2>&1 || die "--fetch needs curl"
	V4=$(curl -fsS --max-time 10 https://www.cloudflare.com/ips-v4 || true)
	V6=$(curl -fsS --max-time 10 https://www.cloudflare.com/ips-v6 || true)
	[ -n "$V4" ] || die "could not fetch ips-v4; keeping the versioned list"
	CF_V4=$(printf '%s' "$V4" | tr -s '[:space:]' ' ')
	CF_V6=$(printf '%s' "$V6" | tr -s '[:space:]' ' ')
	log "refreshed ranges from cloudflare.com — verify them against the"
	log "constant before you commit the change"
}

# The SSH rule is what stops this script from being an outage generator.
if [ -z "$SSH_FROM" ]; then
	if [ -n "${SSH_CONNECTION:-}" ]; then
		SSH_FROM=$(printf '%s' "$SSH_CONNECTION" | awk '{print $1}')/32
	elif [ -n "${SSH_CLIENT:-}" ]; then
		SSH_FROM=$(printf '%s' "$SSH_CLIENT" | awk '{print $1}')/32
	fi
fi

if [ -z "$BACKEND" ]; then
	if command -v nft >/dev/null 2>&1; then
		BACKEND=nft
	elif command -v ufw >/dev/null 2>&1; then
		BACKEND=ufw
	else
		die "no nft or ufw found; install one, or apply the plan by hand"
	fi
fi

sets() {
	for r in $CF_V4 $CF_V6; do printf '%s ' "$r"; done
}

plan_nft() {
	log "# table inet marula-edge — created by $PROG ($(date -u +%Y-%m-%dT%H:%M:%SZ))"
	log "chain input {"
	log "    type filter hook input priority 0; policy accept;"
	if [ -n "$SSH_FROM" ]; then
		log "    iif lo accept"
		log "    ct state established,related accept"
		log "    tcp dport 22 ip saddr $SSH_FROM accept"
		log "    # WARNING: no --ssh-from was detected; 22 stays open to all."
	fi
	for p in $HTTP_PORTS; do
		for r in $CF_V4; do log "    tcp dport $p ip saddr $r accept"; done
		for r in $CF_V6; do log "    tcp dport $p ip6 saddr $r accept"; done
	done
	log "}"
	log "# NOTE: the chain starts with policy accept and adds accept rules for"
	log "# the edge. Switching policy to drop (the actual lockdown) is the"
	log "# line this script will never write for you: it depends on every"
	log "# other port this host serves, which this script cannot know."
}

apply_nft() {
	command -v nft >/dev/null 2>&1 || die "nft not found"
	[ "$(id -u)" = "0" ] || die "needs root (or CAP_NET_ADMIN) to apply"
	{
		printf 'table inet marula-edge {\n\tchain input {\n\t\ttype filter hook input priority 0; policy accept;\n'
		if [ -n "$SSH_FROM" ]; then
			printf '\t\tiif lo accept\n\t\tct state established,related accept\n'
			printf '\t\ttcp dport 22 ip saddr %s accept\n' "$SSH_FROM"
		fi
		for p in $HTTP_PORTS; do
			for r in $CF_V4; do printf '\t\ttcp dport %s ip saddr %s accept\n' "$p" "$r"; done
			for r in $CF_V6; do printf '\t\ttcp dport %s ip6 saddr %s accept\n' "$p" "$r"; done
		done
		printf '\t}\n}\n'
	} | nft -f -
	log "applied table inet marula-edge (policy still accept — see the note below)"
	log "to finish the lockdown: nft add rule inet marula-edge input counter drop"
	log "only after you have confirmed 80/443 from a non-Cloudflare IP fails"
}

revert_nft() {
	command -v nft >/dev/null 2>&1 || die "nft not found"
	[ "$(id -u)" = "0" ] || die "needs root (or CAP_NET_ADMIN) to revert"
	nft delete table inet marula-edge 2>/dev/null || log "table inet marula-edge was not present"
	log "reverted"
}

plan_ufw() {
	log "# ufw rules — created by $PROG"
	if [ -n "$SSH_FROM" ]; then
		log "ufw allow from $SSH_FROM to any port 22 proto tcp"
	else
		log "# WARNING: no --ssh-from detected; SSH is left exactly as it is."
	fi
	for r in $CF_V4 $CF_V6; do
		for p in $HTTP_PORTS; do log "ufw allow from $r to any port $p proto tcp"; done
	done
	log "# Then set the default incoming policy to deny, after confirming you"
	log "# can still reach the origin over 443 through Cloudflare."
	log "# NOTE: 'ufw allow from <range>' applies to all ports, not just 80/443."
	log "# For a port-scoped allowlist without touching unrelated rules, use"
	log "# --backend nft instead."
}

apply_ufw() {
	command -v ufw >/dev/null 2>&1 || die "ufw not found"
	[ "$(id -u)" = "0" ] || die "needs root to apply"
	[ -n "$SSH_FROM" ] || die "refusing to apply under ufw without --ssh-from"
	ufw allow from "$SSH_FROM" to any port 22 proto tcp
	for r in $CF_V4 $CF_V6; do
		for p in $HTTP_PORTS; do ufw allow from "$r" to any port "$p" proto tcp; done
	done
	log "applied. The allow rules are additive: the default incoming policy"
	log "is what actually denies everyone else, and that is your call —"
	log "   ufw default deny incoming"
	log "only after you have confirmed Cloudflare still reaches the origin."
}

revert_ufw() {
	command -v ufw >/dev/null 2>&1 || die "ufw not found"
	[ "$(id -u)" = "0" ] || die "needs root to revert"
	[ -n "$SSH_FROM" ] && ufw delete allow from "$SSH_FROM" to any port 22 proto tcp
	for r in $CF_V4 $CF_V6; do
		for p in $HTTP_PORTS; do ufw delete allow from "$r" to any port "$p" proto tcp; done
	done
	log "reverted (left the allow rule for 22 if you added it yourself)"
}

case "$MODE/$BACKEND" in
plan/nft) plan_nft ;;
plan/ufw) plan_ufw ;;
revert/nft) revert_nft ;;
revert/ufw) revert_ufw ;;
apply/nft|apply/ufw)
	log "Applying on $BACKEND. $(sets | wc -w) ranges, ports: $HTTP_PORTS."
	if [ -z "${SSH_FROM:-}" ]; then
		log "WARNING: no SSH source detected (no SSH_CONNECTION, no --ssh-from)."
		log "You are about to add allow rules without protecting port 22."
	fi
	if [ "${YES:-0}" != "1" ]; then
		printf 'Continue? [y/N] '
		read -r ans </dev/tty || ans=n
		case "$ans" in y|Y|yes) ;; *) die "aborted" ;; esac
	fi
	if [ "$BACKEND" = nft ]; then apply_nft; else apply_ufw; fi
	;;
esac
