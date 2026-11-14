#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_varname_check.sh - the DK standalone variables against ST's default env.
#
#   tools/mp2s_varname_check.sh [-s script.cmd] [-n note.md] [-f st-env.txt]
#
# Names the script sets (setenv / env set / setexpr targets) and the names the
# design note lists (section 3's variable table and the mp2s_* names of
# section 7) must agree, and neither may meet a name of ST's initial env
# (the fixture) or the deny list: bootcmd, bootcount, altbootcmd, bootlimit,
# upgrade_available, boot_targets, boot_prefixes (read, never saved). Each
# conflict is printed as "CONFLICT: ..."; exit 1 on any, 0 if clean, 2 on a
# usage or input error. No board, no network.
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
script=$root/driver/tools/mp2-standalone/mp2-standalone.cmd
note=$root/docs/notes/stm32mp257f-dk-standalone.md
fixture=$root/driver/tools/mp2-standalone/st-env/stm32mp25-dk-initial-env.txt
while getopts s:n:f: o; do
	case $o in
	s) script=$OPTARG ;; n) note=$OPTARG ;; f) fixture=$OPTARG ;;
	*) echo "usage: $0 [-s script] [-n note] [-f fixture]" >&2; exit 2 ;;
	esac
done
for f in "$script" "$note" "$fixture"; do
	[ -r "$f" ] || { echo "mp2s_varname_check: cannot read $f" >&2; exit 2; }
done

exec python3 -I - "$script" "$note" "$fixture" <<'PY'
import re, sys
script, note, fixture = sys.argv[1:4]
DENY = ["bootcmd", "bootcount", "altbootcmd", "bootlimit",
        "upgrade_available", "boot_targets", "boot_prefixes"]
NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")

def script_names(path):
    out = set()
    for line in open(path, encoding="utf-8"):
        if line.lstrip().startswith("#"):
            continue
        # a line may hold several commands: split on ; and then/else/do
        for cmd in re.split(r";|\bthen\b|\belse\b|\bdo\b", line):
            w = cmd.split()
            while w and w[0] in ("if", "elif", "while", "!"):
                w = w[1:]
            if not w:
                continue
            if w[0] == "env" and len(w) > 1 and w[1] == "set":
                w = w[1:]
            elif not re.match(r"^(setenv|setexpr(\.[bwlq])?)$", w[0]):
                continue
            args = [a for a in w[1:] if not a.startswith("-")]
            if args and NAME.match(args[0]):
                out.add(args[0])
    return out

def note_names(path):
    out, sec = set(), None
    for line in open(path, encoding="utf-8").read().split("\n"):
        m = re.match(r"^#+ (\d+)\. ", line)
        if m:
            sec = m.group(1)
        elif re.match(r"^## ", line):
            sec = None
        if sec == "3" and line.startswith("|"):
            for n in re.findall(r"`([A-Za-z_][A-Za-z0-9_]*)`", line.split("|")[1]):
                out.add(n)
        elif sec == "7":
            for n in re.findall(r"\b(mp2s_[a-z0-9_]+)\b(?!\*)", line):
                out.add(n)
    out.discard("variable")
    return out

def fixture_names(path):
    out = set()
    for line in open(path, encoding="utf-8", errors="replace"):
        if "=" in line:
            n = line.split("=", 1)[0]
            if NAME.match(n):
                out.add(n)
    return out

s, d, st = script_names(script), note_names(note), fixture_names(fixture)
bad = []
if not s:
    bad.append("no variable found in the script %s" % script)
if not d:
    bad.append("no variable found in the note %s" % note)
for n in sorted(s - d):
    bad.append("set by the script but not listed in the note: %s" % n)
for n in sorted(d - s):
    bad.append("listed in the note but never set by the script: %s" % n)
for n in sorted((s | d) & st):
    bad.append("collides with ST's default env (fixture): %s" % n)
for n in sorted((s | d) & set(DENY)):
    bad.append("collides with the deny list: %s" % n)
for b in bad:
    print("CONFLICT: " + b)
if bad:
    sys.exit(1)
print("mp2s_varname_check: ok (%d names, none in ST's %d or the deny list)"
      % (len(s), len(st)))
PY
