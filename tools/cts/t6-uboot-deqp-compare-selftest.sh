#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   bash tools/cts/t6-uboot-deqp-compare-selftest.sh
#
# Host-only check of tools/cts/t6-uboot-deqp-compare.py on synthetic inputs.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/t6-uboot-deqp-compare.py
D=$(mktemp -d "${TMPDIR:-/tmp}/t6-udc.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
chk() { l=$1; shift; if "$@"; then echo "  ok    $l"; else echo "  FAIL  $l"; fail=1; fi; }
P=dEQP-GLES2.functional
run() { python3 -I "$TOOL" "$@" 2>&1; }
has() { printf '%s\n' "$1" | grep -qx -- "$2"; }
lacks() { ! printf '%s\n' "$1" | grep -q -- "$2"; }

# U-Boot log: a Pass, b Fail, c Pass, d Pass, e Fail then Pass; noise, CR, stamp
printf '12:00:01 deqp: Pass %s.a\r\ndeqp: Fail %s.b\ndeqp: Pass %s.c\ndeqp: Pass %s.d\nnoise\ndeqp: Fail %s.e\ndeqp: Pass %s.e\n' \
  $P $P $P $P $P $P >"$D/ub.log"
# kexec statuses: a Pass, b Pass, c Fail, d Pass, f Pass
printf 'Pass %s.a\nPass %s.b\nFail %s.c\nPass %s.d\nPass %s.f\n' $P $P $P $P $P >"$D/kx.txt"
# armbian results.tsv: a Pass, b Fail, c Fail, d NotSupported, z Pass
printf 'case\tstatus\tdetail\n%s.a\tPass\tPass\n%s.b\tFail\tx\n%s.c\tFail\tx\n%s.d\tNotSupported\tx\n%s.z\tPass\tPass\n' \
  $P $P $P $P $P >"$D/ar.tsv"

out=$(run --uboot "$D/ub.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv"); rc=$?
chk "exit 0" test $rc -eq 0
chk "a same" has "$out" "$P.a uboot=Pass kexec=Pass armbian=Pass same"
chk "b newly failing (kexec passes)" has "$out" "$P.b uboot=Fail kexec=Pass armbian=Fail newly failing"
chk "c newly passing" has "$out" "$P.c uboot=Pass kexec=Fail armbian=Fail newly passing"
chk "d same across Pass/NotSupported" has "$out" "$P.d uboot=Pass kexec=Pass armbian=NotSupported same"
chk "e last status wins, no baseline: missing" has "$out" "$P.e uboot=Pass kexec=- armbian=- missing"
chk "default scope is the U-Boot run" lacks "$out" "$P.z"
chk "summary" has "$out" "t6-uboot-deqp-compare: 5 cases, same 2, newly failing 1, newly passing 1, missing 1"
out=$(run --quiet --uboot "$D/ub.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv")
chk "--quiet drops same" lacks "$out" "$P.a "
out=$(run --all --uboot "$D/ub.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv")
chk "--all: f is missing" has "$out" "$P.f uboot=- kexec=Pass armbian=- missing"
chk "--all summary" has "$out" "t6-uboot-deqp-compare: 7 cases, same 2, newly failing 1, newly passing 1, missing 3"
printf '%s.a\n%s.q\n' $P $P >"$D/scope.txt"
out=$(run --scope "$D/scope.txt" --uboot "$D/ub.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv")
chk "--scope: q missing" has "$out" "$P.q uboot=- kexec=- armbian=- missing"

# --kexec optional (no per-case kexec baseline is in the tree): without it the
# header says so (also with --quiet), the kexec column is left out, and the
# tally compares U-Boot against Armbian only.
out=$(run --uboot "$D/ub.log" --armbian "$D/ar.tsv"); rc=$?
chk "no --kexec: exit 0" test $rc -eq 0
chk "no --kexec: header" has "$out" "kexec: no per-case baseline"
chk "no --kexec: a same" has "$out" "$P.a uboot=Pass armbian=Pass same"
chk "no --kexec: b same (kexec side gone)" has "$out" "$P.b uboot=Fail armbian=Fail same"
chk "no --kexec: c newly passing" has "$out" "$P.c uboot=Pass armbian=Fail newly passing"
chk "no --kexec: e missing" has "$out" "$P.e uboot=Pass armbian=- missing"
chk "no --kexec: no kexec column" lacks "$out" "kexec="
chk "no --kexec: summary (vs Armbian only)" has "$out" \
  "t6-uboot-deqp-compare: 5 cases, same 3, newly failing 0, newly passing 1, missing 1"
out=$(run --quiet --uboot "$D/ub.log" --armbian "$D/ar.tsv"); rc=$?
chk "no --kexec --quiet: exit 0" test $rc -eq 0
chk "no --kexec --quiet: header kept" has "$out" "kexec: no per-case baseline"
chk "no --kexec --quiet: same dropped" lacks "$out" "$P.a "

# Chain: t6-uboot-rows.sh (fake prompt-go that makes a record dir like the real
# one: image.log with the deqp lines, plus uart.log/typed.log/verdict.txt)
# -> its rows.tsv / out-dir / one record dir as --uboot.
RT=$D/rt; mkdir -p "$RT/img/R1/zephyr" "$RT/img/R2/zephyr" "$RT/img/R3/zephyr" "$RT/recs"
printf '# image R1: 1\n# image R2: 2\n# image R3: 3\n' >"$RT/tree.txt"
for l in R1 R2 R3; do echo $l >"$RT/img/$l/zephyr/zephyr.bin"; done
cat >"$RT/fake.sh" <<'FAKE'
#!/bin/sh
l=$(cat "$1"); rec=$RECS/$l-t6-uboot-go; mkdir -p "$rec"
echo "prompt-go.sh: record $rec"
printf 'deqp: Pass dEQP-GLES2.%s.ok\r\ndeqp: Fail dEQP-GLES2.%s.bad\ndeqp: summary: 2 cases begun, 2 with a result, 0 without\n' $l $l >"$rec/image.log"
echo "typed noise" >"$rec/typed.log"
printf 'deqp: Pass dEQP-GLES2.%s.uartonly\n' $l >"$rec/uart.log"
echo "VERDICT deqp PASS" >"$rec/verdict.txt"
[ "$l" = R3 ] && exit 10
[ "$l" = R2 ] && exit 11
exit 0
FAKE
chmod +x "$RT/fake.sh"
PROMPT_GO=$RT/fake.sh TREE=$RT/tree.txt RECS=$RT/recs bash "$HERE/t6-uboot-rows.sh" "$RT/img" "$RT/out" R1 R2 R3 --go >/dev/null 2>&1
chk "rows.sh stopped at R3 (exit 10 row recorded)" test "$(cut -f2 "$RT/out/rows.tsv" | tr '\n' ' ')" = "0 11 10 "
printf 'Pass dEQP-GLES2.R1.ok\nPass dEQP-GLES2.R1.bad\nPass dEQP-GLES2.R2.ok\nFail dEQP-GLES2.R2.bad\nPass dEQP-GLES2.R3.ok\nPass dEQP-GLES2.R3.bad\n' >"$D/kx2.txt"
for u in "$RT/out/rows.tsv" "$RT/out"; do
	out=$(run --uboot "$u" --kexec "$D/kx2.txt" --armbian "$D/kx2.txt"); rc=$?
	chk "chain ($(basename "$u")): exit 0" test $rc -eq 0
	chk "chain: R1.ok same" has "$out" "dEQP-GLES2.R1.ok uboot=Pass kexec=Pass armbian=Pass same"
	chk "chain: R1.bad newly failing" has "$out" "dEQP-GLES2.R1.bad uboot=Fail kexec=Pass armbian=Pass newly failing"
	chk "chain: R2.bad same (both fail)" has "$out" "dEQP-GLES2.R2.bad uboot=Fail kexec=Fail armbian=Fail same"
	chk "chain: stopped row's image.log is read" has "$out" "dEQP-GLES2.R3.ok uboot=Pass kexec=Pass armbian=Pass same"
	chk "chain: uart.log is not read" lacks "$out" "uartonly"
	chk "chain summary" has "$out" "t6-uboot-deqp-compare: 6 cases, same 4, newly failing 2, newly passing 0, missing 0"
done
out=$(run --uboot "$RT/recs/R2-t6-uboot-go" --kexec "$D/kx2.txt" --armbian "$D/kx2.txt")
chk "one record dir reads image.log only" has "$out" "t6-uboot-deqp-compare: 2 cases, same 2, newly failing 0, newly passing 0, missing 0"
rm "$RT/recs/R1-t6-uboot-go/image.log"
run --uboot "$RT/out" --kexec "$D/kx2.txt" --armbian "$D/kx2.txt" >/dev/null
chk "done row without image.log: exit 2" test $? -eq 2

# bad input exits 2
run --uboot "$D/nope" --kexec "$D/kx.txt" --armbian "$D/ar.tsv" >/dev/null
chk "unreadable file: exit 2" test $? -eq 2
echo noise >"$D/empty.log"
run --uboot "$D/empty.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv" >/dev/null
chk "no outcome line: exit 2" test $? -eq 2
# An explicit --kexec file that is missing or has no outcome line is an error
# with its own exit code 3 (the same files are exit 2 on the --uboot side):
# no baseline is said by omitting --kexec, not by a bad file.
run --uboot "$D/ub.log" --kexec "$D/nope" --armbian "$D/ar.tsv" >/dev/null
chk "explicit missing kexec file: exit 3" test $? -eq 3
run --uboot "$D/ub.log" --kexec "$D/empty.log" --armbian "$D/ar.tsv" >/dev/null
chk "explicit kexec file with no outcome line: exit 3" test $? -eq 3
run --kexec "$D/kx.txt" >/dev/null
chk "usage error: exit 2" test $? -eq 2

[ $fail -eq 0 ] && echo "t6-uboot-deqp-compare-selftest: ok"
exit $fail
