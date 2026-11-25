#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Usage: t6-uboot-log.sh <record-dir> <date YYYY-MM-DD> <name> <sha8>
# Copies the text logs of a NanoPC-T6 U-Boot record (records/nanopc-t6/<stamp>-<name>-<sha8>/) to
# tools/cts/log/<date>-t6-uboot-<name>-<sha8>/, each file passed through console-redact.py, then
# refuses (exit 1, nothing left behind) if the copy still matches the acceptance regex (rfc2217 URL,
# serial tty path, MAC, tailnet IP 100.64.0.0/10), a UUID or a 'serial number: X' line. Never
# overwrites an existing destination (exit 1). Binary files (a NUL byte) are skipped. Exit 2: usage.
# T6_UBOOT_LOG_ROOT overrides the log root (the selftest uses it). Host only; scratch next to the
# destination, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=${T6_UBOOT_LOG_ROOT:-$here/log}
if [ $# -ne 4 ]; then
    echo "usage: t6-uboot-log.sh <record-dir> <date YYYY-MM-DD> <name> <sha8>" >&2
    exit 2
fi
rec=$1 date=$2 name=$3 sha=$4
[ -d "$rec" ] || { echo "t6-uboot-log: no such record dir: $rec" >&2; exit 2; }
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
trap 'rm -rf "$stage"' EXIT

secrets='rfc2217://|/dev/tty(USB|ACM)|([0-9a-f]{2}:){5}[0-9a-f]{2}|100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.'
uuid='[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'
serial='serial[ _-]?(number|no\.?|#)[ \t]*[:=#][ \t]*[^ \t]'

n=0
while IFS= read -r -d '' f; do
    rel=${f#"$rec"/}
    if [ -s "$f" ] && ! grep -qI . "$f" 2>/dev/null; then continue; fi   # binary: skipped
    mkdir -p "$stage/$(dirname "$rel")"
    python3 -I "$here/console-redact.py" < "$f" > "$stage/$rel" || { echo "t6-uboot-log: redactor failed on $rel" >&2; exit 1; }
    n=$((n + 1))
done < <(find "$rec" -type f -print0 | sort -z)

fail=0
if grep -rEin -- "$secrets" "$stage" >&2; then echo "t6-uboot-log: refusing: a secret survived redaction" >&2; fail=1; fi
if grep -rEin -- "$uuid" "$stage" >&2; then echo "t6-uboot-log: refusing: a UUID survived" >&2; fail=1; fi
if grep -rEin -- "$serial" "$stage" >&2; then echo "t6-uboot-log: refusing: a serial number survived" >&2; fail=1; fi
[ $fail -eq 0 ] || exit 1
[ $n -gt 0 ] || { echo "t6-uboot-log: no text files in $rec" >&2; exit 1; }
mv "$stage" "$dest" || exit 1
chmod 755 "$dest"
trap - EXIT
echo "t6-uboot-log: $n files -> $dest"
