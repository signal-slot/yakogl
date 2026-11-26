#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-e2e-selftest.sh - the end-of-campaign chain over a synthetic complete 133-entry campaign.
#
#   sh tools/cts/imx8mp-gles3-e2e-selftest.sh
#
# Copies the files the tools read (tools/cts/imx8mp-*, check-imx8mp-gles3-tree.py, gles3-mustpass.txt,
# RESULTS.md, STATUS.md, docs/es3, docs/es3-cts-failures.md, ROADMAP.md and the three real 9bc82915 slices)
# into a git-free scratch tree under $TMPDIR, adds the 12 synthetic slices and the synthetic ES 2.0 record
# (imx8mp-gles3-synth.py), then runs merge.py --check, regenerates the not-pass list in the scratch docs,
# report.py --date 2026-11-14, redact.py --check, es2-regress.py and acceptance.sh --date 2026-11-14.
# Nothing is written into the repository. Needs TMPDIR (never /tmp), python3, git only if acceptance needs a root.
set -u
: "${TMPDIR:?set TMPDIR to a directory (the scratch tree is never written to /tmp)}"
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
T=$(mktemp -d "$TMPDIR/imx8mp-e2e.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
fails=0
fail() { echo "FAIL $1"; fails=$((fails + 1)); }
py() { python3 -I "$@"; }

mkdir -p "$T/tools/cts/log" "$T/docs" || exit 2
cd "$ROOT" || exit 2
for f in tools/cts/imx8mp-* tools/cts/check-imx8mp-gles3-tree.py tools/cts/gles3-mustpass.txt \
	tools/cts/RESULTS.md tools/cts/STATUS.md ROADMAP.md docs/es3-cts-failures.md; do
	[ -f "$f" ] && { mkdir -p "$T/$(dirname "$f")"; cp "$f" "$T/$f"; }
done
cp -r docs/es3 "$T/docs/es3" || exit 2
for d in tools/cts/log/2026-11-imx8mp-gles3-9bc82915-rows01-13 tools/cts/log/2026-11-imx8mp-gles3-9bc82915-rows17-34 \
	tools/cts/log/2026-11-imx8mp-gles3-9bc82915-rows35-51; do
	cp -r "$d" "$T/$d" || exit 2
done
cd "$T" || exit 2
py tools/cts/imx8mp-gles3-synth.py "$T" || { echo "FAIL synth"; exit 1; }
C=tools/cts
D=$(ls -d $C/log/*-imx8mp-gles3-9bc82915-rows*)
echo "slices: $(echo "$D" | wc -l)"

out=$(py $C/imx8mp-gles3-merge.py --check $D 2>&1); rc=$?
echo "$out" | tail -1
[ $rc -eq 0 ] && echo "$out" | grep -q '133 entries with a record: OK' || fail "merge --check rc=$rc"

{ head -n 2 docs/es3/imx8mp-gles3-notpass.tsv; py $C/imx8mp-gles3-merge.py --notpass $D 2>/dev/null; } >np.tsv &&
	mv np.tsv docs/es3/imx8mp-gles3-notpass.tsv || fail "notpass"

out=$(py $C/imx8mp-gles3-report.py --date 2026-11-14 $D 2>&1); rc=$?
echo "report: rc=$rc"
[ $rc -eq 0 ] && ! echo "$out" | grep -q PARTIAL || fail "report rc=$rc"

py $C/imx8mp-gles3-redact.py --check $C/log/*-imx8mp-gles3-* $C/log/*-imx8mp-es2-* >/dev/null 2>&1 || fail "redact --check"

out=$(py $C/imx8mp-es2-regress.py $C/log/2026-11-imx8mp-es2-9bc82915 2>&1); echo "$out" | tail -1
echo "$out" | grep -q '29/29 entries, 0 worse, 0 missing' || fail "es2-regress"

acc=$(bash $C/imx8mp-gles3-acceptance.sh --date 2026-11-14 "$T" 2>&1); echo "$acc"
if ! echo "$acc" | grep -q '^summary: 8 ok, 0 FAIL of 8$'; then
	# acceptance may need a git root
	git init -q "$T" >/dev/null 2>&1
	acc=$(bash $C/imx8mp-gles3-acceptance.sh --date 2026-11-14 "$T" 2>&1); echo "$acc"
	echo "$acc" | grep -q '^summary: 8 ok, 0 FAIL of 8$' || fail "acceptance"
fi
[ $fails -eq 0 ] && echo "e2e selftest: ok" || echo "e2e selftest: $fails FAIL"
[ $fails -eq 0 ]
