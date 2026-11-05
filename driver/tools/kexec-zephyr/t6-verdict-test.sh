#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6-verdict-test.sh - t6-verdict.sh over the fixtures in testdata/ (*.uart:
# driver/.gitignore ignores *.log): each case's exit code, verdict line and
# per-line statuses. Host only.
#
#   bash driver/tools/kexec-zephyr/t6-verdict-test.sh
#
# The fixtures are the predicted records (driver/docs/nanopc-t6-mali.md:
# Z1's measured t6_hello and t6_gpu lines, Z2's "The expected uart2 record" with
# P = 0x10027000 and B = 0x10037000), not board output.
set -euo pipefail
export LC_ALL=C

HERE=$(cd "$(dirname "$0")" && pwd)
V=$HERE/t6-verdict.sh
D=$HERE/testdata
WORK=$(mktemp -d "${TMPDIR:-$HERE}/t6-verdict-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

fails=0 runs=0

# check <name> <want rc> <want verdict line> <want "ok missing differs"> <probe> <log> [env...]
check() {
	local name=$1 want_rc=$2 want_v=$3 want_n=$4 probe=$5 log=$6
	shift 6
	local out rc=0 got_v got_n
	out=$(env "$@" bash "$V" "$probe" "$log" 2>&1) || rc=$?
	got_v=$(printf '%s\n' "$out" | tail -n 1)
	got_n="$(printf '%s\n' "$out" | grep -c '^ok ' || true) $(printf '%s\n' "$out" | grep -c '^missing ' || true) $(printf '%s\n' "$out" | grep -c '^differs ' || true)"
	runs=$((runs + 1))
	if [ "$rc" = "$want_rc" ] && [ "$got_v" = "$want_v" ] && [ "$got_n" = "$want_n" ]; then
		echo "ok   $name: rc $rc, $got_v, ok/missing/differs $got_n"
	else
		echo "FAIL $name: rc $rc (want $want_rc), '$got_v' (want '$want_v'), ok/missing/differs $got_n (want $want_n)"
		printf '%s\n' "$out" | grep -v '^ok ' | sed 's/^/     /'
		fails=$((fails + 1))
	fi
}

# the line counts: boot 10 + reset 1 + each probe's own
MMU=27 HELLO=25 GPU=31

check t6_mmu-pass 0 'VERDICT t6_mmu PASS' "$MMU 0 0" t6_mmu "$D/t6_mmu-pass.uart"
check t6_mmu-missing 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 1 0" t6_mmu "$D/t6_mmu-missing.uart"
check t6_mmu-differs 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 0 1" t6_mmu "$D/t6_mmu-differs.uart"

# the UART's CR LF, and NULs around the jump
sed 's/$/\r/' "$D/t6_mmu-pass.uart" | sed '1s/^/\x00\x00/' >"$WORK/crlf.log"
check t6_mmu-crlf 0 'VERDICT t6_mmu PASS' "$MMU 0 0" t6_mmu "$WORK/crlf.log"

# the PAs from nm (with and without 0x): equal passes, another differs on
# the pool line and every line derived from P
check t6_mmu-nm 0 'VERDICT t6_mmu PASS' "$MMU 0 0" t6_mmu "$D/t6_mmu-pass.uart" \
	T6_POOL_PA=0000000010027000 T6_BACKING_PA=0x10037000
check t6_mmu-nm-other 1 "VERDICT t6_mmu FAIL (6 of $MMU)" "$((MMU - 6)) 0 6" t6_mmu "$D/t6_mmu-pass.uart" \
	T6_POOL_PA=0x10017000 T6_BACKING_PA=0x10037000

# the identity map: a pool %p that is not its PA differs on that line only
sed 's/^t6_mmu: pool 0x10027000 /t6_mmu: pool 0xffff000010027000 /' "$D/t6_mmu-pass.uart" >"$WORK/va.log"
check t6_mmu-pool-va 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 0 1" t6_mmu "$WORK/va.log"

# FAULTADDRESS as found is predicted 0, not recorded
sed '/AS0 as found/s/FAULTADDRESS 0x0000000000000000/FAULTADDRESS 0x0000000004001000/' "$D/t6_mmu-pass.uart" >"$WORK/fa.log"
check t6_mmu-faultaddress 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 0 1" t6_mmu "$WORK/fa.log"

# a FAIL line from the probe is a differing PASS line, the lines it skips
# missing
sed -e '/the flush after the map/d' \
	-e 's/^t6_mmu: PASS: .*/t6_mmu: FAIL: stop 9 (the flush after the map stopped), refusal 3, detail 0x0000000000000304/' \
	"$D/t6_mmu-pass.uart" >"$WORK/fail.log"
check t6_mmu-probe-fail 1 "VERDICT t6_mmu FAIL (2 of $MMU)" "$((MMU - 2)) 1 1" t6_mmu "$WORK/fail.log"

# an empty record (the first Z1 jump's 0-byte uart.log)
: >"$WORK/empty.log"
check t6_mmu-empty 1 "VERDICT t6_mmu FAIL ($MMU of $MMU)" "0 $MMU 0" t6_mmu "$WORK/empty.log"

# another probe's record
check t6_gpu-over-mmu 1 "VERDICT t6_gpu FAIL ($((GPU - 10)) of $GPU)" "10 $((GPU - 10)) 0" t6_gpu "$D/t6_mmu-pass.uart"

check t6_hello-pass 0 'VERDICT t6_hello PASS' "$HELLO 0 0" t6_hello "$D/t6_hello-pass.uart"
check t6_gpu-pass 0 'VERDICT t6_gpu PASS' "$GPU 0 0" t6_gpu "$D/t6_gpu-pass.uart"

# t6_fw: boot 10 + build, WDT0, image, 7 sections, pool, Z3's rc line and 7
# record lines, PASS, reset = 31
FW=31
check t6_fw-pass 0 'VERDICT t6_fw PASS' "$FW 0 0" t6_fw "$D/t6_fw-pass.uart"
check t6_fw-missing 1 "VERDICT t6_fw FAIL (1 of $FW)" "$((FW - 1)) 1 0" t6_fw "$D/t6_fw-missing.uart"
# Z1 refused (step 3): the rc line differs, so does the stop line, the FAIL
# line stands for PASS (differs, it names the step) and the record never came
check t6_fw-z1fail 1 "VERDICT t6_fw FAIL (10 of $FW)" "$((FW - 9)) 7 3" t6_fw "$D/t6_fw-z1fail.uart"
# stops at boot with no GLB version (step 5, VersionZero): the rc line, the
# version, the interface and the configuration lines differ or are missing
check t6_fw-boot-stop 1 "VERDICT t6_fw FAIL (7 of $FW)" "$((FW - 6)) 2 5" t6_fw "$D/t6_fw-boot-stop.uart"
# the PING not acked (step 5, PingTimeout): the rc line, the last record line
# (acked 0xe, NOT acked) and the FAIL line differ, with the stop line
check t6_fw-ping-lost 1 "VERDICT t6_fw FAIL (4 of $FW)" "$((FW - 3)) 0 4" t6_fw "$D/t6_fw-ping-lost.uart"
# JOB_INT_RAWSTAT without bit 31 differs (hook)
sed 's/JOB_INT_RAWSTAT 0x80000000/JOB_INT_RAWSTAT 0x00000000/' "$D/t6_fw-pass.uart" >"$WORK/job.log"
check t6_fw-job-bit 1 "VERDICT t6_fw FAIL (1 of $FW)" "$((FW - 1)) 0 1" t6_fw "$WORK/job.log"
# GLB version 1.4.0 differs on that line only
sed 's/GLB version 1.5.0 (0x01050000)/GLB version 1.4.0 (0x01040000)/' "$D/t6_fw-pass.uart" >"$WORK/ver.log"
check t6_fw-version 1 "VERDICT t6_fw FAIL (1 of $FW)" "$((FW - 1)) 0 1" t6_fw "$WORK/ver.log"

# usage
rc=0
bash "$V" t6_nonesuch "$D/t6_mmu-pass.uart" >/dev/null 2>&1 || rc=$?
runs=$((runs + 1))
if [ "$rc" = 2 ]; then echo "ok   unknown probe: rc 2"; else echo "FAIL unknown probe: rc $rc (want 2)"; fails=$((fails + 1)); fi

if [ "$fails" -eq 0 ]; then
	echo "t6-verdict-test: $runs ok"
	exit 0
fi
echo "t6-verdict-test: $fails of $runs FAILED"
exit 1
