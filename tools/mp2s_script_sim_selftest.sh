#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_script_sim_selftest.sh - the DK standalone product script
# (driver/tools/mp2-standalone/mp2-standalone.cmd) run boot after boot in
# tools/mp2s_script_sim.py, no board, no QEMU (plan #362, item script-sim).
#
#   tools/mp2s_script_sim_selftest.sh [SCRIPT]
#
# SCRIPT defaults to the merged mp2-standalone.cmd. Cases:
#   - N=2: the console record is section 7's three-boot record of
#     docs/notes/stm32mp257f-dk-standalone.md line for line (boots left 1, the
#     image, boots left 0, the image, default used up, booting Linux);
#   - N=1..9: Linux at boot N+1, N image runs, "boots left" N-1 down to 0,
#     N+1 saves, the env restored and nothing else changed;
#   - forever: no Linux within 20 boots, 20 image runs, no save;
#   - default.bin missing (each boot refused, Linux on the same boot; the
#     count still runs out), the script missing (Linux, the env untouched),
#     a wrong or missing mp2s_ver (refused, no save, the env untouched), no
#     mp2s_left, the image returning, load failing, saveenv failing, a file
#     below the Image header, an image with a bad magic, a value that is not
#     a count, mp2s_prefixes missing;
#   - the fault mode: a power cut before, during and after each saveenv, for
#     N=1..9, and with the flags wrapping 255 -> 0;
#   - negative: mutated copies of the script that the interpreter must refuse
#     (an unknown command, an unknown test operator, a single quote) or whose
#     checks must fail (bootcmd written, boot_prefixes not restored).
#
# A tools/check.sh row for this selftest waits for plan #341, so it is run
# by hand: tools/mp2s_script_sim_selftest.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/.." && pwd)
SCRIPT=${1:-$root/driver/tools/mp2-standalone/mp2-standalone.cmd}
SIM="$here/mp2s_script_sim.py"
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-script-sim-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0
ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -f "$T/out" ] && sed 's/^/     /' "$T/out"; return 0; }

# sim ARGS...: run the simulator, output in $T/out, exit code in $rc.
sim() { rc=0; python3 "$SIM" "$@" > "$T/out" 2>&1 || rc=$?; }
# has TEXT: $T/out has a line that is exactly TEXT.
has() { grep -qxF -- "$1" "$T/out"; }
count() { grep -cxF -- "$1" "$T/out" || true; }

UNDO='mp2s: undo: setenv mp2s_left 0; saveenv; boot   (or a PC: delete yakogl/standalone/boot.scr.uimg)'
LOADED='mp2s: loaded 1000 bytes at 0xa0000000, RAM footprint 0x20000 of the 0x10000000-byte window'
GO='mp2s: go 0xa0000000'
LINUX="sim: boot %d: prefix /mmc0_: ST's extlinux, booting Linux (OpenSTLinux)"
RESTORED='sim: env: boot_prefixes "/mmc0_", no mp2s_*'
UNTOUCHED='sim: env: boot_prefixes "/yakogl/standalone/ /mmc0_", mp2s_left=2, mp2s_prefixes=/mmc0_, mp2s_ver=1'
NOCHANGE='sim: env: other variables changed: none'

# --- N=2: section 7's record, line for line (console lines only).
sim run "$SCRIPT" --left 2 --check-restored
grep -E '^(mp2s: |SCRIPT FAILED|sim: boot [0-9]+: prefix /mmc0_)' "$T/out" > "$T/record" || true
{
	printf '%s\n' "$UNDO" 'mp2s: default /yakogl/standalone/default.bin, boots left 1' "$LOADED" "$GO"
	printf '%s\n' "$UNDO" 'mp2s: default /yakogl/standalone/default.bin, boots left 0' "$LOADED" "$GO"
	printf '%s\n' "$UNDO" 'mp2s: default used up, booting Linux' 'SCRIPT FAILED: continuing...'
	printf "$LINUX\n" 3
} > "$T/want"
if [ "$rc" = 0 ] && cmp -s "$T/record" "$T/want" && has 'sim: result: Linux at boot 3, boots 3, image runs 2, saves 3' \
	&& has "$RESTORED" && has "$NOCHANGE"; then
	ok "N=2: section 7's three-boot record, Linux at boot 3, env restored"
else
	diff "$T/want" "$T/record" > "$T/diff" || true
	cat "$T/diff" >> "$T/out"
	bad "N=2: section 7's three-boot record"
fi
# The four markers of the contract, in that order.
if grep -nE 'boots left 1$|boots left 0$|default used up|booting Linux \(OpenSTLinux\)' "$T/out" |
	sed 's/^[0-9]*://' | sed -n '1p;2p;3p;4p' | tr '\n' '|' |
	grep -qF 'mp2s: default /yakogl/standalone/default.bin, boots left 1|mp2s: default /yakogl/standalone/default.bin, boots left 0|mp2s: default used up, booting Linux|sim: boot 3: prefix /mmc0_:'; then
	ok "N=2: boots left 1, boots left 0, default used up, booting Linux in that order"
else
	bad "N=2: marker order"
fi

# --- N=1..9.
for n in 1 2 3 4 5 6 7 8 9; do
	sim run "$SCRIPT" --left "$n" --check-restored
	why=
	[ "$rc" = 0 ] || why="exit $rc"
	has "sim: result: Linux at boot $((n + 1)), boots $((n + 1)), image runs $n, saves $((n + 1))" || why="$why result"
	grep -E 'boots left [0-9]+$' "$T/out" | sed 's/.* //' | tr '\n' ' ' > "$T/left"
	want=$(seq $((n - 1)) -1 0 | tr '\n' ' ')
	[ "$(cat "$T/left")" = "$want" ] || why="$why counts($(cat "$T/left"))"
	[ "$(count "$GO")" = "$n" ] || why="$why go-count"
	[ "$(count 'mp2s: default used up, booting Linux')" = 1 ] || why="$why used-up"
	has "$RESTORED" && has "$NOCHANGE" || why="$why env"
	if [ -z "$why" ]; then ok "N=$n: Linux at boot $((n + 1)) after $n image runs, env restored"; else bad "N=$n:$why"; fi
done

# --- forever: never Linux within 20 boots.
sim run "$SCRIPT" --left forever --max-boots 20
if [ "$rc" = 0 ] && has 'sim: result: no Linux within 20 boots, image runs 20, saves 0' \
	&& [ "$(count 'mp2s: default /yakogl/standalone/default.bin, forever')" = 20 ] \
	&& ! grep -q 'booting Linux (OpenSTLinux)' "$T/out" && has "$NOCHANGE"; then
	ok "forever: no Linux within 20 boots, the image at each, no save"
else
	bad "forever"
fi

# --- default.bin missing: refused at each boot, Linux on the same boot, the
# count still runs out and restores the env at boot 3.
sim run "$SCRIPT" --left 2 --no-image --until-restored --check-restored
if [ "$rc" = 0 ] && has 'sim: result: Linux at boot 1, boots 3, image runs 0, saves 3' \
	&& [ "$(count 'mp2s: refused: /yakogl/standalone/default.bin not found on mmc 0:8, booting Linux')" = 2 ] \
	&& [ "$(count 'SCRIPT FAILED: continuing...')" = 3 ] && has "$RESTORED" && has "$NOCHANGE"; then
	ok "image missing: refused, Linux at every boot, restored after 3 boots"
else
	bad "image missing"
fi

# --- the script missing: no mp2s line, Linux, the env untouched.
sim run "$SCRIPT" --left 2 --no-script
if [ "$rc" = 0 ] && ! grep -q '^mp2s:' "$T/out" && has 'sim: result: Linux at boot 1, boots 1, image runs 0, saves 0' \
	&& has 'sim: boot 1: prefix /yakogl/standalone/: no extlinux, no boot.scr.uimg, no boot.scr' \
	&& has "$UNTOUCHED" && has "$NOCHANGE"; then
	ok "script missing: Linux at boot 1, env untouched"
else
	bad "script missing"
fi

# --- a wrong or missing contract version: refused, no save.
sim run "$SCRIPT" --left 2 --set mp2s_ver=2
if [ "$rc" = 0 ] && has 'mp2s: refused: contract version 2, expected 1, booting Linux' \
	&& has 'sim: result: Linux at boot 1, boots 1, image runs 0, saves 0' \
	&& has 'sim: env: boot_prefixes "/yakogl/standalone/ /mmc0_", mp2s_left=2, mp2s_prefixes=/mmc0_, mp2s_ver=2'; then
	ok "mp2s_ver=2: refused, Linux at boot 1, no save"
else
	bad "mp2s_ver=2"
fi
sim run "$SCRIPT" --left 2 --unset mp2s_ver
if [ "$rc" = 0 ] && has 'mp2s: refused: contract version , expected 1, booting Linux' \
	&& has 'sim: result: Linux at boot 1, boots 1, image runs 0, saves 0'; then
	ok "mp2s_ver missing: refused, no save"
else
	bad "mp2s_ver missing"
fi

# --- no mp2s_left (the prefix alone).
sim run "$SCRIPT" --left 2 --unset mp2s_left
if [ "$rc" = 0 ] && has 'mp2s: no default set, back to the distro loop' && has 'sim: result: Linux at boot 1, boots 1, image runs 0, saves 0'; then
	ok "no mp2s_left: no default set, Linux, no save"
else
	bad "no mp2s_left"
fi

# --- the image returns: Linux on the same boot, the count already saved.
sim run "$SCRIPT" --left 2 --image-returns --until-restored --check-restored
if [ "$rc" = 0 ] && [ "$(count 'mp2s: the image returned, booting Linux')" = 2 ] \
	&& has 'sim: result: Linux at boot 1, boots 3, image runs 2, saves 3' && has "$RESTORED"; then
	ok "image returns: Linux on the same boot, restored after 3 boots"
else
	bad "image returns"
fi

# --- load fails, saveenv fails.
sim run "$SCRIPT" --left 2 --load-fails
if [ "$rc" = 0 ] && has 'mp2s: refused: load failed, booting Linux' && has 'sim: result: Linux at boot 1, boots 1, image runs 0, saves 1'; then
	ok "load fails: refused after the save, Linux at boot 1"
else
	bad "load fails"
fi
sim run "$SCRIPT" --left 2 --saveenv-fails
if [ "$rc" = 0 ] && has 'mp2s: saveenv failed, booting Linux' && ! grep -q 'boots left' "$T/out" \
	&& has 'sim: result: Linux at boot 1, boots 1, image runs 0, saves 0' && has "$UNTOUCHED"; then
	ok "saveenv fails: nothing loaded, Linux at boot 1, env untouched"
else
	bad "saveenv fails"
fi

# --- images the checks refuse.
head -c 32 /dev/zero > "$T/small.bin"
sim run "$SCRIPT" --left 1 --image "$T/small.bin"
if [ "$rc" = 0 ] && has 'mp2s: refused: file 20 bytes, below the 0x40-byte Image header, booting Linux'; then
	ok "a 0x20-byte file: refused below the Image header"
else
	bad "small file"
fi
head -c 4096 /dev/zero > "$T/nomagic.bin"
sim run "$SCRIPT" --left 1 --image "$T/nomagic.bin"
if [ "$rc" = 0 ] && has 'mp2s: refused: no ARM64 Image magic at byte 56 of the loaded file, booting Linux'; then
	ok "no ARM64 magic: refused"
else
	bad "no magic"
fi

# --- counts that are not 1-9 take the used-up branch.
for v in 0 a 10 -1; do
	sim run "$SCRIPT" --left 2 --set "mp2s_left=$v" --check-restored
	if [ "$rc" = 0 ] && has 'mp2s: default used up, booting Linux' && has 'sim: result: Linux at boot 1, boots 1, image runs 0, saves 1' && has "$RESTORED"; then
		ok "mp2s_left=$v: used up, restored at boot 1"
	else
		bad "mp2s_left=$v"
	fi
done
sim run "$SCRIPT" --left 1 --unset mp2s_prefixes --set mp2s_left=0
if [ "$rc" = 0 ] && has 'mp2s: refused: mp2s_prefixes is not set, boot_prefixes left as it is, booting Linux' \
	&& has 'sim: result: Linux at boot 1, boots 1, image runs 0, saves 0'; then
	ok "used up without mp2s_prefixes: refused, boot_prefixes left alone"
else
	bad "no mp2s_prefixes"
fi

# --- the fault mode: a power cut at each saveenv, N=1..9.
for n in 1 2 3 4 5 6 7 8 9; do
	sim faults "$SCRIPT" --left "$n"
	cuts=$((3 * (n + 1)))
	if [ "$rc" = 0 ] && has "sim: faults: N=$n, $cuts cuts, 0 failed" && [ "$(grep -c ': ok$' "$T/out")" = $((cuts + 1)) ]; then
		ok "faults N=$n: $cuts cuts, each Linux within N+1 boots, env restored"
	else
		bad "faults N=$n"
	fi
done
sim faults "$SCRIPT" --left 3 --start-flags 254,255
if [ "$rc" = 0 ] && has 'sim: faults: N=3, 12 cuts, 0 failed'; then
	ok "faults N=3 with the flags wrapping 255 -> 0"
else
	bad "faults with flag wrap"
fi

# --- negative cases on mutated copies of the script.
mutate() { sed "$1" "$SCRIPT" > "$T/m.cmd"; }
mutate 's/^setenv mp2s_load 0xa0000000$/run mp2s_load/'
sim run "$T/m.cmd" --left 1
if [ "$rc" = 2 ] && grep -q "^sim: error: line [0-9]*: command 'run' is not in the subset" "$T/out"; then
	ok "unknown command: sim error, exit 2"
else
	bad "unknown command (exit $rc)"
fi
mutate 's/test -z "${mp2s_left}"/test -e "${mp2s_left}"/'
sim run "$T/m.cmd" --left 1
if [ "$rc" = 2 ] && grep -q '^sim: error: line [0-9]*: test expression not in the subset' "$T/out"; then
	ok "unknown test operator: sim error, exit 2"
else
	bad "unknown test operator (exit $rc)"
fi
mutate "s/^setenv mp2s_min 0x40\$/setenv mp2s_min '0x40'/"
sim run "$T/m.cmd" --left 1
if [ "$rc" = 2 ] && grep -q "^sim: error: line [0-9]*: ''' is not in the subset" "$T/out"; then
	ok "single quote: sim error, exit 2"
else
	bad "single quote (exit $rc)"
fi
mutate 's/^\tsetenv mp2s_ver$/\tsetenv mp2s_ver; setenv bootcmd reset/'
sim run "$T/m.cmd" --left 1
if [ "$rc" = 1 ] && grep -q "^sim: FAIL: variables outside mp2s_\* and boot_prefixes changed: bootcmd" "$T/out"; then
	ok "a script that writes bootcmd: the env check fails"
else
	bad "bootcmd written (exit $rc)"
fi
mutate 's/^\tsetenv boot_prefixes "${mp2s_prefixes}"$//'
sim faults "$T/m.cmd" --left 2
if [ "$rc" = 1 ] && grep -q 'FAIL: .*env not restored' "$T/out"; then
	ok "a script that does not restore boot_prefixes: the fault mode fails"
else
	bad "boot_prefixes not restored (exit $rc)"
fi
# A count that is saved only after go: the image resets first, so the save
# never runs and the board never comes back.
python3 - "$SCRIPT" "$T/late.cmd" <<'PY'
import sys
s = open(sys.argv[1]).read()
save = '\tif saveenv; then\n\t\techo "mp2s: default /yakogl/standalone/default.bin, boots left ${mp2s_left}"\n\telse\n\t\techo "mp2s: saveenv failed, booting Linux"\n\t\texit\n\tfi\n'
assert save in s
s = s.replace(save, '\techo "mp2s: default /yakogl/standalone/default.bin, boots left ${mp2s_left}"\n')
go = 'dcache flush; dcache off; icache off; go ${mp2s_load}\n'
assert go in s
s = s.replace(go, go + 'saveenv\n')
open(sys.argv[2], "w").write(s)
PY
sim run "$T/late.cmd" --left 2 --check-restored
if [ "$rc" = 1 ] && has 'sim: result: no Linux within 20 boots, image runs 20, saves 0' && has 'sim: FAIL: Linux was not reached'; then
	ok "a count saved only after go: no Linux within 20 boots, the check fails"
else
	bad "late save (exit $rc)"
fi
# The RAM-only variables saved: the count saved after the load checks.
mutate 's/^\(dcache flush; dcache off; icache off; go \${mp2s_load}\)$/saveenv; \1/'
sim run "$T/m.cmd" --left 2
if [ "$rc" = 0 ] && grep -q '^sim: env: boot_prefixes "/mmc0_", mp2s_chk=' "$T/out"; then
	ok "a save after the load: the RAM-only mp2s_* end up stored"
else
	bad "RAM variables saved (exit $rc)"
fi

rm -f "$T/out"
echo "mp2s_script_sim selftest: $pass ok, $fail failed"
[ "$fail" = 0 ]
