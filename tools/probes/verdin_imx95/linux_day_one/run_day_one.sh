#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host wrapper: run day_one.sh on the Verdin iMX95's Linux over ssh, with
# the M4 read list (driver/docs/imx95-display-allow.txt), and write the
# record to stdout. The board is named by the environment only:
#   VI95_SSH       the ssh destination (user@host or an ssh config name)
#   VI95_SSH_OPTS  optional extra ssh options (word-split)
#   VI95_SUDO      optional prefix when the login is not root ("sudo -n")
#   VI95_SSH_CMD   the ssh program (default ssh; the selftest's fake)
#   VI95_REMOTE_TMP the board's directory for the two files (default /tmp,
#                  a tmpfs on Toradex's image: nothing reaches the eMMC)
# Each ssh command line is short: the board's Dropbear (v2022.83) reset the
# connection when the whole script came as one command-line argument
# (2026-11-25). So the script and the read list are copied into a fresh
# directory under the board's /tmp over the connection's stdin (`cat >`),
# run from there, and the directory removed at the end, also on an error.
# Usage:
#   VI95_SSH=... tools/probes/verdin_imx95/linux_day_one/run_day_one.sh [day_one.sh options] > record.txt
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
allow=${ALLOW_FILE:-$repo/driver/docs/imx95-display-allow.txt}
: "${VI95_SSH:?set VI95_SSH to the board's ssh destination}"
ssh_=${VI95_SSH_CMD:-ssh}
tmp=${VI95_REMOTE_TMP:-/tmp}
case $tmp in
/*) ;;
*) echo "run_day_one.sh: VI95_REMOTE_TMP must be an absolute path" >&2; exit 2 ;;
esac
for a in "$@"; do
	case $a in
	*[!A-Za-z0-9_./:=-]*) echo "run_day_one.sh: argument '$a' has a character the remote shell would read" >&2; exit 2 ;;
	esac
done

remote() { # one short command on the board
	# shellcheck disable=SC2086
	"$ssh_" -o BatchMode=yes ${VI95_SSH_OPTS:-} "$VI95_SSH" "$@"
}

dir=$(remote "mktemp -d $tmp/vi95-day-one.XXXXXX" </dev/null)
case $dir in
"$tmp"/vi95-day-one.*) ;;
*) echo "run_day_one.sh: the board's mktemp gave '$dir'" >&2; exit 1 ;;
esac
trap 'remote "rm -rf $dir" </dev/null || echo "run_day_one.sh: could not remove $dir on the board" >&2' EXIT
remote "cat > $dir/day_one.sh" < "$here/day_one.sh"
remote "cat > $dir/readlist" < "$allow"
remote "${VI95_SUDO:-} sh $dir/day_one.sh --readlist $dir/readlist $*" </dev/null
