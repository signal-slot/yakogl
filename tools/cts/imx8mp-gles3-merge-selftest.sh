#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-merge-selftest.sh - the synthetic-record selftest of imx8mp-gles3-merge.py (its --selftest):
# disjoint slices, board-form slices with '-' rows (M1), missing entries and --partial (M2), clean and
# non-clean README verdict lines (M3), Armadillo-X2 three-word rows with begun and noresult "-" (w3 not stored).
# Then the CLI cases for Armadillo-X2 slice directory names (x2-<sha8>) and mixes with Verdin 9bc82915 slices.
# Records are written under $TMPDIR (never /tmp) and removed.
#
#   TMPDIR=<dir> tools/cts/imx8mp-gles3-merge-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory (synthetic records are never written to /tmp)}"
[ -d "$TMPDIR" ] || { echo "TMPDIR is not a directory" >&2; exit 2; }
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
MERGE="$HERE/imx8mp-gles3-merge.py"
python3 "$MERGE" --selftest || exit 1

ROOT=$(mktemp -d "$TMPDIR/imx8mp-merge-x2-XXXXXX") || exit 2
trap 'rm -rf "$ROOT"' EXIT
FAILS=0
expect() { if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; FAILS=$((FAILS + 1)); fi; }

# Synthetic table (3 entries) and mustpass, as the module selftest builds them.
TABLE=$ROOT/table.txt MUST=$ROOT/mustpass.txt
printf '# synthetic\ndEQP-GLES3.a.*\ndEQP-GLES3.b.*\ndEQP-GLES3.c.*\n' > "$TABLE"
printf '# synthetic\ndEQP-GLES3.a.0\ndEQP-GLES3.a.1\ndEQP-GLES3.b.0\ndEQP-GLES3.b.1\ndEQP-GLES3.c.0\n' > "$MUST"
COMMIT=c29a5ffe1111111111111111111111111111111a
OLD=9bc829150000000000000000000000000000000b

# slice DIR COMMIT BOARD ENTRY...: one README verdict line and one results.tsv row per entry (X2 rows are
# board.sh form: begun/passed numeric here, as a finished entry). Entry 1 has one named failure.
slice() {
    local dir=$1 commit=$2 board=$3; shift 3
    mkdir -p "$dir"
    printf '# slice at origin/main %s\n' "$commit" > "$dir/README.md"
    printf '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names\n' > "$dir/results.tsv"
    local i
    for i in "$@"; do
        printf 'board.sh %s e%02d rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=0 record=x back_after=1s\n' \
            "$board" "$i" >> "$dir/README.md"
        case $i in
        0) printf '0\tdEQP-GLES3.a.*\t2\t2\t0\t0\t0\t0\t-\n' >> "$dir/results.tsv" ;;
        1) printf '1\tdEQP-GLES3.b.*\t2\t1\t1\t0\t0\t0\tdEQP-GLES3.b.1\n' >> "$dir/results.tsv" ;;
        2) printf '2\tdEQP-GLES3.c.*\t1\t1\t0\t0\t0\t0\t-\n' >> "$dir/results.tsv" ;;
        esac
    done
}
merge() { python3 "$MERGE" --table "$TABLE" --mustpass "$MUST" "$@"; }

L=$ROOT/log; mkdir -p "$L/x2" "$L/verdin" "$L/mix"
slice "$L/x2/2026-11-imx8mp-gles3-x2-c29a5ffe-rows01-02" "$COMMIT" x2 0 1
slice "$L/x2/2026-11-imx8mp-gles3-x2-c29a5ffe-rows03-03" "$COMMIT" x2 2
slice "$L/verdin/2026-11-imx8mp-gles3-9bc82915-rows01-02" "$OLD" imx8mp 0 1
slice "$L/verdin/2026-11-imx8mp-gles3-9bc82915-rows03-03" "$OLD" imx8mp 2

# (a) a full X2 set of one commit: passes, one commit (no 'different commits').
out=$(merge --check "$L"/x2/*-imx8mp-gles3-*/); rc=$?
expect "(a) x2-<sha8> slices of one commit: --check exits 0" $rc
case $out in *'different commits'*|*FAILED*) expect "(a) reports one commit" 1 ;; *) expect "(a) reports one commit" 0 ;; esac
case $out in *'measured on the Armadillo'*) expect "(a) labels the board X2" 0 ;; *) expect "(a) labels the board X2" 1 ;; esac
case $out in *'3 entries with a record: OK'*) expect "(a) reports all entries (3)" 0 ;; *) expect "(a) reports all entries (3)" 1; echo "$out" ;; esac

# (b) X2 set plus a Verdin 9bc82915 slice: refused, naming different commits.
cp -r "$L/verdin/2026-11-imx8mp-gles3-9bc82915-rows01-02" "$L/x2/2026-11-imx8mp-gles3-x2-c29a5ffe-rows03-03" "$L/mix/"
out=$(merge --check "$L"/mix/*-imx8mp-gles3-*/); rc=$?
expect "(b) x2 slice plus a Verdin 9bc82915 slice: --check exits 1" $((rc == 1 ? 0 : 1))
case $out in *'different commits'*) expect "(b) names different commits" 0 ;; *) expect "(b) names different commits" 1; echo "$out" ;; esac
case $out in *x2-c29a5ffe*|*9bc82915*) expect "(b) names a slice directory" 0 ;; *) expect "(b) names a slice directory" 1 ;; esac

# (c) the same records under X2-style and Verdin-style names: identical --notpass lines. The record
# board.sh lines say x2 in one set and imx8mp in the other; the directory name must not change the case list.
mkdir -p "$L/cx" "$L/cv"
slice "$L/cx/2026-11-imx8mp-gles3-x2-c29a5ffe-rows01-02" "$COMMIT" x2 0 1
slice "$L/cx/2026-11-imx8mp-gles3-x2-c29a5ffe-rows03-03" "$COMMIT" x2 2
slice "$L/cv/2026-11-imx8mp-gles3-c29a5ffe-rows01-02" "$COMMIT" imx8mp 0 1
slice "$L/cv/2026-11-imx8mp-gles3-c29a5ffe-rows03-03" "$COMMIT" imx8mp 2
nx=$(merge --notpass "$L"/cx/*/ 2>&1); nv=$(merge --notpass "$L"/cv/*/ 2>&1)
# --notpass labels X2 records with a "# measured on the Armadillo-X2" comment (the owner's rule); the case
# lines (everything else) must be identical.
[ "$(echo "$nx" | grep -v '^# measured on the Armadillo-X2')" = "$nv" ]; expect "(c) --notpass of x2 slices equals Verdin slices apart from the board label" $?
case $nx in *'# measured on the Armadillo-X2'*) expect "(c) x2 --notpass carries the board label" 0 ;; *) expect "(c) x2 --notpass carries the board label" 1 ;; esac
case $nv in *'Armadillo'*) expect "(c) Verdin --notpass carries no X2 label" 1 ;; *) expect "(c) Verdin --notpass carries no X2 label" 0 ;; esac
case $nx in *'e1:Fail dEQP-GLES3.b.1'*) expect "(c) the output names the failing case" 0 ;; *) expect "(c) the output names the failing case" 1 ;; esac

echo "merge-selftest x2 names: $([ $FAILS = 0 ] && echo OK || echo "FAILED ($FAILS)")"
[ $FAILS = 0 ]
