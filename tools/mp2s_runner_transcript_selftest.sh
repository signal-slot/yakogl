#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_runner_transcript_selftest.sh - the DK standalone runner's own
# transcripts, dry and real, on the fake board (tools/mp2s_fakeboard.sh), for
# the four verbs install, default <image> 2, nodefault and uninstall:
#
#   - every transcript passes tools/mp2s_undo_order.py (an undo before each
#     change; for a real run the transcript is the runner's output, which
#     carries the same UNDO lines, and is checked when it has DRY lines);
#   - no transcript, no fw_setenv line and no ssh command names bootcmd,
#     bootcount or altbootcmd as a written variable, or fsbla1/fsbla2/
#     metadata1/metadata2/fip-a/fip-b/mmcblk*boot*/the eMMC (mmcblk0) as a
#     write target;
#   - after uninstall the fake /boot matches its first manifest
#     (tools/mp2s_boot_manifest.py), and the fake board's protected
#     partitions are byte for byte as init made them.
#
# No board, no ssh, no network. Everything lives in $TMPDIR.
#
#   tools/mp2s_runner_transcript_selftest.sh
#
# Prints "mp2s_runner_transcript_selftest: N checks, F failed"; exit 0 only
# when F is 0.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
R="$HERE/board-stm32mp2-standalone.sh"
W=$(mktemp -d)
[ -n "${MP2S_RT_KEEP:-}" ] || trap 'rm -rf "$W"' EXIT
FB=$W/fb

N=0
F=0
ok() { N=$((N + 1)); echo "ok   $1"; }
bad() { N=$((N + 1)); F=$((F + 1)); echo "FAIL $1: $2"; }
check() { if [ "$2" = 0 ]; then ok "$1"; else bad "$1" "${3:-}"; fi; }

unset BOARD_PERSON_PRESENT MP2_NO_WATCHDOG MP2_SUDO MP2_KNOWN_HOSTS MP2S_BOOT_MANIFEST
export MP2S_STATE_DIR=$W/state BOARD_LOCK=$W/board.lock LOCK_WAIT=5
"$HERE/mp2s_fakeboard.sh" init "$FB" >/dev/null || { echo "fake board init failed"; exit 1; }
eval "$("$HERE/mp2s_fakeboard.sh" shell "$FB")"
export BOARD_HOST=user@host   # a placeholder: the fake ssh ignores it

# a test image that passes the image check
IMG=$W/img.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'gc_wdog_arm: armed'; } >"$IMG"
python3 -I "$HERE/mp2s_image_check.py" "$IMG" >"$W/ic.out" 2>&1 || { echo "test image fails mp2s_image_check.py: $(cat "$W/ic.out")"; exit 1; }

prot() { (cd "$FB" && sha256sum dev/mmcblk1p1 dev/mmcblk1p2 dev/mmcblk1p3 dev/mmcblk1p4 dev/mmcblk1p5 dev/mmcblk1p6 dev/mmcblk0p7); }
prot >"$W/prot.sum"
python3 -I "$HERE/mp2s_boot_manifest.py" record "$FB/boot" -o "$W/first.manifest" || { echo "manifest failed"; exit 1; }

# the protected-name rules, applied to a transcript file
WRITE_VERB='(of=|\bdd\b|\bcp\b|\binstall\b|\bmv\b|\brm\b|\btee\b|\bmkfs|\bsgdisk\b|\bsfdisk\b|\bparted\b|\bfw_setenv\b|>)'
PROT_PART='(fsbla[12]|metadata[12]|fip-[ab]|mmcblk[0-9]+boot[0-9]*|mmcblk0)'
PROT_VAR='(bootcmd|bootcount|altbootcmd)'
protected_names() {   # FILE -> prints offending lines
    local f=$1
    # a fw_setenv script line or a setenv/env set naming a protected variable
    grep -En "(^|[[:space:];&|])(fw_setenv|setenv|env +set|env +delete)[^#]*(^|[[:space:]'\"])$PROT_VAR([[:space:]='\";]|\$)" "$f" || true
    grep -En "^$PROT_VAR=" "$f" || true
    # a write verb on a line that names a protected partition or the eMMC
    grep -E "$WRITE_VERB" "$f" | grep -E "(^|[^A-Za-z0-9_-])$PROT_PART([^A-Za-z0-9_-]|\$)" | grep -Ev '^(skip|note|info):' || true
}

# run NAME VERB...: run the runner, keep its transcript in $W/NAME.txt. A real
# run's transcript is the runner's output (its UNDO lines) followed by the
# changes the fake board logged during that run (writes.log lines since the
# run began), written in the "DRY board$" form mp2s_undo_order.py reads: the
# runner prints every undo before its first change, so the undo lines come
# first and each logged change must find its undo among them.
run() {
    local name=$1 dry=0 before=0
    shift
    [ "$1" = --dry-run ] && dry=1
    [ -f "$FB/writes.log" ] && before=$(wc -l <"$FB/writes.log")
    "$R" "$@" >"$W/$name.txt" 2>&1
    echo $? >"$W/$name.rc"
    [ "$dry" = 1 ] && return 0
    tail -n +"$((before + 1))" "$FB/writes.log" >"$W/$name.writes"
    python3 -I - "$W/$name.writes" "$name" >>"$W/$name.txt" <<'PY'
import sys
stage = "/boot/yakogl/standalone"
mkdir = False
env = []
for ln in open(sys.argv[1]):
    w = ln.split()
    if w[:1] == ["setenv"] and len(w) >= 2:
        env.append(w[1] + "=" + w[2] if len(w) > 2 else w[1] + " (deleted)")
    elif w[:1] == ["file"] and len(w) >= 3:
        path = w[1]
        if w[2] == "removed":
            if path.endswith("/default.bin"):
                print("DRY board$ rm -f " + path)
            elif not mkdir and sys.argv[2].endswith("uninstall"):
                mkdir = True
                print("DRY board$ rm -rf " + stage)
        elif path.endswith("/default.bin"):
            print("DRY board$ tee " + path)
        elif not mkdir:
            mkdir = True
            print("DRY board$ mkdir -p " + stage)
if env:
    print("DRY board$ fw_setenv -s - # " + " ".join(env))
PY
}
transcript_checks() {   # NAME
    local name=$1 f=$W/$1.txt
    check "$name: runner exits 0" "$(cat "$W/$name.rc")" "rc $(cat "$W/$name.rc"): $(tail -5 "$f")"
    grep -q 'DRY board\$' "$f" && ok "$name: has board lines" || bad "$name: board lines" "none"
    if python3 -I "$HERE/mp2s_undo_order.py" "$f" >"$W/$name.uo" 2>&1; then ok "$name: passes mp2s_undo_order.py"
    else bad "$name: mp2s_undo_order.py" "$(cat "$W/$name.uo")"; fi
    local hits
    hits=$(protected_names "$f")
    if [ -z "$hits" ]; then ok "$name: no protected variable, partition or eMMC as a write target"; else bad "$name: protected names" "$hits"; fi
}

# ---- dry runs -----------------------------------------------------------------------
run dry-install --dry-run install
run dry-default --dry-run default "$IMG" 2
run dry-nodefault --dry-run nodefault
run dry-uninstall --dry-run uninstall
for v in install default nodefault uninstall; do transcript_checks "dry-$v"; done
if prot | cmp -s - "$W/prot.sum" && python3 -I "$HERE/mp2s_boot_manifest.py" compare "$FB/boot" "$W/first.manifest" >/dev/null 2>&1; then ok "the dry runs changed nothing on the fake board"
else bad "dry runs" "the fake board changed"; fi
[ ! -s "$FB/writes.log" ] && ok "the dry runs wrote nothing (writes.log empty)" || bad "dry runs" "writes.log: $(head -3 "$FB/writes.log")"

# ---- real runs on the fake board -----------------------------------------------------
: >"$FB/writes.log"
run real-install install
run real-default default "$IMG" 2
run real-nodefault nodefault
run real-uninstall uninstall
for v in install default nodefault uninstall; do transcript_checks "real-$v"; done

# the writes of the whole real cycle, as the fake board saw them
for log in writes.log ssh.log fw_setenv.log; do
    [ -f "$FB/$log" ] || continue
    hits=$(protected_names "$FB/$log")
    if [ -z "$hits" ]; then ok "$log: no protected variable, partition or eMMC as a write target"; else bad "$log: protected names" "$hits"; fi
done
if grep -q VIOLATION "$FB/writes.log" "$FB/ssh.log" 2>/dev/null; then bad "fake board" "$(grep -h VIOLATION "$FB/writes.log" "$FB/ssh.log" | head -3)"; else ok "the fake board saw no VIOLATION"; fi
if [ -f "$FB/fw_setenv.log" ] && grep -Evq '^(mp2s_ver|mp2s_left|mp2s_prefixes|boot_prefixes)([= ]|$)' "$FB/fw_setenv.log"; then
    bad "fw_setenv.log" "a variable other than mp2s_* / boot_prefixes: $(grep -Ev '^(mp2s_ver|mp2s_left|mp2s_prefixes|boot_prefixes)([= ]|$)' "$FB/fw_setenv.log" | head -3)"
else ok "fw_setenv wrote only mp2s_* and boot_prefixes (or nothing logged)"; fi

# after uninstall
python3 -I "$HERE/mp2s_boot_manifest.py" compare "$FB/boot" "$W/first.manifest" >"$W/cmp.out" 2>&1
check "after uninstall /boot matches its first manifest" "$?" "$(cat "$W/cmp.out")"
grep -q '^mp2s-boot: same' "$W/cmp.out" && ok "the verdict is 'mp2s-boot: same'" || bad "verdict" "$(tail -2 "$W/cmp.out")"
if [ -z "$(find "$FB/boot/yakogl" -type f 2>/dev/null)" ]; then ok "no staged file is left"; else bad "staged files" "left: $(find "$FB/boot/yakogl" -type f)"; fi
prot | cmp -s - "$W/prot.sum" && ok "fsbla1/2, metadata1/2, fip-a/b and the eMMC's env are unchanged" || bad "protected partitions" "changed"
python3 -I "$HERE/uboot_env.py" --redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000 print "$FB/dev/mmcblk1p7" 2>/dev/null |
    grep -Eq '^(mp2s_[a-z]+)=' && bad "environment after uninstall" "an mp2s_ variable is left" || ok "no mp2s_ variable is left in the environment"

echo "mp2s_runner_transcript_selftest: $N checks, $F failed"
[ "$F" = 0 ]
