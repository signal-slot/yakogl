#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of glmark2-one-commit-acceptance.sh on fake trees built from
# testdata/glmark2-one-commit-acceptance/: pass/ passes all six checks, and
# each fail-<cN>/ overlay (one file) makes exactly check cN fail. Uses
# --skip-website and --ref none (a fake tree has no website or git history).
# Needs python3 only; stops at the first wrong verdict.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ACC=$HERE/glmark2-one-commit-acceptance.sh
FX=$HERE/testdata/glmark2-one-commit-acceptance
T=$(mktemp -d "${TMPDIR:-/tmp}/glmark2-one-commit-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT

die() { echo "FAIL: $*"; exit 1; }

run() { bash "$ACC" --root "$1" --ref none --skip-website 2>/dev/null; }

cp -r "$FX/pass" "$T/pass"
out=$(run "$T/pass"); rc=$?
[ $rc -eq 0 ] || { printf '%s\n' "$out"; die "pass tree: exit $rc, want 0"; }
[ "$(printf '%s\n' "$out" | grep -c ' PASS ')" -eq 6 ] || { printf '%s\n' "$out"; die "pass tree: want six PASS lines"; }
echo "ok: pass tree, six PASS lines"

for c in c1 c2 c3 c4 c5 c6 c3b; do
	t=$T/$c
	cp -r "$FX/pass" "$t"
	cp -r "$FX/fail-$c/." "$t/"
	out=$(run "$t"); rc=$?
	id=${c%b}
	[ $rc -eq 1 ] || { printf '%s\n' "$out"; die "$c: exit $rc, want 1"; }
	printf '%s\n' "$out" | grep -q "^$id FAIL " || { printf '%s\n' "$out"; die "$c: no '$id FAIL' line"; }
	n=$(printf '%s\n' "$out" | grep -c ' FAIL ')
	[ "$n" -eq 1 ] || { printf '%s\n' "$out"; die "$c: $n FAIL lines, want 1"; }
	echo "ok: $c fails $id alone"
done
echo "glmark2-one-commit-acceptance selftest: ok"
