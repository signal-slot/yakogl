#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# No-board selftest of tools/mp2s_env_plan.py: synthetic redundant environment
# images built here (two copies of 0x2000 bytes each: the standalone note
# gives no ST size, so this is a fixture size only), ST-like bootcmd,
# altbootcmd, bootcount and boot_prefixes='/mmc0_'. Needs python3 only.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
plan=$here/mp2s_env_plan.py
env=$here/uboot_env.py
work=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_env_plan.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
L=(--redundant --size 0x2000)
fail=0 n=0
ok()  { n=$((n+1)); echo "ok   $*"; }
bad() { n=$((n+1)); fail=$((fail+1)); echo "FAIL $*"; }
check() { local d=$1; shift; if "$@"; then ok "$d"; else bad "$d"; fi; }

# fixture: copy 1 flags 1 and copy 2 flags 0 (copy 1 active); args = NAME=VALUE...
mk() {
    local out=$1; shift
    python3 -I - "$out" "$@" <<'PY'
import sys, zlib
out, size = sys.argv[1], 0x2000
data = b"".join(a.encode() + b"\0" for a in sys.argv[2:]) + b"\0"
area = data + b"\0" * (size - 5 - len(data))
def cp(flags):
    return zlib.crc32(area).to_bytes(4, "little") + bytes([flags]) + area
open(out, "wb").write(cp(1) + cp(0))
PY
}
base=(bootcmd='run bootcmd_stm32mp' 'altbootcmd=run bootcmd_stm32mp' bootcount=0 bootlimit=3
      upgrade_available=0 'boot_targets=mmc0 mmc1' 'boot_prefixes=/mmc0_'
      'boot_syslinux_conf=extlinux/extlinux.conf' preboot=run\ preboot_st
      'boot_scripts=boot.scr.uimg' 'boot_extlinux=sysboot ${devtype} ${devnum}:${distro_bootpart} any ${scriptaddr} ${prefix}${boot_syslinux_conf}')
mk "$work/base.img" "${base[@]}"
refuse() { # desc, expected stderr fragment, then command; no OUT, no record
    local d=$1 frag=$2; shift 2
    rm -f "$work/o.img" "$work/o.rec"
    local err rc
    err=$("$@" 2>&1 >/dev/null); rc=$?
    if [ $rc = 2 ] && [ ! -e "$work/o.img" ] && [ ! -e "$work/o.rec" ] \
        && [ "$(printf '%s\n' "$err" | wc -l)" = 1 ] && [[ $err == *"$frag"* ]]; then
        ok "refuses: $d"
    else bad "refuses: $d (rc=$rc err=$err)"; fi
}
get() { python3 "$env" "${L[@]}" print -n "$2" "$1" 2>/dev/null; }
# base with boot_prefixes replaced by $1
withbp() { local e; for e in "${base[@]}"; do case $e in boot_prefixes=*) echo "boot_prefixes=$1";; *) echo "$e";; esac; done; }
D=(python3 "$plan" default "$work/base.img" "$work/o.img" "${L[@]}" --record "$work/o.rec")

# --- default
out=$("${D[@]}" --count 2 2>"$work/err"); rc=$?
check "default 2 exits 0" test $rc = 0
check "undo lines printed" grep -q '^UNDO, from Linux: .*nodefault' <<<"$out"
check "undo without Linux for N" grep -q 'none needed: after 2 boots' <<<"$out"
check "record is the live boot_prefixes, exact bytes" test "$(cat "$work/o.rec")" = /mmc0_
check "record has no newline" test "$(wc -c <"$work/o.rec")" = 6
check "mp2s_ver=1" test "$(get mp2s_ver "$work/o.img")" = 1
check "mp2s_left=2" test "$(get mp2s_left "$work/o.img")" = 2
check "mp2s_prefixes is the live value" test "$(get mp2s_prefixes "$work/o.img")" = /mmc0_
check "boot_prefixes has our prefix" test "$(get boot_prefixes "$work/o.img")" = '/yakogl/standalone/ /mmc0_'
cp "$work/o.img" "$work/def.img"; cp "$work/o.rec" "$work/def.rec"
check "verify default" python3 "$plan" verify "$work/base.img" "$work/def.img" --expect default "${L[@]}"
check "verify default of the unchanged image fails" \
    test "$(python3 "$plan" verify "$work/base.img" "$work/base.img" --expect default "${L[@]}" >/dev/null 2>&1; echo $?)" = 2
check "inactive copy written, active left alone" \
    cmp -s <(head -c 8192 "$work/base.img") <(head -c 8192 "$work/def.img")

# the undo is printed before OUT exists: a wrapper that checks at the first line
rm -f "$work/o.img" "$work/o.rec"
early=$(python3 -I - "$plan" "$work" "${L[@]}" <<'PY'
import os, subprocess, sys
plan, w, *lay = sys.argv[1:]
p = subprocess.Popen([sys.executable, plan, "default", w + "/base.img", w + "/o.img",
                      *lay, "--count", "forever", "--record", w + "/o.rec"],
                     stdout=subprocess.PIPE, text=True)
line = p.stdout.readline()
# the first UNDO line is read; OUT is written only after all undo lines
print("first=%s" % line.split(",")[0])
p.stdout.read(); p.wait()
PY
)
check "first output line is an UNDO line" test "$early" = "first=UNDO"
# order proof inside the tool: stdout is flushed before OUT's os.replace; check source order
check "OUT written after the undo print" python3 -I - "$plan" <<'PY'
import sys
s = open(sys.argv[1]).read()
f = s[s.index("def cmd_default"):s.index("def cmd_nodefault")]
sys.exit(0 if f.index("undo_lines(") < f.index("ue.write_out(") else 1)
PY
out=$("${D[@]}" --count forever 2>&1)
check "forever prints the two person lines" test "$(grep -c 'UNDO, without Linux' <<<"$out")" = 2
rm -f "$work/o.img" "$work/o.rec"

# --- nodefault restores everything
python3 "$plan" nodefault "$work/def.img" "$work/nd.img" "${L[@]}" --record "$work/def.rec" >/dev/null
check "nodefault exits 0" test $? = 0
check "verify nodefault" python3 "$plan" verify "$work/def.img" "$work/nd.img" --expect nodefault "${L[@]}"
check "boot_prefixes byte-identical after default+nodefault" \
    test "$(get boot_prefixes "$work/nd.img")" = /mmc0_
check "every variable as it was (print, base vs final)" \
    cmp -s <(python3 "$env" "${L[@]}" print "$work/base.img") <(python3 "$env" "${L[@]}" print "$work/nd.img")
for v in bootcmd altbootcmd bootcount; do
    for img in def nd; do
        check "$v bytes unchanged in $img" \
            test "$(python3 "$env" "${L[@]}" print -n "$work/base.img" "$v" | od -An -tx1)" = \
                 "$(python3 "$env" "${L[@]}" print -n "$work/$img.img" "$v" | od -An -tx1)"
    done
done
for v in bootlimit upgrade_available boot_targets preboot boot_syslinux_conf boot_extlinux boot_scripts; do
    check "$v unchanged across default and nodefault" \
        test "$(get $v "$work/base.img")" = "$(get $v "$work/nd.img")" -a "$(get $v "$work/base.img")" = "$(get $v "$work/def.img")"
done
# nothing to do: OUT identical
python3 "$plan" nodefault "$work/base.img" "$work/same.img" "${L[@]}" --record "$work/none.rec" >/dev/null
check "nodefault with nothing to do: exit 0, OUT == IMAGE" \
    bash -c "cmp -s '$work/base.img' '$work/same.img'"
# the used-up state (script restored boot_prefixes, deleted mp2s_*) is also nothing to do
python3 "$plan" nodefault "$work/nd.img" "$work/same2.img" "${L[@]}" --record "$work/def.rec" >/dev/null
check "nodefault again is a no-op" bash -c "cmp -s '$work/nd.img' '$work/same2.img'"

# --- default for every count, bad counts
for c in 1 9 forever; do
    rm -f "$work/o.img"
    check "count $c accepted" bash -c "python3 '$plan' default '$work/base.img' '$work/o.img' ${L[*]} --record '$work/o.rec' --count $c >/dev/null"
done
rm -f "$work/o.img" "$work/o.rec"
for c in 0 10 abc '' -1 Forever; do
    refuse "count '$c'" "--count" "${D[@]}" --count "$c"
done

# --- default refusals
mkr() { mk "$work/r.img" "$@"; }
R=(python3 "$plan" default "$work/r.img" "$work/o.img" "${L[@]}" --record "$work/o.rec" --count 1)
mkr "${base[@]}" mp2s_left=1;                 refuse "mp2s_left present" "already set" "${R[@]}"
mkr "${base[@]}" mp2s_prefixes=/mmc0_;        refuse "mp2s_prefixes present" "already set" "${R[@]}"
mkr "${base[@]}" mp2s_ver=1;                  refuse "mp2s_ver present" "already set" "${R[@]}"
mkr "${base[@]}" yako_once=1;                 refuse "yako_* present" "yako_once" "${R[@]}"
mkr "${base[@]}" mp2z_state=1;                refuse "mp2z_* present" "mp2z_state" "${R[@]}"
mkr "${base[@]/boot_prefixes=\/mmc0_/zz=1}";  refuse "no boot_prefixes" "boot_prefixes is not set" "${R[@]}"
mapfile -t bp < <(withbp '/yakogl/standalone/ /mmc0_'); mkr "${bp[@]}"
refuse "boot_prefixes already prefixed (with a space)" "already starts" "${R[@]}"
mapfile -t bp < <(withbp '/yakogl/standalone/x'); mkr "${bp[@]}"
refuse "boot_prefixes already prefixed (no space)" "already starts" "${R[@]}"
mkr "${base[@]}"
python3 -I - "$work/r.img" <<'PY'
import sys
p = sys.argv[1]; b = bytearray(open(p, "rb").read()); b[100] ^= 0xff; open(p, "wb").write(b)
PY
refuse "bad CRC in copy 1" "bad CRC" "${R[@]}"
mkr "${base[@]}"
python3 -I - "$work/r.img" <<'PY'
import sys
p = sys.argv[1]; b = bytearray(open(p, "rb").read()); b[0x2000 + 100] ^= 0xff; open(p, "wb").write(b)
PY
refuse "bad CRC in copy 2" "bad CRC" "${R[@]}"
refuse "missing image" "No such file" "${R[@]/$work\/r.img/$work/missing.img}"
check "--record missing: usage error, exit 2, no output" bash -c "python3 '$plan' default '$work/base.img' '$work/o.img' ${L[*]} --count 1 2>/dev/null; [ \$? = 2 ] && [ ! -e '$work/o.img' ]"

# --- nodefault refusals
mapfile -t pfx < <(withbp '/yakogl/standalone/ /mmc0_')
mk "$work/ns.img" "${pfx[@]}" mp2s_ver=1 mp2s_left=1 mp2s_prefixes=/mmc1_
N=(python3 "$plan" nodefault "$work/ns.img" "$work/o.img" "${L[@]}" --record "$work/def.rec")
refuse "mp2s_prefixes differs from the record" "differs from the record" "${N[@]}"
mk "$work/ns.img" "${pfx[@]}" mp2s_ver=1 mp2s_left=1
refuse "our prefix but no mp2s_prefixes" "needs a person" "${N[@]}"
mk "$work/ns.img" "${base[@]}" mp2s_ver=1 mp2s_left=1 mp2s_prefixes=/mmc0_ mp2s_load=0xa0000000
refuse "stray mp2s_ variable" "needs a person" "${N[@]}"
mk "$work/ns.img" "${pfx[@]}" mp2s_ver=1 mp2s_left=1 mp2s_prefixes=/mmc0_
refuse "record unreadable" "cannot read the record" python3 "$plan" nodefault "$work/ns.img" "$work/o.img" "${L[@]}" --record "$work/none.rec"
python3 -I - "$work/ns.img" <<'PY'
import sys
p = sys.argv[1]; b = bytearray(open(p, "rb").read()); b[100] ^= 0xff; open(p, "wb").write(b)
PY
refuse "nodefault bad CRC" "bad CRC" "${N[@]}"

# --- nodefault when the script already restored boot_prefixes but left mp2s_* (partial): drops them
mk "$work/ns.img" "${base[@]}" mp2s_ver=1 mp2s_left=0
python3 "$plan" nodefault "$work/ns.img" "$work/o.img" "${L[@]}" --record "$work/def.rec" >/dev/null
check "mp2s_* left over with plain boot_prefixes: dropped" \
    test "$(python3 "$env" "${L[@]}" print "$work/o.img" | grep -c '^mp2s_')" = 0 -a "$(get boot_prefixes "$work/o.img")" = /mmc0_

# --- the write guard: every forbidden name, through the one function
for v in bootcmd bootcmd_stm32mp altbootcmd bootcount bootlimit upgrade_available boot_targets \
         preboot boot_syslinux_conf boot_extlinux boot_scripts; do
    check "write_guard refuses a change of $v" python3 -I - "$here" "$v" <<'PY'
import importlib.util, sys
here, v = sys.argv[1:]
s = importlib.util.spec_from_file_location("m", here + "/mp2s_env_plan.py")
m = importlib.util.module_from_spec(s); s.loader.exec_module(m)
vb = v.encode()
before = [vb + b"=a", b"boot_prefixes=/mmc0_"]
for after in ([vb + b"=b", b"boot_prefixes=/mmc0_"],      # changed
              [b"boot_prefixes=/mmc0_"],                   # deleted
              before + [vb + b"=c"]):                      # shadowed by a later duplicate
    try:
        m.write_guard(before, after)
    except m.Refused:
        continue
    sys.exit(1)
m.write_guard(before, [vb + b"=a", b"boot_prefixes=/x", b"mp2s_ver=1"])  # allowed ones pass
PY
done
# a new forbidden variable (and one of ours only through an allowed name) is refused too
check "write_guard refuses an added bootcmd_foo" python3 -I - "$here" <<'PY'
import importlib.util, sys
s = importlib.util.spec_from_file_location("m", sys.argv[1] + "/mp2s_env_plan.py")
m = importlib.util.module_from_spec(s); s.loader.exec_module(m)
try:
    m.write_guard([b"a=1"], [b"a=1", b"bootcmd_foo=1"])
except m.Refused:
    sys.exit(0)
sys.exit(1)
PY

# --- verify refuses a hand-made forbidden change and a wrong result
python3 "$env" "${L[@]}" set "$work/def.img" "$work/vbad.img" altbootcmd=x >/dev/null
refuse "verify: altbootcmd changed" "altbootcmd" python3 "$plan" verify "$work/base.img" "$work/vbad.img" --expect default "${L[@]}"
python3 "$env" "${L[@]}" set "$work/def.img" "$work/vbad.img" boot_prefixes=/yakogl/standalone/\ /other >/dev/null
refuse "verify: boot_prefixes not as default sets it" "boot_prefixes" python3 "$plan" verify "$work/base.img" "$work/vbad.img" --expect default "${L[@]}"
refuse "verify nodefault: mp2s_* still set" "still set" python3 "$plan" verify "$work/def.img" "$work/def.img" --expect nodefault "${L[@]}"

# --- uboot_env.py is not modified by us (the tool only imports it)
check "tool does not write uboot_env.py" bash -c "! grep -n \"open(.*uboot_env\" '$plan'"

echo "mp2s_env_plan_selftest: $((n-fail)) ok, $fail failed"
[ $fail = 0 ]
