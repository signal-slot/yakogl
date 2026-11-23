#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_note_undo_selftest.sh - does section 5 of the design note
# (docs/notes/stm32mp257f-dk-standalone.md, "The undo") name the same things
# the runner changes and undoes?
#
# It drives tools/board-stm32mp2-standalone.sh --dry-run for install,
# `default <test image> 2`, nodefault and uninstall on the fake board
# (tools/mp2s_fakeboard.sh), and collects from each transcript
#   - the change set: the mp2s_* variables and /boot paths named by the
#     `DRY board$` lines that write (mkdir, tee, mv, rm, dd of=);
#   - the undo set: what the verb the UNDO line names (install <-> uninstall,
#     default -> nodefault) itself removes, deletes or recreates.
# It parses the undo table of section 5 (only mp2s_* names and /boot paths;
# a bare /boot is the compare target, a read, and is skipped) and asserts:
#   1. every change in a transcript is covered by its undo's transcript;
#   2. every change is named by the note, in a row that gives the undo from
#      Linux (the verb) and an entry for "without Linux";
#   3. the note names nothing the runner never changes.
# A path counts as named/covered when it or an ancestor is.
#
# No board, no ssh, no network. Everything lives in $TMPDIR.
#
#   tools/mp2s_note_undo_selftest.sh
#
# Prints "mp2s_note_undo_selftest: N checks, F failed"; exit 0 only when F is 0.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
R="$HERE/board-stm32mp2-standalone.sh"
NOTE=${MP2S_NOTE:-$REPO/docs/notes/stm32mp257f-dk-standalone.md}
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
[ -f "$NOTE" ] || { echo "note missing: $NOTE"; exit 1; }

dry() {   # NAME VERB...
    local name=$1
    shift
    "$R" --dry-run "$@" >"$W/$name.txt" 2>&1
    if [ $? = 0 ]; then ok "dry-run $name exits 0"; else bad "dry-run $name" "rc != 0: $(tail -3 "$W/$name.txt")"; fi
}
dry install install
dry default default "$IMG" 2
dry nodefault nodefault
dry uninstall uninstall

cat >"$W/check.py" <<'PY'
import re
import sys

note, w = sys.argv[1], sys.argv[2]
out = []


def ok(m):
    out.append("ok   " + m)


def bad(m, why):
    out.append("FAIL %s: %s" % (m, why))


def check(cond, m, why):
    (ok(m) if cond else bad(m, why))


VAR = re.compile(r"(?<![\w/])mp2s_[a-z]+(?![\w*])(?!\.(?:py|sh)\b)")
PATH = re.compile(r"(?<![\w])/boot(?:/[\w./-]*)?")
WRITER = re.compile(r"(?:^|[\s;&|])(mkdir|tee|mv|rm|fw_setenv|dd(?=\s[^#]*\bof=))\b")


def norm(p):
    p = p.rstrip("/.,;:)`'\"")
    return p[:-5] if p.endswith(".part") else p


def under(p, anc):
    return p == anc or p.startswith(anc + "/")


def parse(name):
    """-> (changes, removed paths, deleted vars, undo verb)"""
    changes, removed, deleted, undo = set(), set(), set(), None
    for ln in open("%s/%s.txt" % (w, name)):
        ln = ln.rstrip("\n")
        m = re.search(r"\bUNDO(?:, from Linux)?\b.*standalone\.sh (\w+)", ln)
        if m and undo is None:
            undo = m.group(1)
        if "nothing to undo: nodefault only removes" in ln:
            undo = undo or "(none)"
        m = re.match(r"DRY board\$ (.*)$", ln)
        if not m:
            continue
        code, _, comment = m.group(1).partition("   #")
        words = WRITER.findall(code)
        if not words:
            continue
        paths = {norm(p) for p in PATH.findall(code)} - {"/boot"}
        changes |= paths
        changes |= set(VAR.findall(comment))
        if "rm" in words:
            removed |= paths
        d = re.search(r"deletes (.*)$", comment)
        if d:
            deleted |= set(VAR.findall(d.group(1)))
    return changes, removed, deleted, undo


T = {n: parse(n) for n in ("install", "default", "nodefault", "uninstall")}
INVERSE = {"install": "uninstall", "default": "nodefault", "uninstall": "install"}

for n in ("install", "default", "uninstall"):
    check(bool(T[n][0]), "%s: the transcript has changes" % n, "no change line found")
    check(T[n][3] == INVERSE[n], "%s: the transcript's UNDO line names %s" % (n, INVERSE[n]),
          "names %r" % (T[n][3],))
check(T["nodefault"][3] == "(none)", "nodefault: the transcript says nothing to undo (it only removes)",
      "got %r" % (T["nodefault"][3],))

# 1. every change is covered by the undo verb's own transcript
for n in ("install", "default", "uninstall"):
    inv = T[INVERSE[n]]
    # a variable the transcript only deletes (uninstall runs nodefault's edit) is a removal: no undo
    for x in sorted(T[n][0] - T[n][2]):
        if x.startswith("mp2s_"):
            cov = x in inv[2]
        elif n == "uninstall":
            cov = any(under(x, a) for a in inv[0])      # install recreates the stage dir
        else:
            cov = any(under(x, a) for a in inv[1])      # the inverse removes it
        check(cov, "%s: %s is restored by %s's dry-run" % (n, x, INVERSE[n]),
              "not restored by '%s' in its own transcript" % INVERSE[n])
nd = T["nodefault"]
extra = nd[0] - nd[1] - nd[2]
check(not extra, "nodefault: it changes only by removing", "changes beyond removals: %s" % sorted(extra))

# the note: section 5, table rows
txt = open(note).read()
m = re.search(r"^## 5\. The undo\n(.*?)^## 6\.", txt, re.S | re.M)
if not m:
    bad("note", "section 5 not found")
    print("\n".join(out))
    sys.exit(0)
rows = []
for ln in m.group(1).splitlines():
    if ln.startswith("|") and not re.match(r"^\|[-| ]+\|$", ln):
        cells = [c.strip() for c in ln.strip().strip("|").split("|")]
        if len(cells) == 3 and cells[0] != "state":
            rows.append(cells)
check(len(rows) >= 3, "note section 5: the undo table has rows", "found %d" % len(rows))

note_names = set()
for r in rows:
    for c in r:
        note_names |= set(VAR.findall(c))
        note_names |= {norm(p) for p in PATH.findall(c)} - {"/boot"}
ok("note names: " + ", ".join(sorted(note_names)))

runner = set().union(*(T[n][0] for n in T))
# 2. every runner change is named by the note with its undo
for x in sorted(runner):
    if x.startswith("mp2s_"):
        hit = [r for r in rows if x in VAR.findall(" ".join(r))]
        verb = "nodefault"
    else:
        hit = [r for r in rows
               if any(under(x, a) for a in (norm(p) for p in PATH.findall(" ".join(r))) if a != "/boot")]
        verb = "uninstall"
    if not hit:
        bad("note names %s" % x, "section 5 names neither it nor an ancestor; add it to the undo table with `%s` as its undo" % verb)
        continue
    good = [r for r in hit if ("`%s`" % verb) in r[1] and r[2]]
    check(bool(good), "note: %s has undo `%s` from Linux and a without-Linux entry" % (x, verb),
          "no row naming it gives `%s` in the from-Linux cell and a without-Linux cell" % verb)
# 3. the note names nothing the runner never changes
for x in sorted(note_names):
    hit = x in runner if x.startswith("mp2s_") else any(under(c, x) for c in runner)
    check(hit, "runner changes what the note names: %s" % x,
          "no change line of any verb's dry-run touches it; remove it from section 5 or fix the runner")
print("\n".join(out))
PY
python3 -I "$W/check.py" "$NOTE" "$W" >"$W/py.out" 2>&1
[ -s "$W/py.out" ] || bad "parser" "no output"
while IFS= read -r l; do
    case $l in
        "ok   "*) N=$((N + 1)); echo "$l" ;;
        FAIL*) N=$((N + 1)); F=$((F + 1)); echo "$l" ;;
        *) echo "$l"; bad "parser" "$l" ;;
    esac
done <"$W/py.out"

echo "mp2s_note_undo_selftest: $N checks, $F failed"
[ "$F" = 0 ]
