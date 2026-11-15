#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_contract_check.sh - section 7's console-line and variable contract of
# the DK standalone boot, script against design note (and record check).
#
#   tools/mp2s_contract_check.sh [--script s.cmd] [--note n.md]
#                                [--record-check r.py] [--root dir]
#
# (a) the echo lines and the mp2s_ names (and the other names setenv/setexpr
# set) of mp2-standalone.cmd, (b) the names and console lines the note quotes
# (inline backticks, and the "Console lines" block of section 7), (c) each
# mismatch is "CONTRACT: <what> in <file> missing from <file>". (d) a
# "(pending)" line of the note whose tools/ file exists under --root (default:
# the repository) is "STALE-PENDING: <tool>". (e) only when the record check
# exists: every pattern or literal it matches about the script's lines must
# match one line the script prints; otherwise "contract-check: record check
# absent, skipped". Exit 0 clean, 1 findings, 2 usage. No board, no network.
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
script= note= rec=
usage() { echo "usage: $0 [--script s.cmd] [--note n.md] [--record-check r.py] [--root dir]" >&2; exit 2; }
while [ $# -gt 0 ]; do
	case $1 in
	--script) [ $# -ge 2 ] || usage; script=$2; shift 2 ;;
	--note) [ $# -ge 2 ] || usage; note=$2; shift 2 ;;
	--record-check) [ $# -ge 2 ] || usage; rec=$2; shift 2 ;;
	--root) [ $# -ge 2 ] || usage; root=$2; shift 2 ;;
	*) usage ;;
	esac
done
: "${script:=$root/driver/tools/mp2-standalone/mp2-standalone.cmd}"
: "${note:=$root/docs/notes/stm32mp257f-dk-standalone.md}"
: "${rec:=$root/tools/mp2s_record_check.py}"
for f in "$script" "$note"; do
	[ -r "$f" ] || { echo "mp2s_contract_check: cannot read $f" >&2; exit 2; }
done
have_rec=0
if [ -e "$rec" ]; then
	[ -r "$rec" ] || { echo "mp2s_contract_check: cannot read $rec" >&2; exit 2; }
	have_rec=1
else
	echo "contract-check: record check absent, skipped"
fi

exec python3 -I - "$script" "$note" "$rec" "$have_rec" "$root" <<'PY'
import ast, os, re, sys
script, note, rec, have_rec, root = sys.argv[1:6]
sn, nn, rn = (os.path.basename(p) for p in (script, note, rec))
bad = []
DOLLAR = re.compile(r"\$\{[^}]*\}")
ANGLE = re.compile(r"<[^>]*>")

# ---- (a) the script
NAME = r"[A-Za-z_][A-Za-z0-9_]*"
s_vars, s_echo = set(), []
for line in open(script, encoding="utf-8"):
    st = line.strip()
    if st.startswith("#") or not st:
        continue
    for m in re.finditer(r"\b(mp2s_[a-z0-9_]+)\b", line):
        s_vars.add(m.group(1))
    for m in re.finditer(r"(?:^|[;&|]|\bthen\b|\belse\b)\s*(?:setenv|setexpr(?:\.[bwlq])?|env set)\s+(?:-\S+\s+)*(%s)" % NAME, st):
        s_vars.add(m.group(1))
    m = re.match(r'(?:.*?[;&|]\s*|.*?\b(?:then|else)\s+)?echo\s+"((?:[^"\\]|\\.)*)"', st)
    if m:
        s_echo.append(m.group(1))

# ---- (b) the note (a tool name such as mp2s_guard.py or tools/mp2s_x is not a variable)
nlines = open(note, encoding="utf-8").read().split("\n")
prose, console, infence, sec = [], [], False, None
for line in nlines:
    if line.startswith("```"):
        infence = not infence
        continue
    if infence:
        if sec == "Console lines":
            console.append(line)
        continue
    m = re.match(r"^#+ (.*)", line)
    if m:
        sec = m.group(1).strip()
    prose.append(line)
n_vars = set()
for line in prose:
    for tick in re.findall(r"`([^`]*)`", line):
        for m in re.finditer(r"(?<![/\w])(mp2s_[a-z0-9_]+)\b(?![*.]\w|\*)", tick):
            n_vars.add(m.group(1))
console = [re.sub(r"\s+<-\s.*$", "", l).rstrip() for l in console]
console = [l for l in console if l.startswith("mp2s: ")]

# ---- (c) variables
for v in sorted(s_vars - n_vars):
    if v.startswith("mp2s_"):
        bad.append("CONTRACT: variable %s in %s missing from %s" % (v, sn, nn))
for v in sorted(n_vars - s_vars):
    bad.append("CONTRACT: variable %s in %s missing from %s" % (v, nn, sn))

# ---- (c) console lines
def rx(text, sep):  # the placeholders match anything
    return re.compile("(?s)" + ".*".join(re.escape(p) for p in sep.split(text)))
def cov(nl, el):
    return bool(rx(el, DOLLAR).fullmatch(ANGLE.sub("X", nl))
                or rx(nl, ANGLE).fullmatch(DOLLAR.sub("X", el)))
if not s_echo:
    bad.append("CONTRACT: no echo line found in %s" % sn)
if not console:
    bad.append("CONTRACT: no console line block found in %s" % nn)
for el in s_echo:
    if el.startswith("mp2s: ") and not any(cov(nl, el) for nl in console):
        bad.append("CONTRACT: console line '%s' in %s missing from %s" % (el, sn, nn))
for nl in console:
    if not any(cov(nl, el) for el in s_echo):
        bad.append("CONTRACT: console line '%s' in %s missing from %s" % (nl, nn, sn))

# ---- (d) stale pending
seen = set()
for line in nlines:
    if "(pending" not in line:
        continue
    for t in re.findall(r"tools/[A-Za-z0-9_./-]+", line):
        t = t.rstrip(".,:;")
        if t not in seen and os.path.isfile(os.path.join(root, t)):
            seen.add(t)
            bad.append("STALE-PENDING: " + t)

# ---- (e) the record check
if have_rec == "1":
    try:
        tree = ast.parse(open(rec, encoding="utf-8").read())
    except SyntaxError as e:
        print("mp2s_contract_check: cannot parse %s: %s" % (rec, e), file=sys.stderr)
        sys.exit(2)
    def lit(n):
        return n.value if isinstance(n, ast.Constant) and isinstance(n.value, str) else None
    pats = []
    for n in ast.walk(tree):
        if isinstance(n, ast.Call) and n.args:
            f = n.func
            name = f.attr if isinstance(f, ast.Attribute) else getattr(f, "id", "")
            v = lit(n.args[0])
            if v is not None and name in ("compile", "search", "match", "fullmatch", "findall", "finditer"):
                pats.append((v, True))
            elif v is not None and name in ("startswith", "endswith", "find", "count", "index"):
                pats.append((v, False))
        elif isinstance(n, ast.Compare) and any(isinstance(o, (ast.In, ast.NotIn)) for o in n.ops):
            v = lit(n.left)
            if v is not None:
                pats.append((v, False))
    # only patterns about the script's own lines; Linux or image markers are
    # not lines of the script
    key = re.compile(r"mp2s|boots left|used up|forever|refused|image returned|saveenv failed|no default set")
    fills = [DOLLAR.sub(f, e) for e in s_echo for f in ("1", "x")]
    done = set()
    for v, isre in pats:
        if v in done or not key.search(v):
            continue
        done.add(v)
        if isre:
            try:
                r = re.compile(v)
            except re.error:
                bad.append("CONTRACT: record-check pattern '%s' in %s is not a regex" % (v, rn))
                continue
            hit = any(r.search(l) for l in fills)
        else:
            hit = any(v in l for l in fills)
        if not hit:
            bad.append("CONTRACT: record-check pattern '%s' in %s matches no line printed by %s" % (v, rn, sn))
for b in bad:
    print(b)
if bad:
    sys.exit(1)
print("mp2s_contract_check: ok (%d names, %d console lines)" % (len(s_vars), len(s_echo)))
PY
