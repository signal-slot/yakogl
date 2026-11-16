#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/record.sh new <name>
#   tools/probes/rpi5_display/record.sh throttled <dir> <label>     (value on stdin)
#   tools/probes/rpi5_display/record.sh redact <dir>
#   tools/probes/rpi5_display/record.sh check <dir>
#
# The record directory of a Pi 5 display session:
# records/rpi5/<YYYYMMDD-HHMM>-<name>-<sha8>/ (gitignored). `new` makes it and
# prints its path as the last line (RECORDS_DIR overrides records/rpi5 for the
# self-test). `throttled` appends "<label> <value>" to throttled.txt, the value
# being the 'vcgencmd get_throttled' output or a bare 0x value (unknown when
# the board did not answer). `redact` rewrites every file of the directory:
# the names of REDACT_NAMES (comma separated host and login names of the
# session, from the environment), IPv4 and IPv6 addresses, serial device
# paths and home directories. `check` exits 1 when a file still holds one of
# those, 0 otherwise. Nothing of the environment is written to a file.
# Exit: 0 done; 1 check refused; 2 usage.
set -u
export LC_ALL=C

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)

# patterns of what a record must not keep
IPV4='[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}'
IPV6='\b[0-9A-Fa-f]{1,4}(:[0-9A-Fa-f]{0,4}){3,7}\b'
TTY='/dev/(tty|serial)[A-Za-z0-9_./-]*'
HOMEDIR='/(home|Users)/[A-Za-z0-9_.-]+'

usage() {
	echo "usage: record.sh new <name> | throttled <dir> <label> | redact <dir> | check <dir>" >&2
	exit 2
}

dir_arg() {
	[ -n "${1:-}" ] && [ -d "$1" ] || { echo "record.sh: not a directory: ${1:-}" >&2; exit 2; }
}

cmd=${1:-}
[ -n "$cmd" ] || usage
shift
case $cmd in
new)
	name=${1:-}
	[[ $name =~ ^[A-Za-z0-9_.-]+$ ]] || { echo "record.sh: bad run name" >&2; exit 2; }
	sha=$(git -C "$ROOT" rev-parse --short=8 HEAD 2>/dev/null) || sha=
	[ -n "$sha" ] || sha=00000000
	d=${RECORDS_DIR:-$ROOT/records/rpi5}/$(date +%Y%m%d-%H%M)-$name-$sha
	mkdir -p "$d" || exit 2
	chmod 700 "$d"
	echo "$d"
	;;
throttled)
	dir_arg "${1:-}"
	label=${2:-}
	[[ $label =~ ^[A-Za-z0-9_.-]+$ ]] || usage
	v=$(head -n 1)
	if [[ $v =~ 0x[0-9a-fA-F]+ ]]; then v=${BASH_REMATCH[0]}; else v=unknown; fi
	echo "$label $v" >>"$1/throttled.txt"
	;;
redact)
	dir_arg "${1:-}"
	IFS=, read -ra names <<<"${REDACT_NAMES:-}"
	while IFS= read -r -d '' f; do
		for n in "${names[@]}"; do
			[ -n "$n" ] || continue
			esc=$(printf '%s' "$n" | sed 's/[][\.*^$/|&]/\\&/g')
			sed -i "s|$esc|<name>|g" "$f"
		done
		sed -i -E -e "s#$IPV4#<ip>#g" -e "s#$IPV6#<ip6>#g" -e "s#$TTY#<uart>#g" -e "s#$HOMEDIR#<home>#g" "$f"
	done < <(find "$1" -type f -print0)
	;;
check)
	dir_arg "${1:-}"
	bad=0
	for p in "$IPV4" "$IPV6" "$TTY" "$HOMEDIR"; do
		if grep -rqE -- "$p" "$1"; then bad=1; fi
	done
	[ $bad -eq 0 ] || { echo "record.sh: $1 keeps an address, device path or home directory" >&2; exit 1; }
	;;
*) usage ;;
esac
