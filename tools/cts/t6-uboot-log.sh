#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Usage: t6-uboot-log.sh <record-dir> <date YYYY-MM-DD> <name> <sha8>
# Copy one NanoPC-T6 U-Boot run record — a driver/tools/uboot-t6/prompt-reads.sh
# or prompt-go.sh --go record directory (its record.txt, uart.log, typed.log,
# answers.txt, image.log, verdict.txt; README.md beside those tools) — into
# tools/cts/log/<date>-t6-uboot-<name>-<sha8>/, redacted. The name is what ran
# (prompt-reads, t6_cold, t6_fw, gl_test, a dEQP row's label); the sha8 is the
# origin/main commit the board image was built from.
#
# Every text file is copied through t6-uboot-redact.py --redact: the kinds the
# record carries (a UART path or device name, a board or tailnet address in the
# 10/8, 172.16/12, 192.168/16, 100.64/10, 169.254/16 ranges, a MAC, a serial
# number, a partition UUID, a ts.net host name, a login at a host, a link-local
# IPv6 address, a /dev/serial/ path, a serial-bridge port, an rfc2217:// URL, a
# home path) become placeholders; a dEQP version line keeps its bracketed form.
# The staged copy is then checked with t6-uboot-redact.py --check: a kind that
# survived the redaction refuses the run (exit 1, every hit named by kind and
# line only — never its text, the staged tree removed, nothing left behind).
# A binary file (a NUL byte) is skipped. An existing destination is never
# overwritten (exit 1). A record with no text file is refused (exit 1). Exit 2:
# usage, a bad date/name/sha8, a missing record dir, a staged file the check
# could not read. T6_UBOOT_LOG_ROOT overrides the log root (the selftest uses
# it); the stage directory is mktemp'd inside it, next to the destination, and
# removed on any exit — never under /tmp. Host only.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=${T6_UBOOT_LOG_ROOT:-$here/log}
redact=$here/t6-uboot-redact.py

if [ $# -ne 4 ]; then
    echo "usage: t6-uboot-log.sh <record-dir> <date YYYY-MM-DD> <name> <sha8>" >&2
    exit 2
fi
rec=$1 date=$2 name=$3 sha=$4
[ -d "$rec" ] || { echo "t6-uboot-log: no such record dir: $rec" >&2; exit 2; }
rec=$(cd "$rec" && pwd)
[[ $date =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "t6-uboot-log: bad date: $date" >&2; exit 2; }
[[ $name =~ ^[A-Za-z0-9][A-Za-z0-9_-]*$ ]] || { echo "t6-uboot-log: bad name: $name" >&2; exit 2; }
[[ $sha =~ ^[0-9a-f]{8}$ ]] || { echo "t6-uboot-log: bad sha8: $sha" >&2; exit 2; }
dest=$root/$date-t6-uboot-$name-$sha
if [ -e "$dest" ] || [ -L "$dest" ]; then
    echo "t6-uboot-log: $dest exists; not overwriting" >&2
    exit 1
fi
mkdir -p "$root" || exit 2
stage=$(mktemp -d "$root/.t6-uboot-log.XXXXXX") || exit 2
trap 'rm -rf -- "$stage"' EXIT

n=0 binary=0
while IFS= read -r -d '' f; do
    rel=${f#"$rec"/}
    if [ ! -r "$f" ]; then echo "t6-uboot-log: cannot read $f" >&2; exit 2; fi
    mkdir -p -- "$stage/$(dirname -- "$rel")" || exit 2
    rc=0
    python3 -I "$redact" --redact < "$f" > "$stage/$rel" || rc=$?
    case $rc in
        0) n=$((n + 1)) ;;
        76) rm -f -- "$stage/$rel"; binary=$((binary + 1)) ;;   # binary: skipped
        2) echo "t6-uboot-log: the redactor refused $rel" >&2; exit 2 ;;
        *) echo "t6-uboot-log: the redactor failed on $rel (exit $rc)" >&2; exit 1 ;;
    esac
done < <(find "$rec" -type f -print0 | sort -z)

if [ $n -eq 0 ]; then
    echo "t6-uboot-log: no text files under $rec (a record is record.txt, uart.log, typed.log, answers.txt, image.log, verdict.txt)" >&2
    exit 1
fi

out=$(python3 -I "$redact" --check "$stage" 2>&1)
rc=$?
if [ $rc -eq 2 ]; then
    printf '%s\n' "$out" >&2
    echo "t6-uboot-log: the check could not read the staged tree" >&2
    exit 2
elif [ $rc -ne 0 ]; then
    printf '%s\n' "$out" >&2
    echo "t6-uboot-log: refusing $dest: the redaction left a secret kind (each line names a kind and a line, never the text); fix that line at the source and run again" >&2
    exit 1
fi

mv -- "$stage" "$dest" || exit 1
chmod 755 "$dest"
trap - EXIT
echo "t6-uboot-log: $n text file(s) -> $dest$( [ $binary -gt 0 ] && echo ", $binary binary skipped")"
