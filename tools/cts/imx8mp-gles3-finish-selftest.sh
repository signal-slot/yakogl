#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-finish-selftest.sh - runs imx8mp-gles3-finish.sh (no --apply, --partial: the three recorded slices
# are 48 of 133 entries) into $TMPDIR and checks that its notpass.tsv equals docs/es3/imx8mp-gles3-notpass.tsv and its
# compare.tsv equals docs/es3/imx8mp-gles3-compare-9bc82915.tsv byte for byte, and that no committed file changed.
#
#   TMPDIR=<dir> sh tools/cts/imx8mp-gles3-finish-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
: "${TMPDIR:?set TMPDIR to a directory (never /tmp)}"
[ -d "$TMPDIR" ] || { echo "TMPDIR is not a directory" >&2; exit 2; }
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
O=$(mktemp -d "$TMPDIR/finish-selftest.XXXXXX") || exit 2
trap 'rm -rf "$O"' EXIT
fail=0
ck() { if cmp -s "$2" "$3"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
B=$(cd "$ROOT" && git status --porcelain docs 2>/dev/null | md5sum)
sh "$HERE/imx8mp-gles3-finish.sh" --partial "$O/out" >"$O/summary.txt" || { cat "$O/summary.txt"; echo "FAIL finish.sh exit"; exit 1; }
cat "$O/summary.txt"
[ "$(grep -c '^[1-6] ' "$O/summary.txt")" -eq 6 ] && echo "ok   six summary lines" || { echo "FAIL summary lines"; fail=1; }
ck "notpass.tsv == docs/es3/imx8mp-gles3-notpass.tsv" "$O/out/notpass.tsv" "$ROOT/docs/es3/imx8mp-gles3-notpass.tsv"
ck "compare.tsv == docs/es3/imx8mp-gles3-compare-9bc82915.tsv" "$O/out/compare.tsv" "$ROOT/docs/es3/imx8mp-gles3-compare-9bc82915.tsv"
# without --partial the three slices are an incomplete campaign: the chain stops at step 1
if sh "$HERE/imx8mp-gles3-finish.sh" "$O/strict" >"$O/strict.txt" 2>&1; then echo "FAIL strict run did not stop"; fail=1; else echo "ok   incomplete campaign stops without --partial"; fi
A=$(cd "$ROOT" && git status --porcelain docs 2>/dev/null | md5sum)
[ "$A" = "$B" ] && echo "ok   no committed file changed" || { echo "FAIL docs changed"; fail=1; }
[ $fail -eq 0 ] && echo "imx8mp-gles3-finish-selftest: ok" || echo "imx8mp-gles3-finish-selftest: FAILED"
exit $fail
