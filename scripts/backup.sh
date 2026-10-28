#!/bin/sh
# Copy an encrypted snapshot of the database from HOST to backups/.
#
# The server takes the snapshot while it runs (no downtime), and it is
# encrypted here with age as it arrives: the plain copy never rests on a
# disk. You provide the age recipients in .env (BACKUP_RECIPIENTS); only
# the matching identity, kept off the server, can decrypt it.
#
# Backups are kept 6 months at most (spec section 9): older ones in
# backups/, by the date in their name, are deleted on every run. The deletion ledger is not in a
# backup (ADR 0019); make restore replays it.
set -eu

cd "$(dirname "$0")/.."
. scripts/host.sh

command -v age >/dev/null 2>&1 ||
	fail "age is not installed (https://age-encryption.org): brew install age, or apt install age"
[ -n "${BACKUP_RECIPIENTS:-}" ] ||
	fail "BACKUP_RECIPIENTS is not set in .env: the age public keys (age1…) backups are encrypted to; see deploy/README.md, \"Backups\""

recipients=
for r in $BACKUP_RECIPIENTS; do
	recipients="$recipients -r $r"
done

backups=backups
mkdir -p "$backups"
# Personal data, even encrypted: readable by you only.
umask 077
name="$backups/drywire-$(date -u +%Y%m%d-%H%M%S).db.age"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp" "$name.partial"' EXIT

echo "==> snapshot of the database on $HOST, encrypted to $name"
# A pipeline's status is its last command's: the failure of the first
# half is written down so that it is not lost.
# shellcheck disable=SC2086 # recipients is a list of options
{ on_host "sh host/backup.sh" || echo failed >"$tmp/failed"; } |
	age $recipients -o "$name.partial"
[ ! -e "$tmp/failed" ] || fail "the snapshot failed on $HOST; nothing was saved"
mv "$name.partial" "$name"
echo "saved $name ($(wc -c <"$name" | tr -d ' ') bytes)"

cutoff=$(backup_cutoff)
deleted=0
for f in "$backups"/drywire-*.db.age; do
	d=$(backup_date "$f")
	if [ -n "$d" ] && [ "$d" -lt "$cutoff" ]; then
		rm -f "$f"
		deleted=$((deleted + 1))
	fi
done
[ "$deleted" -eq 0 ] || echo "deleted $deleted backup(s) older than 6 months"
