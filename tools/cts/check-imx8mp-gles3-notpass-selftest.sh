#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of check-imx8mp-gles3-notpass.py: one good and several bad fixtures under $TMPDIR.
set -u
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/check-imx8mp-gles3-notpass.py"
t=$(mktemp -d "${TMPDIR:-/tmp}/notpass-selftest.XXXXXX") || exit 2
trap 'rm -rf "$t"' EXIT
mp=$t/mustpass.txt
printf 'dEQP-GLES3.a.one\ndEQP-GLES3.a.two\ndEQP-GLES3.b.three\n' > "$mp"
H1='# i.MX8MP dEQP-GLES3 at 0123456789abcdef0123456789abcdef01234567: every case that is not Pass'
H2='# rows: e<i>:<status> <case>, in entry then mustpass order'
fail=0
mk() { printf '%s\n' "$@" > "$t/f.tsv"; }
expect() { # name want-rc ; fixture lines follow in $t/f.tsv
  python3 "$chk" --mustpass "$mp" "$t/f.tsv" > "$t/out" 2>&1; rc=$?
  if { [ "$2" = 0 ] && [ $rc -ne 0 ]; } || { [ "$2" = 1 ] && { [ $rc -eq 0 ] || [ ! -s "$t/out" ]; }; }; then
    echo "FAIL: $1 (rc=$rc)"; cat "$t/out"; fail=1
  else echo "ok: $1"; fi
}
mk "$H1" "$H2" '# entries run (1 of 1): 0' 'e0:Fail dEQP-GLES3.a.one' 'e0:NotSupported dEQP-GLES3.a.two' 'e1:Fail dEQP-GLES3.b.three'
expect good 0
mk "$H1" "$H2"; expect 'header only' 0
mk "$H2" 'e0:Fail dEQP-GLES3.a.one'; expect 'no header' 1
mk "$H1" 'e0:Fail dEQP-GLES3.a.one'; expect 'no rows line' 1
mk "$H1" "$H2" 'e0:Pass dEQP-GLES3.a.one'; expect 'Pass row' 1
mk "$H1" "$H2" 'e0:Bogus dEQP-GLES3.a.one'; expect 'unknown status' 1
mk "$H1" "$H2" 'e0:Fail dEQP-GLES3.zzz'; expect 'case not in mustpass' 1
mk "$H1" "$H2" 'e0:Fail dEQP-GLES3.a.one' 'e0:Fail dEQP-GLES3.a.one'; expect duplicate 1
mk "$H1" "$H2" 'e0:Fail dEQP-GLES3.a.two' 'e0:Fail dEQP-GLES3.a.one'; expect 'unsorted in entry' 1
mk "$H1" "$H2" 'e1:Fail dEQP-GLES3.a.one' 'e0:Fail dEQP-GLES3.a.two'; expect 'unsorted entries' 1
mk "$H1" "$H2" 'e0 Fail dEQP-GLES3.a.one'; expect 'bad columns' 1
mk "$H1" "$H2" 'e0:Fail dEQP-GLES3.a.one extra'; expect 'too many columns' 1
exit $fail
