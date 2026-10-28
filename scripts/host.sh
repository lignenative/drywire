# Shared by the deployment scripts (deploy, status, logs, backup, restore):
# which host, and how to reach it. Sourced by them, never run alone.
#
# HOST is an alias from ~/.ssh/config, set in .env (never committed): the
# repository holds no host name. Drywire lives in /srv/drywire there and
# touches nothing else on the host.

if [ -z "${HOST:-}" ]; then
	echo "drywire: HOST is not set. Put the SSH alias of your server in .env:" >&2
	echo "  cp .env.example .env, then set HOST=<alias from ~/.ssh/config>" >&2
	exit 1
fi

dir=/srv/drywire
# Never ask a question on the way: a missing key or a password prompt is
# an error to read, not a hang.
ssh_opts="-o BatchMode=yes -o ConnectTimeout=10"

# ssh_host COMMAND...: run a command on the host.
ssh_host() {
	# The command is meant to run there, as written here.
	# shellcheck disable=SC2086,SC2029 # ssh_opts is a list of options
	ssh $ssh_opts "$HOST" "$@"
}

# on_host COMMAND: run a shell command on the host, in /srv/drywire.
on_host() {
	ssh_host "cd $dir 2>/dev/null || { echo 'nothing is deployed in $dir: run make deploy' >&2; exit 1; }; $1"
}

# instance_setting NAME: a setting's value in deploy/instance.env.
instance_setting() {
	sed -n "s/^$1=//p" deploy/instance.env 2>/dev/null | tail -n 1
}

fail() {
	echo "drywire: $*" >&2
	exit 1
}

# Backups are kept 6 months at most (spec section 9). Their date is the
# one in their name, drywire-YYYYMMDD-HHMMSS.db.age, which a copy or a
# restore from another disk cannot change, unlike a file's time.

# backup_cutoff: the date, as YYYYMMDD, before which a backup is too old.
backup_cutoff() {
	date -u -v-180d +%Y%m%d 2>/dev/null || date -u -d '180 days ago' +%Y%m%d
}

# backup_date FILE: the date in a backup's name, as YYYYMMDD, or nothing.
backup_date() {
	d=${1##*/}
	d=${d#drywire-}
	d=${d%%-*}
	case $d in
	[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]) echo "$d" ;;
	esac
}
