#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_written_vars_selftest.sh - the variables the DK standalone runner and
# tools/mp2s_env_plan.py actually WRITE, collected from behaviour:
#
#   (a) tools/board-stm32mp2-standalone.sh --dry-run for install,
#       default <valid image> 2, nodefault and uninstall on the fake board
#       (tools/mp2s_fakeboard.sh): every variable named in a fw_setenv line or
#       a `change:` line;
#   (b) mp2s_env_plan.py default and nodefault applied to
#       tools/testdata/mp2s-env/pristine.bin, the variable sets diffed with
#       tools/uboot_env.py.
#
# Asserts: every written name starts with mp2s_; none is in ST's default env
# fixture (driver/tools/mp2-standalone/st-env/stm32mp25-dk-initial-env.txt);
# the set equals the one section 7 of docs/notes/stm32mp257f-dk-standalone.md
# lists. Where the code writes boot_prefixes (a variable of ST's env, by
# design: section 3), the first two assertions FAIL and say so; that is the
# mismatch to report, the tools stay unchanged.
#
# No ssh, no board. Everything lives in $TMPDIR.
#
#   tools/mp2s_written_vars_selftest.sh
#
# Prints "mp2s_written_vars_selftest: N checks, F failed"; exit 0 only when F is 0.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
R=$HERE/board-stm32mp2-standalone.sh
NOTE=$REPO/docs/notes/stm32mp257f-dk-standalone.md
STENV=$REPO/driver/tools/mp2-standalone/st-env/stm32mp25-dk-initial-env.txt
PRISTINE=$HERE/testdata/mp2s-env/pristine.bin
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
N=0
F=0
ok() { N=$((N + 1)); echo "ok   $1"; }
bad() { N=$((N + 1)); F=$((F + 1)); echo "FAIL $1: $2"; }

unset BOARD_PERSON_PRESENT MP2_NO_WATCHDOG MP2_SUDO MP2_KNOWN_HOSTS MP2S_BOOT_MANIFEST
export MP2S_STATE_DIR=$W/state BOARD_LOCK=$W/board.lock LOCK_WAIT=5
"$HERE/mp2s_fakeboard.sh" init "$W/fb" >/dev/null || { echo "fake board init failed"; exit 1; }
eval "$("$HERE/mp2s_fakeboard.sh" shell "$W/fb")"
export BOARD_HOST=user@host   # a placeholder: the fake ssh ignores it

IMG=$W/img.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'gc_wdog_arm: armed'; } >"$IMG"
python3 -I "$HERE/mp2s_image_check.py" "$IMG" >/dev/null 2>&1 || { echo "test image fails the image check"; exit 1; }

# ---- (a) the runner's dry runs --------------------------------------------------------
: >"$W/a.txt"
for v in install "default $IMG 2" nodefault uninstall; do
    # shellcheck disable=SC2086
    "$R" --dry-run $v >"$W/dry.txt" 2>&1 || bad "dry-run $v" "rc $?: $(tail -3 "$W/dry.txt")"
    cat "$W/dry.txt" >>"$W/a.txt"
done
# names. The runner never calls fw_setenv: it writes the environment with one dd of
# a copy planned by mp2s_env_plan.py, and says what the copy holds in the dd line's
# comment ("NAME=value ..." for set, "deletes A, B and C", "NAME back to"). Both
# the fw_setenv / `change:` forms and those comments are read.
python3 -I - "$W/a.txt" >"$W/a.names" <<'PY'
import re, sys
ID = r"[A-Za-z_][A-Za-z0-9_]*"
names = set()
for ln in open(sys.argv[1], errors="replace"):
    ln = ln.rstrip("\n")
    m = re.search(r"fw_setenv\s+(?:-s\s+-\s*)?(.*)", ln)
    if m:
        rest = m.group(1)
        if "#" in rest:
            for w in rest.split("#", 1)[1].split():
                names.add(w.split("=")[0])
            rest = rest.split("#", 1)[0]
        for w in rest.split():
            if re.fullmatch(ID, w):
                names.add(w)
                break
            if re.match(ID + "=", w):
                names.add(w.split("=")[0])
    m = re.search(r"\bchange:\s*(.*)", ln)
    if m:
        for w in m.group(1).split():
            if re.match(ID + "=", w):
                names.add(w.split("=")[0])
            elif re.fullmatch(ID, w):
                names.add(w)
                break
    m = re.search(r"\bof=/dev/\S+.*?#\s*<(.*)", ln)
    if m:
        c = m.group(1)
        names.update(re.findall(r"(?<![A-Za-z0-9_])(" + ID + r")=", c))
        for d in re.findall(r"deletes ([^;]*)", c):
            names.update(re.findall(ID, d.replace(" and ", " ")))
        names.update(re.findall(r"(" + ID + r") back to", c))
for n in sorted(names):
    print(n)
PY
[ -s "$W/a.names" ] && ok "(a) the dry runs name written variables: $(tr '\n' ' ' <"$W/a.names")" || bad "(a) written names" "none found in fw_setenv / change: lines"

# ---- (b) env_plan on the pristine fixture ---------------------------------------------
L=(--redundant --size 0x2000)
vars_of() { python3 -I "$HERE/uboot_env.py" "${L[@]}" print "$1" 2>/dev/null | sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p' | sort -u; }
: >"$W/b.names"
if python3 -I "$HERE/mp2s_env_plan.py" default "$PRISTINE" "$W/def.bin" --count 2 --record "$W/rec" "${L[@]}" >"$W/def.out" 2>&1; then
    python3 -I "$HERE/uboot_env.py" "${L[@]}" diff "$PRISTINE" "$W/def.bin" >"$W/def.diff" 2>&1
    vars_of "$PRISTINE" >"$W/p.v"; vars_of "$W/def.bin" >"$W/d.v"
    comm -13 "$W/p.v" "$W/d.v" >>"$W/b.names"
    # changed variables (same name, new value): the diff lists them too
    sed -n 's/^\(changed\|added\|removed\) *\([A-Za-z_][A-Za-z0-9_]*\).*/\2/p' "$W/def.diff" >>"$W/b.names"
    ok "(b) env_plan default applied to pristine.bin"
    if python3 -I "$HERE/mp2s_env_plan.py" nodefault "$W/def.bin" "$W/nod.bin" --record "$W/rec" "${L[@]}" >"$W/nod.out" 2>&1; then
        python3 -I "$HERE/uboot_env.py" "${L[@]}" diff "$W/def.bin" "$W/nod.bin" >"$W/nod.diff" 2>&1
        vars_of "$W/nod.bin" >"$W/n.v"
        comm -23 "$W/d.v" "$W/n.v" >>"$W/b.names"
        sed -n 's/^\(changed\|added\|removed\) *\([A-Za-z_][A-Za-z0-9_]*\).*/\2/p' "$W/nod.diff" >>"$W/b.names"
        ok "(b) env_plan nodefault applied to the default's result"
    else bad "(b) env_plan nodefault" "$(tail -3 "$W/nod.out")"; fi
else bad "(b) env_plan default" "$(tail -3 "$W/def.out")"; fi
sort -u "$W/b.names" -o "$W/b.names"
[ -s "$W/b.names" ] && ok "(b) env_plan writes: $(tr '\n' ' ' <"$W/b.names")" || bad "(b) written names" "the diffs name no variable"

# ---- assertions -----------------------------------------------------------------------
sort -u "$W/a.names" "$W/b.names" >"$W/all.names"
cmp -s "$W/a.names" "$W/b.names" && ok "the runner's and env_plan's written sets are equal" ||
    bad "runner vs env_plan" "runner: $(tr '\n' ' ' <"$W/a.names") env_plan: $(tr '\n' ' ' <"$W/b.names")"

pre=$(grep -v '^mp2s_' "$W/all.names" | tr '\n' ' ')
[ -z "$pre" ] && ok "every written name starts with mp2s_" || bad "mp2s_ prefix" "written, not mp2s_*: $pre"

[ -r "$STENV" ] || { bad "ST fixture" "unreadable: $STENV"; echo "mp2s_written_vars_selftest: $N checks, $F failed"; exit 1; }
sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p' "$STENV" | sort -u >"$W/st.names"
clash=$(comm -12 "$W/all.names" "$W/st.names" | tr '\n' ' ')
[ -z "$clash" ] && ok "no written name is in ST's default env fixture" || bad "ST's env" "written and present in ST's fixture: $clash"

# section 7: the variables the script/runner write, from the contract text
python3 -I - "$NOTE" >"$W/note.names" <<'PY'
import re, sys
t = open(sys.argv[1]).read()
sec = t.split("## 7. The interface contract", 1)[1].split("\n## ", 1)[0]
names = set()
# the default verb's row: "one environment write: `mp2s_ver=1`, `mp2s_left=...`, ..."
for ln in sec.splitlines():
    if ln.startswith("| `default <image>"):
        seg = ln.split("one environment write:", 1)[1].split(", as one", 1)[0]
        names |= set(re.findall(r"`([A-Za-z_][A-Za-z0-9_]*)=", seg))
for n in sorted(names):
    print(n)
PY
[ -s "$W/note.names" ] && ok "section 7 lists: $(tr '\n' ' ' <"$W/note.names")" || bad "section 7" "no variable list found in the default row"
if cmp -s "$W/all.names" "$W/note.names"; then ok "the written set equals section 7's list"
else bad "section 7" "written: $(tr '\n' ' ' <"$W/all.names") note: $(tr '\n' ' ' <"$W/note.names")"; fi

echo "mp2s_written_vars_selftest: $N checks, $F failed"
[ "$F" = 0 ]
