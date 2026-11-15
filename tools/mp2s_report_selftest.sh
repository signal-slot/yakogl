#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_report_selftest.sh - tools/mp2s_report.py without a board (plan #362,
# item report-builder).
#
#   tools/mp2s_report_selftest.sh
#
# Fake record directories under $TMPDIR, one per case:
#   - complete: verdict file, console log, identical manifest pair -> exit 0,
#     the record name and sha8, the verdict lines, "boots left 1" / "boots
#     left 0" / the login prompt with their line numbers, "byte-identical:
#     yes", both draft comments with "Result: met";
#   - no manifest -> exit 3, "missing: sha256 manifest";
#   - differing manifest -> exit 0, "byte-identical: no", the stock file that
#     differs, our yakogl/standalone/ file listed apart, "Result: not met";
#   - a *manifest*diff* file instead of the pair, identical and differing;
#   - an IP address, a /dev/tty path, a USB serial, a host name and the
#     BOARD_HOST value in the record -> every one replaced with <redacted>,
#     no address left in the output, the count printed;
#   - no console log, no verdict file, a name without a sha8 -> exit 3;
#   - usage errors -> exit 64.
#
# A tools/check.sh row for this selftest waits for plan #341, so it is run
# by hand: tools/mp2s_report_selftest.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-report-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0

report() { python3 -I "$here/mp2s_report.py" "$@"; }

ok() { pass=$((pass + 1)); echo "ok $pass - $1"; }
bad() { fail=$((fail + 1)); echo "not ok - $1"; }

# run NAME WANT_RC DIR...: runs the tool, keeps stdout in $T/NAME.out.
run() {
	local name=$1 want=$2 rc=0
	shift 2
	report "$@" >"$T/$name.out" 2>"$T/$name.err" || rc=$?
	if [ "$rc" = "$want" ]; then ok "$name: exit $rc"; else bad "$name: exit $rc, want $want"; cat "$T/$name.out" "$T/$name.err"; fi
}
has() { # NAME TEXT (fixed string)
	if grep -qF -- "$2" "$T/$1.out"; then ok "$1: has '$2'"; else bad "$1: lacks '$2'"; fi
}
hasnt() { # NAME ERE
	if grep -qE -- "$2" "$T/$1.out"; then bad "$1: matches /$2/"; grep -nE -- "$2" "$T/$1.out"; else ok "$1: no /$2/"; fi
}

H1=1111111111111111111111111111111111111111111111111111111111111111
H2=2222222222222222222222222222222222222222222222222222222222222222
H3=3333333333333333333333333333333333333333333333333333333333333333
H4=4444444444444444444444444444444444444444444444444444444444444444

# mkrec DIR: a complete record.
mkrec() {
	mkdir -p "$1"
	cat >"$1/record-verdict.txt" <<-EOF
	verdict: ok
	PASS boots left 1 then 0, then Linux
	ok 1 - image ran twice
	some other line
	EOF
	cat >"$1/console.log" <<-'EOF'
	1763200000 NOTICE:  BL2: v2.10-stm32mp2-r1.0
	1763200001 mp2s: undo: setenv mp2s_left 0; saveenv; boot   (or a PC: delete yakogl/standalone/boot.scr.uimg)
	1763200002 mp2s: default /yakogl/standalone/default.bin, boots left 1
	1763200003 mp2s: loaded 1a2b0 bytes at 0xa0000000, RAM footprint 0x22000 of the 0x10000000-byte window
	1763200004 mp2s: go 0xa0000000
	1763200005 *** Booting Zephyr OS build v4.1.0 ***
	1763200040 NOTICE:  BL2: v2.10-stm32mp2-r1.0
	1763200041 mp2s: default /yakogl/standalone/default.bin, boots left 0
	1763200042 mp2s: go 0xa0000000
	1763200080 NOTICE:  BL2: v2.10-stm32mp2-r1.0
	1763200081 mp2s: default used up, booting Linux
	1763200082 SCRIPT FAILED: continuing...
	1763200100 stm32mp25-disco-e3-c4 login:
	EOF
	printf '%s  ./st-boot.scr.uimg\n%s  ./mmc0_extlinux/extlinux.conf\n%s  ./yakogl/standalone/boot.scr.uimg\n' \
		"$H1" "$H2" "$H3" >"$1/boot-manifest-before.txt"
	cp "$1/boot-manifest-before.txt" "$1/boot-manifest-after.txt"
}

R=$T/records/stm32mp257f-dk
NAME=20261116-1015-standalone-mp2_hello-1556113d

# complete
mkrec "$R/$NAME"
run complete 0 "$R/$NAME"
has complete "record: \`$NAME\`"
has complete "commit: \`1556113d\`"
has complete '`record-verdict.txt`: `verdict: ok`'
has complete '`record-verdict.txt`: `PASS boots left 1 then 0, then Linux`'
has complete '`record-verdict.txt`: `ok 1 - image ran twice`'
hasnt complete 'some other line'
has complete '`console.log:3`: `1763200002 mp2s: default /yakogl/standalone/default.bin, boots left 1`'
has complete '`console.log:8`: `1763200041 mp2s: default /yakogl/standalone/default.bin, boots left 0`'
has complete '`console.log:12`: `1763200082 SCRIPT FAILED: continuing...`'
has complete '`console.log:13`: `1763200100 <redacted> login:`'
has complete 'order boots left 1, boots left 0, Linux login: yes'
has complete 'stock boot files byte-identical: yes'
has complete '## Draft comment for #1281 (not posted)'
has complete '## Draft comment for #7 (not posted)'
has complete '> Result: met.'
has complete "criterion 'a standalone boot from its own storage'"
hasnt complete '^missing: |- missing: '
hasnt complete 'stm32mp25-disco'
has complete 'redactions: 1 '

# no manifest
mkrec "$T/nomani/20261116-1016-standalone-1556113d"
rm "$T"/nomani/*/boot-manifest-*
run nomanifest 3 "$T/nomani/20261116-1016-standalone-1556113d"
has nomanifest '- missing: sha256 manifest'
has nomanifest 'stock boot files byte-identical: missing'
has nomanifest 'Result: incomplete record.'
has nomanifest 'boots left 1'

# differing manifest (one stock file changed, one stock file added, our file removed)
D=$T/diffm/20261116-1017-standalone-1556113d
mkrec "$D"
printf '%s  ./st-boot.scr.uimg\n%s  ./mmc0_extlinux/extlinux.conf\n%s  ./ubootefi.var\n' \
	"$H1" "$H4" "$H2" >"$D/boot-manifest-after.txt"
run diffmanifest 0 "$D"
has diffmanifest 'stock boot files byte-identical: no'
has diffmanifest 'differs (changed): `./mmc0_extlinux/extlinux.conf`'
has diffmanifest 'differs (added): `./ubootefi.var`'
has diffmanifest 'ours, not counted (removed): `./yakogl/standalone/boot.scr.uimg`'
hasnt diffmanifest 'differs \(.*\): `./st-boot'
has diffmanifest 'Result: not met.'

# a diff file instead of the pair
D=$T/difffile/20261116-1018-standalone-1556113d
mkrec "$D"
rm "$D"/boot-manifest-*
: >"$D/boot-manifest-diff.txt"
run difffile-same 0 "$D"
has difffile-same 'from `boot-manifest-diff.txt`'
has difffile-same 'stock boot files byte-identical: yes'
printf -- '--- before\n+++ after\n@@ -1,1 +1,1 @@\n-%s  ./st-boot.scr.uimg\n+%s  ./st-boot.scr.uimg\n' "$H1" "$H2" >"$D/boot-manifest-diff.txt"
run difffile-differ 0 "$D"
has difffile-differ 'stock boot files byte-identical: no'
has difffile-differ 'differs (changed): `./st-boot.scr.uimg`'

# redaction
D=$T/ip/20261116-1019-standalone-1556113d
mkrec "$D"
cat >>"$D/record-verdict.txt" <<-'EOF'
FAIL ssh to 192.168.10.23 port 22 timed out (via dk-lab.example.lan)
not ok 2 - console /dev/ttyUSB0 serial: 002A00123456 on fe80::1ff:fe23:4567:890a
verdict: ok on secretboardhost
EOF
sed -i '0,/mp2s: go 0xa0000000$/s|mp2s: go 0xa0000000$|mp2s: refused: load failed from 10.1.2.3 root@board7, booting Linux|' "$D/console.log"
BOARD_HOST=secretboardhost run ip 0 "$D"
has ip '<redacted>'
hasnt ip '([0-9]{1,3}\.){3}[0-9]{1,3}'
hasnt ip 'fe80|::1ff'
hasnt ip '/dev/tty'
hasnt ip '002A00123456'
hasnt ip 'secretboardhost'
hasnt ip 'dk-lab|example\.lan|board7'
has ip 'FAIL ssh to <redacted> port 22'
has ip 'mp2s: refused: load failed from <redacted> root@<redacted>, booting Linux'
has ip 'serial: <redacted>'
has ip 'Result: not met.'
has ip 'redactions: 9 '

# other missing parts
D=$T/nolog/20261116-1020-standalone-1556113d
mkrec "$D"
rm "$D/console.log"
run nolog 3 "$D"
has nolog '- missing: console log (*.log)'
D=$T/noverdict/20261116-1021-standalone-1556113d
mkrec "$D"
rm "$D/record-verdict.txt"
run noverdict 3 "$D"
has noverdict '- missing: verdict file'
D=$T/nolinux/20261116-1022-standalone-1556113d
mkrec "$D"
sed -i '/login:/d' "$D/console.log"
run nolinux 3 "$D"
has nolinux "- missing: console line 'Linux login'"
D=$T/nosha/20261116-1023-standalone
mkrec "$D"
run nosha 3 "$D"
has nosha '- missing: commit sha8 in the record name'

# usage
run usage-none 64
run usage-two 64 "$R/$NAME" "$R/$NAME"
run usage-notdir 64 "$T/does-not-exist"
run usage-file 64 "$R/$NAME/console.log"

echo "$pass passed, $fail failed"
[ "$fail" = 0 ]
