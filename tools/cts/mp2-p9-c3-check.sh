#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2-p9-c3-check.sh [--notes FILE] [--diff-opt OPT]... [LOG...]
#
# Acceptance c3 of the DK's P9 (docs/notes/stm32mp257f-dk-p9.md): the shard
# logs have a result for every listed case, and mp2-p9-diff over them gives the
# summary line the notes quote. Runs
#   python3 -I tools/cts/mp2-p9-diff.py --report-only <OPT>... LOG...
# takes its last line and compares it exactly with the last 'mp2-p9-diff: ...'
# line quoted in the notes (the latest results section; default
# docs/notes/stm32mp257f-dk-p9.md). When the diff reports missing > 0, every
# missing name must appear in the notes file (listed there with its cause).
# LOG defaults to tools/cts/log/*-mp2-gles2-*/s*/shard-*.log. --diff-opt passes
# one argument through to the diff (--baseline FILE needs two --diff-opt).
#
# Last line: 'mp2-p9-c3-check: ok' or 'mp2-p9-c3-check: FAIL <reason>'.
# Exit 0 ok, 1 FAIL, 2 usage error or no logs match.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
NOTES=$ROOT/docs/notes/stm32mp257f-dk-p9.md
opts=()
logs=()
usage() { echo "mp2-p9-c3-check: $1" >&2; exit 2; }
while [ $# -gt 0 ]; do
	case $1 in
	--notes) [ $# -ge 2 ] || usage "--notes needs a file"
		NOTES=$2; shift 2 ;;
	--diff-opt) [ $# -ge 2 ] || usage "--diff-opt needs an argument"
		opts+=("$2"); shift 2 ;;
	-h|--help) sed -n 4,19p "$0"; exit 0 ;;
	-*) usage "unknown option $1" ;;
	*) logs+=("$1"); shift ;;
	esac
done
if [ ${#logs[@]} -eq 0 ]; then
	shopt -s nullglob
	logs=("$HERE"/log/*-mp2-gles2-*/s*/shard-*.log)
	shopt -u nullglob
fi
real=()
for f in "${logs[@]}"; do
	[ -f "$f" ] && real+=("$f")
done
[ ${#real[@]} -gt 0 ] || usage "no shard logs match (default tools/cts/log/*-mp2-gles2-*/s*/shard-*.log)"
[ -f "$NOTES" ] || usage "notes file $NOTES not found"

fail() { echo "mp2-p9-c3-check: FAIL $1"; exit 1; }

out=$(python3 -I "$HERE/mp2-p9-diff.py" --report-only "${opts[@]}" "${real[@]}" 2>&1)
rc=$?
if [ $rc -ne 0 ]; then
	printf '%s\n' "$out" | tail -3 >&2
	[ $rc -eq 2 ] && usage "the diff could not run"
	fail "mp2-p9-diff exited $rc"
fi
got=$(printf '%s\n' "$out" | tail -1)
case $got in
'mp2-p9-diff: shared '*) ;;
*) fail "unexpected last diff line: $got" ;;
esac

quoted=$(grep -o 'mp2-p9-diff: shared [0-9]*, new [0-9]*, fixed [0-9]*, missing [0-9]*' "$NOTES" | tail -1)
[ -n "$quoted" ] || fail "no 'mp2-p9-diff: shared ...' line quoted in $NOTES"
echo "logs:   ${#real[@]} file(s)"
echo "diff:   $got"
echo "quoted: $quoted"
[ "$got" = "$quoted" ] || fail "diff line differs from the one in the notes"

missing=${got##*missing }
if [ "$missing" -gt 0 ]; then
	names=$(printf '%s\n' "$out" | awk '/^== /{s=($0 ~ /^== mustpass names missing/)} s && !/^==/ && !/^mp2-p9-diff:/ && NF')
	n=0
	unlisted=()
	while IFS= read -r name; do
		[ -n "$name" ] || continue
		n=$((n + 1))
		grep -qF -- "$name" "$NOTES" || unlisted+=("$name")
	done <<<"$names"
	[ "$n" -eq "$missing" ] || fail "diff says missing $missing but lists $n names"
	if [ ${#unlisted[@]} -gt 0 ]; then
		printf 'not in the notes: %s\n' "${unlisted[@]}"
		fail "${#unlisted[@]} of $missing missing case(s) not listed in the notes"
	fi
	echo "missing: $missing, all listed in the notes"
fi
echo "mp2-p9-c3-check: ok"
