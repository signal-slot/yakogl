#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# selftest.sh: host check of the DK standalone script, no board, no network.
#   - build.sh builds boot.scr.uimg and prints its sha256, which equals the
#     file's; two builds give the same bytes;
#   - the uImage header is parsed here in Python (magic, both CRCs, os linux,
#     arch arm64, type script, no compression, the length table, the script
#     bytes equal mp2-standalone.cmd). With mkimage present (on PATH, or
#     MKIMAGE=/path) "mkimage -l" must read it too and mkimage's build must
#     equal uboot_script.py's; without it that is counted as skipped, and
#     MP2_REQUIRE_MKIMAGE=1 fails it;
#   - the lint of docs/notes/stm32mp257f-dk-standalone.md section 7: allowed
#     commands only; setenv of mp2s_* and boot_prefixes only; boot_prefixes
#     set only from ${mp2s_prefixes}; saveenv exactly twice, after the
#     decrement and in the used-up branch; the forbidden tokens; and the order
#     of section 3 (undo first, save before any load, go last);
#   - every "mp2s: " line of the script is one of section 7's console lines,
#     and each of those lines is printed by the script;
#   - mp2s_load and mp2s_window equal ../uboot-mp2/mp2-mmc.cmd's;
#   - negative cases: mutated copies of the script that the checks must refuse.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
T=$(mktemp -d "${TMPDIR:-$PWD}/mp2-standalone-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0
skip=0
skipped=()
ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; }
expect_ok() { local what=$1; shift; if "$@" > "$T/out" 2>&1; then ok "$what"; else bad "$what"; sed 's/^/     /' "$T/out"; fi; }
# A refusal is reported with its first reason, so a case refused for the
# wrong reason shows.
expect_fail() { local what=$1; shift; if "$@" > "$T/out" 2>&1; then bad "$what (accepted)"; else ok "$what: $(head -n 1 "$T/out")"; fi; }

CMD="$here/mp2-standalone.cmd"
NOTE="$root/docs/notes/stm32mp257f-dk-standalone.md"
MMC="$here/../uboot-mp2/mp2-mmc.cmd"

cat > "$T/uimage.py" <<'PY'
"""uimage.py IMAGE SCRIPT: the legacy uImage header of a script, parsed here.

Exit 1 with one line per problem. Independent of tools/uboot_script.py."""
import struct
import sys
import zlib

img = open(sys.argv[1], "rb").read()
want = open(sys.argv[2], "rb").read()
bad = []
if len(img) < 64:
    print("shorter than the 64-byte header")
    sys.exit(1)
(magic, hcrc, _time, size, load, ep, dcrc, os_, arch, typ, comp,
 name) = struct.unpack(">7I4B32s", img[:64])
hdr = img[:4] + b"\0\0\0\0" + img[8:64]
data = img[64:]
if magic != 0x27051956:
    bad.append("magic 0x%08x" % magic)
if zlib.crc32(hdr) & 0xffffffff != hcrc:
    bad.append("header CRC")
if size != len(data):
    bad.append("ih_size %d, data %d bytes" % (size, len(data)))
if zlib.crc32(data) & 0xffffffff != dcrc:
    bad.append("data CRC")
if (os_, arch, typ, comp) != (5, 22, 6, 0):
    bad.append("os/arch/type/comp %d/%d/%d/%d, want 5 (linux)/22 (arm64)/6 (script)/0 (none)" % (os_, arch, typ, comp))
if (load, ep) != (0, 0):
    bad.append("load/ep 0x%x/0x%x, want 0/0" % (load, ep))
if name.rstrip(b"\0") != b"mp2-standalone":
    bad.append("name %r" % name)
if len(data) < 8:
    bad.append("no length table")
else:
    n, end = struct.unpack(">II", data[:8])
    if end != 0 or n != len(data) - 8:
        bad.append("length table %d, %d for %d script bytes" % (n, end, len(data) - 8))
    elif data[8:] != want:
        bad.append("the script bytes differ from %s" % sys.argv[2])
print("\n".join(bad))
sys.exit(1 if bad else 0)
PY

cat > "$T/lint.py" <<'PY'
"""lint.py SCRIPT NOTE MMC: section 7's lint, section 3's order, the console lines.

Exit 1 with one line per problem."""
import re
import shlex
import sys

path, note_path, mmc_path = sys.argv[1:4]
src = open(path, encoding="utf-8").read()
problems = []

ALLOWED = {"echo", "setenv", "test", "itest", "setexpr", "size", "load",
           "saveenv", "dcache", "icache", "go", "exit"}
CONTROL = {"if", "then", "elif", "else", "fi"}
SEP = {";", "&&", "||"}
SETENV_OK = re.compile(r"^(mp2s_[a-z0-9_]+|boot_prefixes)$")
# Section 3: the variables the runner and the count own; anything else of
# ours lives in RAM and must come after the last save.
SAVED = {"mp2s_left", "mp2s_prefixes", "mp2s_ver", "boot_prefixes"}
FILE = "/yakogl/standalone/default.bin"
PART = ["${devtype}", "${devnum}:${distro_bootpart}"]

# ---- forbidden tokens, over the whole file (comments too) but the SPDX lines
FORBIDDEN = [("altbootcmd", r"altbootcmd"), ("bootcmd", r"bootcmd"),
             ("bootcount", r"bootcount"), ("bootlimit", r"bootlimit"),
             ("upgrade_available", r"upgrade_available"),
             ("boot_targets", r"boot_targets"), ("preboot", r"preboot"),
             ("mp2z_", r"mp2z_"), ("yako_", r"yako_"),
             ("env default", r"\benv\s+default\b"), ("eraseenv", r"eraseenv"),
             ("mmc write", r"\bmmc\s+write\b"), ("ext4write", r"ext4write"),
             ("fatwrite", r"fatwrite"), ("gpt", r"\bgpt\b"), ("run", r"\brun\b")]
for n, line in enumerate(src.splitlines(), 1):
    if line.startswith("# SPDX-"):
        continue
    for name, pat in FORBIDDEN:
        if re.search(pat, line):
            problems.append("line %d: contains %r" % (n, name))

# ---- statements: (line, tokens), the control words stripped
stmts = []
for n, line in enumerate(src.splitlines(), 1):
    if not line.strip() or line.lstrip().startswith("#"):
        continue
    try:
        lex = shlex.shlex(line, posix=True, punctuation_chars=";&|")
        lex.whitespace_split = True
        words = list(lex)
    except ValueError as e:
        problems.append("line %d: %s" % (n, e))
        continue
    cur = []
    for w in words + [";"]:
        if w in SEP:
            while cur and cur[0] in CONTROL:
                cur = cur[1:]
            if cur:
                stmts.append((n, cur))
            cur = []
        else:
            cur.append(w)

for n, s in stmts:
    cmd = s[0]
    base = cmd.split(".")[0]
    if base not in ALLOWED or (cmd != base and cmd not in ("itest.l", "setexpr.l")):
        problems.append("line %d: command %r is not allowed" % (n, cmd))
        continue
    if cmd == "echo":
        if len(s) != 2 or not s[1].startswith("mp2s: ") or "\n" in s[1]:
            problems.append("line %d: echo is not one 'mp2s: ' line" % n)
    elif cmd == "setenv":
        if len(s) < 2 or not SETENV_OK.match(s[1]):
            problems.append("line %d: setenv of %r (only mp2s_* and boot_prefixes)" % (n, s[1:2]))
        elif s[1] == "boot_prefixes" and s[2:] != ["${mp2s_prefixes}"]:
            problems.append("line %d: boot_prefixes set to %r, only ${mp2s_prefixes}" % (n, " ".join(s[2:])))
    elif base == "setexpr":
        if len(s) < 2 or not re.match(r"^mp2s_[a-z0-9_]+$", s[1]):
            problems.append("line %d: setexpr of %r (only mp2s_*)" % (n, s[1:2]))
    elif cmd in ("dcache", "icache"):
        if s[1:] not in (["off"], ["flush"]):
            problems.append("line %d: %s %s" % (n, cmd, " ".join(s[1:])))
    elif cmd in ("size", "load"):
        want = PART + (["${mp2s_load}"] if cmd == "load" else []) + [FILE]
        if s[1:] != want:
            problems.append("line %d: %s %s, want %s" % (n, cmd, " ".join(s[1:]), " ".join(want)))
    elif cmd == "go":
        if s[1:] != ["${mp2s_load}"]:
            problems.append("line %d: go does not jump to ${mp2s_load}" % n)
    elif cmd in ("saveenv", "exit"):
        if len(s) != 1:
            problems.append("line %d: %s with arguments" % (n, cmd))

text = [" ".join(s) for _, s in stmts]


def where(pred):
    return [i for i, s in enumerate(text) if pred(s)]


# ---- order (section 3)
UNDO = "echo mp2s: undo: setenv mp2s_left 0; saveenv; boot   (or a PC: delete yakogl/standalone/boot.scr.uimg)"
if not text or text[0] != UNDO:
    problems.append("the first statement is not the undo echo")
saves = where(lambda s: s == "saveenv")
if len(saves) != 2:
    problems.append("%d saveenv statements, want exactly 2" % len(saves))
else:
    a, b = saves
    if text[a - 1] != "setexpr mp2s_left ${mp2s_left} - 1":
        problems.append("the first saveenv does not follow the decrement")
    if not (a + 1 < len(text) and
            text[a + 1] == "echo mp2s: default %s, boots left ${mp2s_left}" % FILE):
        problems.append("'boots left' does not follow the first saveenv")
    if where(lambda s: s.startswith("echo mp2s: default %s, boots left" % FILE)) != [a + 1]:
        problems.append("'boots left' is printed somewhere else than after the save")
    tail = ["setenv boot_prefixes ${mp2s_prefixes}", "setenv mp2s_left",
            "setenv mp2s_prefixes", "setenv mp2s_ver", "saveenv", "exit"]
    if text[b - 4:b + 2] != tail:
        problems.append("the used-up branch is not %r" % tail)
    if text[b - 5:b - 4] != ["echo mp2s: default used up, booting Linux"]:
        problems.append("the used-up branch does not print 'default used up' first")
    for i in range(b):
        s = stmts[i][1]
        if s[0].split(".")[0] in ("setenv", "setexpr") and len(s) > 1 and s[1] not in SAVED:
            problems.append("line %d: %s set before the last saveenv (it would be saved)" % (stmts[i][0], s[1]))
        if s[0] in ("size", "load", "go", "dcache", "icache"):
            problems.append("line %d: %s before the last saveenv" % (stmts[i][0], s[0]))
gos = where(lambda s: s.startswith("go "))
if len(gos) != 1:
    problems.append("%d go statements, want 1" % len(gos))
else:
    g = gos[0]
    want = ["echo mp2s: go ${mp2s_load}", "dcache flush", "dcache off", "icache off",
            "go ${mp2s_load}", "echo mp2s: the image returned, booting Linux", "exit"]
    if text[g - 4:] != want:
        problems.append("the end of the script is not %r" % want)
    for i in where(lambda s: s.split()[0] in ("size", "load")):
        if i > g:
            problems.append("a %s after go" % text[i].split()[0])

# ---- the window, equal to the one-shot's
vals = {}
for var in ("mp2s_load", "mp2s_window"):
    m = re.findall(r"^setenv %s (\S+)$" % var, src, re.M)
    if len(m) != 1:
        problems.append("%s is set %d times, want once" % (var, len(m)))
    else:
        vals[var] = m[0]
mmc = open(mmc_path, encoding="utf-8").read()
for ours, theirs, val in (("mp2s_load", "mp2z" + "_load", "0xa0000000"),
                          ("mp2s_window", "mp2z" + "_window", "0x10000000")):
    m = re.findall(r"^setenv %s (\S+)$" % theirs, mmc, re.M)
    if vals.get(ours) != val or m != [val]:
        problems.append("%s is %r, %s in mp2-mmc.cmd %r, the contract %s" % (ours, vals.get(ours), theirs, m, val))

# ---- the console lines of section 7
note = open(note_path, encoding="utf-8").read()
m = re.search(r"^### Console lines\n.*?^```\n(.*?)^```", note, re.M | re.S)
if not m:
    problems.append("no console-line block under '### Console lines' in the note")
    listed = []
else:
    listed = [re.split(r"\s{2,}<-", l)[0].rstrip() for l in m.group(1).splitlines() if l.strip()]


def pattern(line):
    parts = re.split(r"<[^>]+>", line)
    return re.compile("^" + ".+".join(re.escape(p) for p in parts) + "$")


pats = [(l, pattern(l)) for l in listed]
used = set()
for (n, s) in stmts:
    if s[0] != "echo" or len(s) != 2:
        continue
    line = s[1].replace("${mp2s_load}", vals.get("mp2s_load", "?")) \
               .replace("${mp2s_window}", vals.get("mp2s_window", "?"))
    line = re.sub(r"\$\{[a-z0-9_]+\}", "V", line)
    hit = [l for l, p in pats if p.match(line)]
    if not hit:
        problems.append("line %d: %r is not a console line of section 7" % (n, s[1]))
    used.update(hit)
for l in listed:
    if l not in used:
        problems.append("section 7's line %r is never printed" % l)

for p in problems:
    print(p)
sys.exit(1 if problems else 0)
PY

PY3="python3 -I"
lint() { $PY3 "$T/lint.py" "$1" "$NOTE" "$MMC"; }

# ---- 1. build.sh: the image and its sha256
export SOURCE_DATE_EPOCH=1700000000
expect_ok "build.sh (uboot_script.py) runs" env MP2_NO_MKIMAGE=1 bash "$here/build.sh" "$T/py"
line=$(tail -n 1 "$T/out")
sum=$(sha256sum "$T/py/boot.scr.uimg" | cut -d' ' -f1)
if [ "$line" = "build.sh: $T/py/boot.scr.uimg sha256 $sum (via uboot_script.py)" ]; then
	ok "build.sh prints the image's sha256"
else
	bad "build.sh prints '$line', want the sha256 $sum"
fi
MP2_NO_MKIMAGE=1 bash "$here/build.sh" "$T/py2" > /dev/null
expect_ok "two builds give the same bytes" cmp "$T/py/boot.scr.uimg" "$T/py2/boot.scr.uimg"
expect_ok "uImage header parsed in Python (CRCs, arm64 script, the script bytes)" \
	$PY3 "$T/uimage.py" "$T/py/boot.scr.uimg" "$CMD"
expect_ok "tools/uboot_script.py check reads it" $PY3 "$root/tools/uboot_script.py" check "$T/py/boot.scr.uimg"
# Without SOURCE_DATE_EPOCH the header time is the commit's: still reproducible.
(unset SOURCE_DATE_EPOCH; MP2_NO_MKIMAGE=1 bash "$here/build.sh" "$T/c1" > /dev/null; sleep 1
	MP2_NO_MKIMAGE=1 bash "$here/build.sh" "$T/c2" > /dev/null)
expect_ok "without SOURCE_DATE_EPOCH two builds still give the same bytes" cmp "$T/c1/boot.scr.uimg" "$T/c2/boot.scr.uimg"

# ---- 2. mkimage, when present
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
	expect_ok "build.sh (mkimage) runs" bash "$here/build.sh" "$T/mk"
	expect_ok "build.sh used mkimage" grep -q '(via mkimage)$' "$T/out"
	expect_ok "mkimage -l reads it as an AArch64 script" \
		bash -c 'mkimage -l "$1" | grep -q "AArch64 Linux Script"' _ "$T/mk/boot.scr.uimg"
	expect_ok "mkimage and uboot_script.py give identical bytes" cmp "$T/mk/boot.scr.uimg" "$T/py/boot.scr.uimg"
elif [ -n "${MP2_REQUIRE_MKIMAGE:-}" ]; then
	bad "mkimage: not on PATH and MP2_REQUIRE_MKIMAGE is set"
else
	skip=$((skip + 1))
	skipped+=("mkimage not on PATH: the header was checked by the Python parser only")
	echo "skip mkimage -l and the byte comparison: mkimage not on PATH (the header was parsed in Python)"
fi

# ---- 3. the lint, on the script as built
$PY3 "$root/tools/uboot_script.py" extract "$T/py/boot.scr.uimg" -o "$T/ext.cmd"
expect_ok "extracted script equals the source" cmp "$CMD" "$T/ext.cmd"
expect_ok "lint: section 7 (commands, variables, saveenv x2, tokens, order, console lines)" lint "$T/ext.cmd"

# ---- 4. negative cases: each mutation must be refused
mut() { # name, sed expression
	sed "$2" "$CMD" > "$T/neg.cmd"
	if cmp -s "$CMD" "$T/neg.cmd"; then bad "negative case '$1' changed nothing"; return; fi
	expect_fail "lint refuses: $1" lint "$T/neg.cmd"
}
ins() { # name, line inserted before the go echo
	mut "$1" "/^echo \"mp2s: go /i $2"
}
# the forbidden tokens of section 7
ins "bootcmd" "setenv bootcmd run bootcmd_stm32mp"
ins "altbootcmd" "setenv altbootcmd x"
ins "bootcount" "setenv bootcount 0"
ins "bootlimit" "setenv bootlimit 3"
ins "upgrade_available" "setenv upgrade_available 0"
ins "boot_targets" "setenv boot_targets mmc0"
ins "preboot" "setenv preboot x"
ins "mp2z_" "setenv mp2z_file /x"
ins "yako_" "setenv yako_once 1"
ins "env default" "env default -a"
ins "eraseenv" "eraseenv"
ins "mmc write" "mmc write 0xa0000000 0 1"
ins "ext4write" "ext4write mmc 0:8 0xa0000000 /x 4"
ins "fatwrite" "fatwrite mmc 0:8 0xa0000000 x 4"
ins "gpt" "gpt write mmc 0 x"
ins "run" "run mp2s_next"
ins "a forbidden token inside an echo" 'echo "mp2s: refused: bootcount, booting Linux"'
ins "a forbidden token in a comment" "# then run it"
# commands and variables
ins "a command that is not listed" "bootm 0xa0000000"
ins "env save" "env save"
ins "setenv of a stock variable" "setenv bootdelay 5"
ins "setenv inside an if" "if test 1 -eq 1; then setenv boot_syslinux_conf x; fi"
ins "setexpr of a stock variable" "setexpr filesize 0 + 1"
ins "a third saveenv" "saveenv"
mut "boot_prefixes from a literal" 's/^\tsetenv boot_prefixes "${mp2s_prefixes}"$/\tsetenv boot_prefixes "\/mmc0_"/'
mut "boot_prefixes with our prefix kept" 's/^\tsetenv boot_prefixes "${mp2s_prefixes}"$/\tsetenv boot_prefixes "\/yakogl\/standalone\/ ${mp2s_prefixes}"/'
mut "only one saveenv (the used-up one gone)" '/^\tsaveenv$/d'
mut "the count saved without a check (no if)" 's/^\tif saveenv; then$/\tsaveenv; if test 1 -eq 1; then/'
mut "the used-up branch not deleting mp2s_ver" '/^\tsetenv mp2s_ver$/d'
# order
mut "the undo no longer first" 's/^echo "mp2s: undo: setenv mp2s_left 0; saveenv; boot   (or a PC: /echo "mp2s: undo: setenv mp2s_left 0; saveenv; boot (or a PC: /'
mut "boots left before the save" 's/^\tsetexpr mp2s_left ${mp2s_left} - 1$/\tsetexpr mp2s_left ${mp2s_left} - 1\n\techo "mp2s: default \/yakogl\/standalone\/default.bin, boots left ${mp2s_left}"/'
mut "a RAM variable set before the save" 's/^\tsetexpr mp2s_left ${mp2s_left} - 1$/\tsetenv mp2s_ok 0\n&/'
mut "a load before the save" 's/^\tsetexpr mp2s_left ${mp2s_left} - 1$/\tload ${devtype} ${devnum}:${distro_bootpart} ${mp2s_load} \/yakogl\/standalone\/default.bin\n&/'
mut "no cache flush before go" 's/^dcache flush; dcache off/dcache off/'
mut "go to another address" 's/; go ${mp2s_load}$/; go 0xa0000040/'
mut "no exit after the image returned" '$d'
mut "a different mp2s_load" 's/^setenv mp2s_load 0xa0000000$/setenv mp2s_load 0xa1000000/'
mut "a different mp2s_window" 's/^setenv mp2s_window 0x10000000$/setenv mp2s_window 0x20000000/'
mut "load from another file" 's/^\(if load .*\) \/yakogl\/standalone\/default.bin; /\1 \/yakogl\/x.bin; /'
# console lines
mut "an echo without the prefix" 's/echo "mp2s: loaded/echo "loaded/'
mut "a console line that section 7 does not list" 's/"mp2s: no default set, back to the distro loop"/"mp2s: no default set"/'
mut "a refusal that does not end in booting Linux" 's/"mp2s: refused: load failed, booting Linux"/"mp2s: refused: load failed"/'
mut "section 7's 'forever' line never printed" 's/default.bin, forever"/default.bin, for ever"/'

# the image itself: a corrupted uImage is refused by the parser
cp "$T/py/boot.scr.uimg" "$T/neg.scr"
printf '\377' | dd of="$T/neg.scr" bs=1 seek=100 conv=notrunc status=none
expect_fail "the header parser refuses a corrupted data byte" $PY3 "$T/uimage.py" "$T/neg.scr" "$CMD"
$PY3 "$root/tools/uboot_script.py" build "$CMD" -o "$T/neg.scr" --name mp2-standalone --arch arm > /dev/null
expect_fail "the header parser refuses an arm (not arm64) header" $PY3 "$T/uimage.py" "$T/neg.scr" "$CMD"

summary="mp2-standalone selftest: $pass ok, $fail failed, $skip skipped"
if [ "$skip" -gt 0 ]; then
	summary="$summary ($(printf '%s; ' "${skipped[@]}" | sed 's/; $//'))"
fi
echo "$summary"
[ "$fail" -eq 0 ]
