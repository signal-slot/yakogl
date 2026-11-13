#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# selftest.sh: host check of the DK one-shot scripts, no board, no network.
#   - build.sh builds both scripts; tools/uboot_script.py check/extract read
#     them back, byte for byte;
#   - with mkimage present (on PATH, or MKIMAGE=/path), its output equals
#     uboot_script.py's (with SOURCE_DATE_EPOCH set); without it the check is
#     counted as skipped in the summary, and MP2_REQUIRE_MKIMAGE=1 fails it;
#   - lint: only allowed commands, no write of the environment or of a disk,
#     setenv only of mp2z_* (and autoload), every echo starts with "mp2z: ",
#     the undo is the first line, "mp2z: go" the last before go;
#   - mp2z_load and mp2z_window equal the DK DTS's zephyr,sram and
#     tools/board-stm32mp2.sh's window;
#   - negative cases: the lint and the DTS check fail on mutated scripts.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
T=$(mktemp -d "${TMPDIR:-$PWD}/uboot-mp2-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0
skip=0
skipped=()
ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; }
expect_ok() { local what=$1; shift; if "$@" > "$T/out" 2>&1; then ok "$what"; else bad "$what"; sed 's/^/     /' "$T/out"; fi; }
expect_fail() { local what=$1; shift; if "$@" > "$T/out" 2>&1; then bad "$what (accepted)"; else ok "$what"; fi; }

cat > "$T/lint.py" <<'PY'
"""lint.py KIND SCRIPT: KIND is mmc or net. Exit 1 with one line per problem."""
import re
import shlex
import sys

kind, path = sys.argv[1], sys.argv[2]
src = open(path, encoding="utf-8").read()
problems = []

ALLOWED = {"echo", "setenv", "test", "itest", "itest.l", "setexpr", "setexpr.l",
           "dcache", "icache", "go", "exit"}
ALLOWED |= {"ext4size", "ext4load"} if kind == "mmc" else {"dhcp", "tftpboot"}
CONTROL = {"if", "then", "else", "fi"}
SETENV_OK = re.compile(r"^(mp2z_[a-z0-9_]+|autoload)$")

stmts = []          # (line number, tokens)
for n, line in enumerate(src.splitlines(), 1):
    if line.strip().startswith("#") or not line.strip():
        continue
    lex = shlex.shlex(line, posix=True, punctuation_chars=";")
    lex.whitespace_split = True
    toks, cur = [], []
    for t in lex:
        if t == ";":
            toks.append(cur)
            cur = []
        else:
            cur.append(t)
    toks.append(cur)
    for s in toks:
        while s and s[0] in CONTROL:
            s = s[1:]
        if s:
            stmts.append((n, s))

for n, s in stmts:
    cmd = s[0]
    if cmd not in ALLOWED:
        problems.append("line %d: command %r is not allowed" % (n, cmd))
        continue
    if cmd != "echo" and any("bootcmd" in t for t in s):
        problems.append("line %d: %s names bootcmd" % (n, cmd))
    if cmd == "echo":
        if len(s) != 2 or not s[1].startswith("mp2z: "):
            problems.append("line %d: echo does not start with 'mp2z: '" % n)
    elif cmd == "setenv":
        if len(s) < 2 or not SETENV_OK.match(s[1]):
            problems.append("line %d: setenv of %r (only mp2z_* and autoload)" % (n, s[1:2]))
    elif cmd in ("dcache", "icache"):
        if s[1:] not in (["off"], ["flush"]):
            problems.append("line %d: %s %s" % (n, cmd, " ".join(s[1:])))
    elif cmd == "go":
        if s[1:] != ["${mp2z_load}"]:
            problems.append("line %d: go does not jump to ${mp2z_load}" % n)

# The raw text: no staged name the distro loop could take, no board address.
for bad in ("extlinux", "boot.scr", "saveenv", "mmc write", "ext4write", "fatwrite", "gpt "):
    for n, line in enumerate(src.splitlines(), 1):
        if bad in line and not line.lstrip().startswith("#"):
            problems.append("line %d: contains %r" % (n, bad))
if re.search(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b", src):
    problems.append("an IP address in the script")

# Order: the undo is the first statement, "mp2z: go" the last one before go.
if not stmts or stmts[0][1][:1] != ["echo"] or \
        not stmts[0][1][1].startswith("mp2z: undo: nothing persistent; run bootcmd boots ST Linux"):
    problems.append("the first statement is not the undo echo")
gos = [i for i, (_, s) in enumerate(stmts) if s[0] == "go"]
if len(gos) != 1:
    problems.append("%d go statements, want 1" % len(gos))
else:
    g = gos[0]
    # the jump: the echo, then dcache flush; dcache off; icache off; go
    tail = [" ".join(s) for _, s in stmts[g - 4:g + 1]]
    want = ["echo mp2z: go ${mp2z_load}", "dcache flush", "dcache off", "icache off", "go ${mp2z_load}"]
    if tail != want:
        problems.append("the jump is not %r" % want)
    if any(s[0] == "exit" for _, s in stmts[g:]):
        problems.append("exit after go")

# Contract variables set to the shared values.
for var, val in (("mp2z_load", "0xa0000000"), ("mp2z_window", "0x10000000")):
    m = re.findall(r"^setenv %s (\S+)$" % var, src, re.M)
    if m != [val]:
        problems.append("%s is set to %r, the contract says %s" % (var, m, val))

for p in problems:
    print(p)
sys.exit(1 if problems else 0)
PY

cat > "$T/dts.py" <<'PY'
"""dts.py SCRIPT DTS BOARD_SH: mp2z_load/window against zephyr,sram and the window."""
import glob
import re
import sys

script, dts_glob, board = sys.argv[1:4]
src = open(script, encoding="utf-8").read()
dts = open(sorted(glob.glob(dts_glob))[0], encoding="utf-8").read()
sh = open(board, encoding="utf-8").read()

def num(x):
    return int(x, 0)

m = re.search(r"zephyr,sram\s*=\s*&(\w+)\s*;", dts)
label = m.group(1)
m = re.search(label + r":\s*\w+@\w+\s*\{\s*reg\s*=\s*<\s*(0x[0-9a-fA-F]+)\s+DT_SIZE_M\((\d+)\)\s*>", dts)
base, size = num(m.group(1)), int(m.group(2)) << 20
a = re.search(r"^YAKO_ADDR=(0x[0-9a-fA-F]+)", sh, re.M)
e = re.search(r"^LOAD_END=(0x[0-9a-fA-F]+)", sh, re.M)
load = num(re.search(r"^setenv mp2z_load (\S+)$", src, re.M).group(1))
win = num(re.search(r"^setenv mp2z_window (\S+)$", src, re.M).group(1))
bad = []
if load != base:
    bad.append("mp2z_load 0x%x, DTS zephyr,sram base 0x%x" % (load, base))
if win != size:
    bad.append("mp2z_window 0x%x, DTS zephyr,sram size 0x%x" % (win, size))
if load != num(a.group(1)):
    bad.append("mp2z_load 0x%x, board-stm32mp2.sh YAKO_ADDR %s" % (load, a.group(1)))
if load + win != num(e.group(1)):
    bad.append("mp2z_load+window 0x%x, board-stm32mp2.sh LOAD_END %s" % (load + win, e.group(1)))
print("\n".join(bad))
sys.exit(1 if bad else 0)
PY

PY3="python3 -I"
DTS="$root/driver/boards/st/stm32mp257f_dk/*.dts"
BOARD_SH="$root/tools/board-stm32mp2.sh"
lint() { $PY3 "$T/lint.py" "$1" "$2"; }
dtscheck() { $PY3 "$T/dts.py" "$1" "$DTS" "$BOARD_SH"; }

# ---- 1. build, read back
export SOURCE_DATE_EPOCH=1700000000
MP2_NO_MKIMAGE=1 bash "$here/build.sh" "$T/py" > /dev/null
for k in mmc net; do
	cmd="$here/mp2-$k.cmd"
	expect_ok "$k: uboot_script.py check" $PY3 "$root/tools/uboot_script.py" check "$T/py/mp2-$k.scr"
	$PY3 "$root/tools/uboot_script.py" extract "$T/py/mp2-$k.scr" -o "$T/$k.ext"
	expect_ok "$k: extracted script equals the source" cmp "$cmd" "$T/$k.ext"
	expect_ok "$k: lint (allowed commands, undo first, go last)" lint "$k" "$T/$k.ext"
	expect_ok "$k: window equals the DTS and board-stm32mp2.sh" dtscheck "$T/$k.ext"
done

# ---- 2. mkimage, when present
# MKIMAGE=/path/to/mkimage names the tool (build.sh finds "mkimage" on PATH, so
# a link to it goes first on PATH). MP2_REQUIRE_MKIMAGE=1 makes its absence a failure.
have_mk=""
if [ -n "${MKIMAGE:-}" ]; then
	if [ -x "$MKIMAGE" ]; then
		mkdir "$T/mkbin"
		ln -s "$(readlink -f "$MKIMAGE")" "$T/mkbin/mkimage"
		export PATH="$T/mkbin:$PATH"
		have_mk=1
	else
		bad "MKIMAGE=$MKIMAGE is not an executable"
	fi
elif command -v mkimage > /dev/null; then
	have_mk=1
fi
if [ -n "$have_mk" ]; then
	bash "$here/build.sh" "$T/mk" > /dev/null
	for k in mmc net; do
		expect_ok "$k: mkimage and uboot_script.py give identical bytes" cmp "$T/mk/mp2-$k.scr" "$T/py/mp2-$k.scr"
	done
elif [ -n "${MP2_REQUIRE_MKIMAGE:-}" ]; then
	bad "mkimage comparison: mkimage not on PATH and MP2_REQUIRE_MKIMAGE is set"
else
	skip=$((skip + 1))
	skipped+=("mkimage not on PATH")
	echo "skip mkimage comparison: mkimage not on PATH"
fi

# ---- 3. negative cases: each mutation must be refused
for k in mmc net; do
	src="$here/mp2-$k.cmd"
	mut() { # name, sed expression
		sed "$2" "$src" > "$T/neg.cmd"
		if cmp -s "$src" "$T/neg.cmd"; then bad "$k: negative case '$1' changed nothing"; return; fi
		expect_fail "$k: lint refuses: $1" lint "$k" "$T/neg.cmd"
	}
	app() { # name, line appended before the go echo
		sed "/^echo \"mp2z: go /i $2" "$src" > "$T/neg.cmd"
		expect_fail "$k: lint refuses: $1" lint "$k" "$T/neg.cmd"
	}
	app "saveenv" "saveenv"
	app "env save" "env save"
	app "env write" "env write -f"
	app "env erase" "env erase"
	app "mmc write" "mmc write 0xa0000000 0 1"
	app "ext4write" "ext4write mmc 1:4 0xa0000000 /x 4"
	app "fatwrite" "fatwrite mmc 1:4 0xa0000000 x 4"
	app "gpt" "gpt write mmc 1 x"
	app "setenv of a stock variable" "setenv bootdelay 5"
	app "setenv bootcmd" "setenv bootcmd run x"
	app "run bootcmd" "run bootcmd"
	app "a command that is not listed" "bootm 0xa0000000"
	app "setenv inside an if" "if test 1 -eq 1; then setenv boot_targets mmc1; fi"
	mut "an echo without the prefix" 's/echo "mp2z: loaded/echo "loaded/'
	mut "the undo no longer first" 's/^echo "mp2z: undo: nothing persistent; run bootcmd boots ST Linux"$/echo "mp2z: undo: x"/'
	mut "go without its echo" 's/^echo "mp2z: go ${mp2z_load}"$/echo "mp2z: loaded"/'
	mut "no cache flush before go" 's/^dcache flush; dcache off/dcache off/'
	mut "go to another address" 's/^\(dcache flush.*; go \)${mp2z_load}$/\10xa0000040/'
	mut "a different mp2z_load" 's/^setenv mp2z_load 0xa0000000$/setenv mp2z_load 0xa1000000/'
	# the DTS check, on its own
	sed 's/^setenv mp2z_load 0xa0000000$/setenv mp2z_load 0xa1000000/' "$src" > "$T/neg.cmd"
	expect_fail "$k: DTS check refuses a different mp2z_load" dtscheck "$T/neg.cmd"
	sed 's/^setenv mp2z_window 0x10000000$/setenv mp2z_window 0x20000000/' "$src" > "$T/neg.cmd"
	expect_fail "$k: DTS check refuses a different mp2z_window" dtscheck "$T/neg.cmd"
done

# the script images themselves: a corrupted .scr is refused by uboot_script.py check
cp "$T/py/mp2-mmc.scr" "$T/neg.scr"
printf '\377' | dd of="$T/neg.scr" bs=1 seek=100 conv=notrunc status=none
expect_fail "a corrupted .scr is refused" $PY3 "$root/tools/uboot_script.py" check "$T/neg.scr"

# the two scripts keep the same checks of the image (diff of the shared tail)
tail_of() { sed -n '/^# The Image header magic/,$p' "$1"; }
expect_ok "both scripts carry the same image checks and jump" \
	cmp <(tail_of "$here/mp2-mmc.cmd") <(tail_of "$here/mp2-net.cmd")

summary="uboot-mp2 selftest: $pass ok, $fail failed, $skip skipped"
if [ "$skip" -gt 0 ]; then
	summary="$summary ($(printf '%s; ' "${skipped[@]}" | sed 's/; $//'))"
fi
echo "$summary"
[ "$fail" -eq 0 ]
