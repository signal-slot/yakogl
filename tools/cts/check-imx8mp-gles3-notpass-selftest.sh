#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of check-imx8mp-gles3-notpass.py: one good and several bad fixtures in $TMPDIR. Host only.
set -u
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/check-imx8mp-gles3-notpass.py"
t=$(mktemp -d "${TMPDIR:-/tmp}/notpass-shape.XXXXXX") || exit 2
trap 'rm -rf "$t"' EXIT
fail=0
printf '# synthetic\nA.a\nA.b\nA.c\nB.a\nB.b\n' > "$t/must"
H=$'# i.MX8MP dEQP-GLES3 at abc\n# measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL)\n# rows: e<i>:<status> <case>\n'
mk() { printf '%s' "$H$2" > "$t/$1"; }
mk good $'e0:Fail A.a\ne0:NotSupported A.c\ne1:NoResult B.a\n'
mk good-empty ''
mk pi4 $'D1.console.txt:Fail A.a\nD1.console.txt:Fail B.b\n'
mk pass $'e0:Pass A.a\n'
mk status $'e0:Bogus A.a\n'
mk cols $'e0:Fail A.a extra\n'
mk tab $'e0:Fail\tA.a\n'
mk unknown $'e0:Fail A.zzz\n'
mk unsorted $'e0:Fail A.c\ne0:Fail A.a\n'
mk entryorder $'e1:Fail A.a\ne0:Fail A.b\n'
mk dup $'e0:Fail A.a\ne0:Fail A.a\n'
printf 'e0:Fail A.a\n# i.MX8MP x\n# rows: r\n' > "$t/nohdr"
printf 'e0:Fail A.a\n' > "$t/rowsfirst"
printf '# i.MX8MP x\ne0:Fail A.a\n' > "$t/norows"
run() { python3 -I "$chk" --mustpass "$t/must" "$t/$1" 2>"$t/err" >"$t/out"; echo $?; }
expect() { # name want-rc [pattern]
  local rc; rc=$(run "$1")
  if [ "$rc" != "$2" ]; then echo "FAIL $1: rc $rc, want $2"; cat "$t/err"; fail=1; return; fi
  if [ -n "${3:-}" ] && ! grep -q -- "$3" "$t/err"; then echo "FAIL $1: stderr lacks '$3'"; cat "$t/err"; fail=1; return; fi
  echo "ok   $1"
}
expect good 0
expect good-empty 0
expect pi4 0
expect pass 1 'Pass row'
expect status 1 'unknown status'
expect cols 1 'two columns'
expect tab 1 'two columns'
expect unknown 1 'not in the mustpass'
expect unsorted 1 'out of order'
expect entryorder 1 'out of order'
expect dup 1 'duplicate'
expect nohdr 1 'first line must be'
expect rowsfirst 1 'before the'
expect norows 1 'rows:'
rc=$(python3 -I "$chk" 2>/dev/null; echo $?); [ "$rc" = 2 ] && echo "ok   usage" || { echo "FAIL usage rc $rc"; fail=1; }
[ $fail = 0 ] && echo "all ok"
exit $fail
