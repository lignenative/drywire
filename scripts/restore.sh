#!/bin/sh
# Put a backup back on HOST: the newest in backups/, or FILE=<path>.
#
# After confirmation, the server stops, the backup is decrypted here with
# your age identity (BACKUP_IDENTITY in .env) and sent to the host, which
# checks it, replaces the database, replays the deletion ledger (nothing
# deleted since the backup comes back, spec section 9), and starts the
# server again. A backup that does not decrypt or is damaged changes
# nothing: the server starts again on its current database.
set -eu

cd "$(dirname "$0")/.."
. scripts/host.sh

command -v age >/dev/null 2>&1 ||
	fail "age is not installed (https://age-encryption.org): brew install age, or apt install age"
[ -n "${BACKUP_IDENTITY:-}" ] ||
	fail "BACKUP_IDENTITY is not set in .env: the age identity file that decrypts the backups; see deploy/README.md, \"Backups\""
# make passes .env's values as written: expand a leading ~ here.
# shellcheck disable=SC2088 # the ~ is matched as written, on purpose
case $BACKUP_IDENTITY in
"~/"*) BACKUP_IDENTITY=$HOME/${BACKUP_IDENTITY#"~/"} ;;
esac
[ -r "$BACKUP_IDENTITY" ] || fail "cannot read BACKUP_IDENTITY ($BACKUP_IDENTITY)"

file=${FILE:-}
if [ -z "$file" ]; then
	# Names sort by date: the last is the newest.
	file=$(find backups -name 'drywire-*.db.age' 2>/dev/null | sort | tail -n 1)
	[ -n "$file" ] || fail "no backup in backups/: make backup, or FILE=<path>"
fi
[ -f "$file" ] || fail "no backup at $file"

# A backup older than 6 months is out of its retention, and older than
# what the deletion ledger remembers (7 months, then pruned): restoring it
# could bring back what was deleted long ago.
taken=$(backup_date "$file")
[ -n "$taken" ] ||
	fail "cannot tell when $file was taken: make backup names them drywire-YYYYMMDD-HHMMSS.db.age"
[ "$taken" -ge "$(backup_cutoff)" ] ||
	fail "$file is older than 6 months: backups are kept 6 months at most, and deletions older than that could come back"

echo "Restore $file on $HOST."
echo "The database there now is replaced; what changed since this backup is"
echo "lost, except deletions, which are replayed. make backup first keeps a copy."
printf 'Replace it? [y/N] '
read -r answer || answer=
case $answer in
y | yes) ;;
*)
	echo "Nothing changed."
	exit 0
	;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# A pipeline's status is its last command's: a decryption failure is
# written down so that it is not lost.
if { age -d -i "$BACKUP_IDENTITY" "$file" || echo failed >"$tmp/failed"; } |
	on_host "sh host/restore.sh"; then
	[ ! -e "$tmp/failed" ] || fail "$file did not decrypt, yet the host restored something: check make status"
	echo "Restored. Next: make status"
elif [ -e "$tmp/failed" ]; then
	fail "$file did not decrypt; the server kept its current database"
else
	fail "the restore failed on $HOST (see above); the server kept its current database"
fi
