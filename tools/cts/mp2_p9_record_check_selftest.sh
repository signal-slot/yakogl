#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_p9_record_check_selftest.sh
#
# Host-only check of tools/cts/mp2-p9-record-check.py on synthetic records in
# a temporary directory: clean gl_test and dEQP records, then one record per
# failure. Nothing here is a real capture.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2-p9-record-check.py
D=$(mktemp -d "${TMPDIR:-/tmp}/mp2-p9-rc.XXXXXX") || exit 2
trap 'rm -rf "${D:?}"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

SHA=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
GOOD=20261124-1000-gl_test_s1-01234567

# mk NAME: a clean gl_test record
mk() {
    local r=$D/$1
    mkdir -p "$r"
    cat >"$r/meta.txt" <<M
board=synthetic
image_sha256=$SHA
u_boot_env=byte-for-byte
bootfs_list=staged
stock_files=unchanged
M
    printf 'MP2: start\ngl_test: all: 3 of 50 cases failed\n' >"$r/console.txt"
}
# t LABEL RECORD EXPECTED_RC PATTERN: run on RECORD, expect the rc and a line matching PATTERN
t() {
    local out rc
    out=$(python3 "$TOOL" "$D/$2" 2>&1); rc=$?
    if [ "$rc" = "$3" ] && grep -qE -- "$4" <<<"$out"; then ok "$1"; else bad "$1 (rc=$rc)"; printf '%s\n' "$out" | sed 's/^/        /'; fi
}

mk $GOOD
t "clean gl_test record" $GOOD 0 '^mp2-p9-record-check: 1 record\(s\), 0 problem\(s\)$'

R=20261124-1010-deqp_gles2_a-01234567; mk $R
printf 'MP2: start\n' >"$D/$R/console.txt"; printf '# index\tfilter\n0\tf\n' >"$D/$R/results.tsv"
t "clean dEQP record" $R 0 ' 0 problem\(s\)$'

R=20261124-1020-gl_test_s2-01234567; mk $R
printf 'MP2: start\n' >"$D/$R/console.txt"; echo 'no_tally=run ended by the watchdog' >>"$D/$R/meta.txt"
t "explicit no tally" $R 0 ' 0 problem\(s\)$'

R=20261124-1030-gl_test_s3-01234567; mk $R; printf 'MP2: start\n' >"$D/$R/console.txt"
t "no tally, no reason" $R 1 'tally: console.txt has no'

R=bad-name; mk $R
t "bad name" $R 1 'name: .*not <YYYYMMDD'

R=20261124-1040-gl_test_s4-89abcdef; mk $R
t "sha8 differs from the commit" $R 1 'commit: .*does not start'

R=20261124-1050-gl_test_s5-01234567; mk $R; sed -i '/image_sha256/d' "$D/$R/meta.txt"
t "no commit in meta" $R 1 'commit: meta.txt records no'

R=20261124-1100-gl_test_s6-01234567; mk $R; rm -f "${D:?}/${R:?}/console.txt"
t "console missing" $R 1 'files: console.txt is missing'

R=20261124-1110-gl_test_s7-01234567; mk $R; : >"$D/$R/meta.txt"
t "meta empty" $R 1 'files: meta.txt is empty'

R=20261124-1120-gl_test_s8-01234567; mk $R; sed -i '/u_boot_env/d' "$D/$R/meta.txt"
t "u-boot-env restore not recorded" $R 1 'restore: .*u-boot-env'

R=20261124-1130-gl_test_s9-01234567; mk $R; sed -i 's/^stock_files=.*/stock_files=differ/' "$D/$R/meta.txt"
t "/boot restore not recorded" $R 1 'restore: .*/boot'

R=20261124-1140-deqp_gles2_b-01234567; mk $R; printf 'MP2: start\n' >"$D/$R/console.txt"
t "dEQP without results" $R 1 'results: no non-empty'

# redaction: the texts are assembled so this file holds none of them
IP="10.1.2"".3"; TTY="/dev/t""tyUSB0"; HM="/ho""me/x"; UH="someone""@host1"
for pair in "ipv4:$IP" "tty-path:$TTY" "home-path:$HM" "login:$UH"; do
    pn=${pair%%:*}; txt=${pair#*:}
    R=20261124-1150-gl_test_r$pn-01234567; mk "$R"; printf 'line %s here\n' "$txt" >>"$D/$R/console.txt"
    t "redaction $pn" "$R" 1 "redaction: console.txt:3: $pn\$"
    out=$(python3 "$TOOL" "$D/$R" 2>&1)
    if grep -qF -- "$txt" <<<"$out"; then bad "redaction $pn: output repeats the match"; else ok "redaction $pn: output does not repeat the match"; fi
done

R=20261124-1200-gl_test_rv-01234567; mk $R; printf 'v1.2.3 and 1.2.3\n' >>"$D/$R/console.txt"
t "three-part version is not an address" $R 0 ' 0 problem\(s\)$'

out=$(python3 "$TOOL" "$D/$GOOD" "$D/bad-name" 2>&1); rc=$?
if [ $rc = 1 ] && grep -q '^mp2-p9-record-check: 2 record(s), ' <<<"$out"; then ok "two records"; else bad "two records"; fi

python3 "$TOOL" "$D/nonexistent" >/dev/null 2>&1; [ $? = 2 ] && ok "not a directory: rc 2" || bad "not a directory: rc 2"
python3 "$TOOL" >/dev/null 2>&1; [ $? = 2 ] && ok "no argument: rc 2" || bad "no argument: rc 2"

echo "mp2_p9_record_check_selftest: $pass ok, $fail failed"
[ $fail = 0 ]
