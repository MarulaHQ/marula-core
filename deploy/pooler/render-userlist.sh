#!/bin/sh
# Render PgBouncer's auth file from the environment.
#
# Why this exists (F65): `deploy/pooler/userlist.txt` used to be a checked-in
# file holding both database passwords — the published dev values, which are
# also the values migration 0114 mints for the app role. A password in git is
# a password anyone who has read the repository holds, and the file could not
# follow a rotated password without a hand edit that nothing verified.
#
# PgBouncer itself cannot read the environment (its auth_file is a path), so
# the file is produced by this script instead: the compose `pooler-auth`
# one-shot runs it into a volume the pooler mounts, and an operator deploying
# a pooler that compose does not manage runs it once per container start
# (deploy/pooler/README.md has the command).
#
# The file format is PgBouncer's, not ours: one `"role" "password"` pair per
# line, double quotes, `\` and `"` backslash-escaped, and no comments — a
# comment line makes PgBouncer refuse the whole file. That is why the notes
# live here and in the README instead of inside the output.
#
# Usage: render-userlist.sh [output-path]   (default /auth/userlist.txt)
# Reads: POSTGRES_PASSWORD (owner role), MARULA_APP_PASSWORD (app role),
#        POOLER_OWNER_ROLE / POOLER_APP_ROLE (role names; default `marula` and
#        `marula_app`, which is what the compose shape creates). A deployment
#        shaped differently — the e2e stack's scratch database is owned by
#        `postgres` — passes the names its database actually has.
# Writes: that one file, mode 644, atomically (via a temporary beside it).
set -eu

out=${1:-/auth/userlist.txt}

: "${POSTGRES_PASSWORD:?POSTGRES_PASSWORD is required (the marula owner role password)}"
: "${MARULA_APP_PASSWORD:?MARULA_APP_PASSWORD is required (the marula_app role password)}"

# Backslash and double quote are the only characters with meaning inside a
# quoted field. A generated hex value contains neither; a hand-picked one
# might, and a file that silently mis-parses is a pooler that authenticates
# nobody.
#
# Walked character by character rather than through sed: the image this runs
# in is a hardened Postgres client with no sed (found the hard way — the
# substitution silently produced empty fields), and a renderer that needs an
# editor to write a password file is a renderer with a hidden dependency.
escape() {
	_in=$1
	_out=
	while [ -n "$_in" ]; do
		_rest=${_in#?}
		_c=${_in%"$_rest"}
		_in=$_rest
		# Two comparisons rather than a case pattern: a pattern holding a
		# quote is a parse puzzle for humans and linters alike, and this is
		# the whole of the escaping rule.
		if [ "$_c" = "\\" ] || [ "$_c" = '"' ]; then
			_out="$_out\\$_c"
		else
			_out="$_out$_c"
		fi
	done
	printf '%s' "$_out"
}

tmp="$out.tmp.$$"
trap 'rm -f "$tmp"' EXIT INT TERM

# 644, not 600: the pooler runs as its own uid (65532) and the volume arrives
# root-owned, so a 600 file written here would be one the pooler cannot read.
# 644 is readable inside two containers that already hold the pooler's own
# credentials in memory, and nowhere else — the volume is not a host path.
umask 022
{
	printf '"%s" "%s"\n' "${POOLER_OWNER_ROLE:-marula}" "$(escape "$POSTGRES_PASSWORD")"
	printf '"%s" "%s"\n' "${POOLER_APP_ROLE:-marula_app}" "$(escape "$MARULA_APP_PASSWORD")"
} >"$tmp"

# Rename, so a pooler restarted mid-write never reads half a file.
mv "$tmp" "$out"
trap - EXIT INT TERM

echo "wrote $out (2 roles) from POSTGRES_PASSWORD and MARULA_APP_PASSWORD" >&2
