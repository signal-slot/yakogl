#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of glmark2-pictures-column.py on a copy of the real table under $TMPDIR with a
# synthetic console (glmark2-picture: lines in the format common-tally.py parses).
# Host only; no board. Exit 0 when every check passes.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/glmark2-pictures-column.py
table=$here/glmark2-pictures.txt
work=$(mktemp -d "${TMPDIR:-/var/tmp}/pcol.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
py() { python3 -I "$tool" "$@"; }

cp "$table" "$work/t.txt"
# the console: each scene of the am62p column as a glmark2-picture: line
awk -F' [|] ' '$1=="am62p"{print "12:00:00 glmark2-picture: " $2 ": " $3}' "$table" > "$work/console.txt"
n=$(wc -l < "$work/console.txt")
if [ "$n" = 33 ]; then ok "console has 33 scenes"; else bad "console has $n scenes"; fi

hdr="looked at by eye, commit 0123abcd, records/imx8mp/test-run"
common=(--commit 0123abcd --record records/imx8mp/test-run)

if py "$work/t.txt" imx8mp "$work/console.txt" "${common[@]}" --header-text "$hdr" \
      --out "$work/new.txt" 2> "$work/err"; then ok "replacement runs"
else bad "replacement runs"; cat "$work/err"; fi

# other lines byte for byte: drop the imx8mp header and lines from both and compare
strip() { grep -v -e '^# imx8mp:' -e '^imx8mp | ' "$1"; }
strip "$work/t.txt" > "$work/a"; strip "$work/new.txt" > "$work/b"
if cmp -s "$work/a" "$work/b"; then ok "other lines identical"; else bad "other lines differ"; fi
grep -n -e '^# imx8mp:' -e '^imx8mp | ' "$work/t.txt" | cut -d: -f1 > "$work/l1"
grep -n -e '^# imx8mp:' -e '^imx8mp | ' "$work/new.txt" | cut -d: -f1 > "$work/l2"
if cmp -s "$work/l1" "$work/l2"; then ok "column kept its place"; else bad "column moved"; fi
awk -F' [|] ' '$1=="imx8mp"{print $2 "|" $3}' "$work/new.txt" > "$work/v1"
awk -F' [|] ' '$1=="am62p"{print $2 "|" $3}' "$table" > "$work/v2"
if cmp -s "$work/v1" "$work/v2"; then ok "new values are the console's"; else bad "new values differ from the console"; fi
if grep -q "^# imx8mp: $hdr\$" "$work/new.txt"; then ok "header replaced"; else bad "header not replaced"; fi
if py "$work/new.txt" imx8mp --check > /dev/null 2>&1; then ok "--check on the new table"; else bad "--check on the new table"; fi
if py "$table" imx8mp --check > /dev/null 2>&1; then ok "--check on the real table"; else bad "--check on the real table"; fi

# --in-place gives the same bytes as --out
cp "$work/t.txt" "$work/ip.txt"
py "$work/ip.txt" imx8mp "$work/console.txt" "${common[@]}" --header-text "$hdr" --in-place 2> /dev/null
if cmp -s "$work/ip.txt" "$work/new.txt"; then ok "--in-place equals --out"; else bad "--in-place differs"; fi

# a run directory source (a board leaves <run dir>/console.txt; common-tally.py reads it)
mkdir -p "$work/run"
cp "$work/console.txt" "$work/run/console.txt"
py "$work/t.txt" imx8mp "$work/run" "${common[@]}" --header-text "$hdr" --out "$work/rd.txt" 2> /dev/null
if cmp -s "$work/rd.txt" "$work/new.txt"; then ok "run dir source equals console file"
else bad "run dir source differs"; fi

refused() { # name, args...: exit 1 and "refused" on stderr
    local name=$1 rc; shift
    py "$@" > /dev/null 2> "$work/err"; rc=$?
    if [ "$rc" = 1 ] && grep -q refused "$work/err"; then ok "refused: $name"
    else bad "refused: $name (exit $rc)"; fi
}

sed 1d "$work/console.txt" > "$work/missing.txt"
refused "a missing scene" "$work/t.txt" imx8mp "$work/missing.txt" "${common[@]}" --header-text "$hdr"
# common-tally.py folds a repeated scene line into one, so a duplicate can only reach the
# tool's own check: call it directly
if python3 -I -c 'import importlib.util as u, sys
s = u.spec_from_file_location("c", sys.argv[1]); m = u.module_from_spec(s); s.loader.exec_module(m)
try:
    m.check_scenes(["a", "b", "a"], ["a", "b", "c"])
except m.Refused as e:
    sys.exit(0 if "duplicate" in str(e) else 1)
sys.exit(1)' "$tool"; then ok "refused: a duplicate scene"; else bad "refused: a duplicate scene"; fi
refused "an IP in the header" "$work/t.txt" imx8mp "$work/console.txt" "${common[@]}" \
    --header-text "$hdr, board 192.168.1.20"
refused "a /dev/tty path in the header" "$work/t.txt" imx8mp "$work/console.txt" "${common[@]}" \
    --header-text "$hdr, /dev/ttyUSB0"
refused "a header without the commit" "$work/t.txt" imx8mp "$work/console.txt" "${common[@]}" \
    --header-text "looked at by eye, records/imx8mp/test-run"
refused "a header without the record" "$work/t.txt" imx8mp "$work/console.txt" "${common[@]}" \
    --header-text "looked at by eye, commit 0123abcd"
refused "a short commit" "$work/t.txt" imx8mp "$work/console.txt" --commit 0123ab \
    --record records/imx8mp/test-run --header-text "commit 0123ab records/imx8mp/test-run"
# a table whose column has a /dev/tty line fails --check
awk -F' [|] ' 'BEGIN{OFS=" | "} $1=="imx8mp" && !d {$3=$3 " /dev/ttyS0"; d=1} {print}' "$table" > "$work/leak.txt"
refused "--check on a leaking table" "$work/leak.txt" imx8mp --check
if cmp -s "$table" "$work/t.txt"; then ok "real table untouched"; else bad "real table changed"; fi

if [ "$fail" = 0 ]; then echo "glmark2-pictures-column-selftest: all ok"; else echo "glmark2-pictures-column-selftest: FAILED"; fi
exit "$fail"
