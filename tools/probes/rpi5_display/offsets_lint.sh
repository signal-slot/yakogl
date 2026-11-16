#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# offsets_lint.sh - register offsets and bits agree across the rpi5_scanout headers.
#
# usage: offsets_lint.sh [header.h ...]     (default: every *.h of
#        driver/zephyr/lib/board/rpi5_scanout/ in this checkout)
#
# Scans for '#define NAME value', enum members and 'static const T NAME = value;'
# whose value is an integer expression (numbers, + - << |, parentheses; hex and
# decimal are the same value; u/l suffixes are ignored). Other definitions are skipped.
#
# Normalisation: each header keeps its own copies under its own prefix. The
# prefixes below are stripped, in this order, once each, from the front of every
# name, so YD5_HVS6_VERSION and HVS6_VERSION both become VERSION. HVS6C_/HVS6D_
# (chip-variant register banks) do not match 'HVS6_' and stay distinct on purpose.
# Override the list with OFFSETS_LINT_PREFIXES="A_ B_ ...".
PREFIXES="${OFFSETS_LINT_PREFIXES:-YD5_ HVS6_ PV_ HDMI_}"
# The block of a name is its first token after only this vendor prefix is
# stripped (HVS6_VERSION -> block HVS6, HVS6C_DISP0_CTRL0 -> block HVS6C).
VENDOR_PREFIXES="${OFFSETS_LINT_VENDOR:-YD5_}"
#
# Names ending in _LOG_MAX (each header's own write-log capacity) are skipped.
#
# Two checks, one conflict line each (all pairs, across different headers):
#   offsets_lint: <name> <file1>=<v1> <file2>=<v2>      same normalised name, two values
#   offsets_lint: <block>+<off> <name1>=<file1> <name2>=<file2>
#                                      same block and offset, two names; only for
#                                      offsets (hex literal of 4+ digits, not
#                                      *_MASK/*_SHIFT/*_STRIDE)
# Last line: offsets_lint: N headers, M shared names, K conflicts
# Exit 0 for K = 0, 1 otherwise, 2 on usage errors. Needs only bash and awk.

usage() { echo "usage: offsets_lint.sh [header.h ...]" >&2; exit 2; }

files=()
for a in "$@"; do
	case "$a" in
	-h|--help|-*) usage ;;
	*) files+=("$a") ;;
	esac
done
if [ ${#files[@]} -eq 0 ]; then
	here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
	dir="$here/../../../driver/zephyr/lib/board/rpi5_scanout"
	[ -d "$dir" ] || { echo "offsets_lint: no such directory $dir" >&2; exit 2; }
	for f in "$dir"/*.h; do [ -f "$f" ] && files+=("$f"); done
	[ ${#files[@]} -gt 0 ] || { echo "offsets_lint: no headers in $dir" >&2; exit 2; }
fi
for f in "${files[@]}"; do
	[ -f "$f" ] || { echo "offsets_lint: no such file $f" >&2; exit 2; }
done

LC_ALL=C awk -v prefixes="$PREFIXES" -v vendors="$VENDOR_PREFIXES" '
function hexval(s,   i, c, v) {
	v = 0; s = tolower(s)
	for (i = 1; i <= length(s); i++) {
		c = index("0123456789abcdef", substr(s, i, 1)) - 1
		v = v * 16 + c
	}
	return v
}
function bitor(a, b,   r, p, x, y) {
	r = 0; p = 1
	while (a >= 1 || b >= 1) {
		x = a % 2; y = b % 2
		if (x >= 1 || y >= 1) r += p
		a = int(a / 2); b = int(b / 2); p *= 2
	}
	return r
}
# tokenise s into tk[1..ntk]; returns 0 on an unknown token
function lex(s,   m) {
	ntk = 0
	while (s != "") {
		if (match(s, /^[ \t]+/)) { s = substr(s, RLENGTH + 1); continue }
		if (match(s, /^0[xX][0-9a-fA-F]+[uUlL]*/)) {
			m = substr(s, 1, RLENGTH); sub(/[uUlL]+$/, "", m)
			tk[++ntk] = "N" hexval(substr(m, 3)); s = substr(s, RLENGTH + 1); continue
		}
		if (match(s, /^[0-9]+[uUlL]*/)) {
			m = substr(s, 1, RLENGTH); sub(/[uUlL]+$/, "", m)
			tk[++ntk] = "N" (m + 0); s = substr(s, RLENGTH + 1); continue
		}
		if (match(s, /^<</)) { tk[++ntk] = "<<"; s = substr(s, 3); continue }
		if (match(s, /^[-+|()]/)) { tk[++ntk] = substr(s, 1, 1); s = substr(s, 2); continue }
		return 0
	}
	tk[ntk + 1] = ""	# no stale tokens from the previous expression
	return 1
}
function p_prim(   v, t) {
	t = tk[pos]
	if (t == "(") {
		pos++; v = p_or()
		if (tk[pos] != ")") bad = 1
		pos++; return v
	}
	if (t == "-") { pos++; return -p_prim() }
	if (substr(t, 1, 1) == "N") { pos++; return substr(t, 2) + 0 }
	bad = 1; return 0
}
function p_shift(   v, n) {
	v = p_prim()
	while (tk[pos] == "<<") { pos++; n = p_prim(); v = v * (2 ^ n) }
	return v
}
function p_add(   v) {
	v = p_shift()
	while (tk[pos] == "+" || tk[pos] == "-") {
		if (tk[pos++] == "+") v += p_shift(); else v -= p_shift()
	}
	return v
}
function p_or(   v) {
	v = p_add()
	while (tk[pos] == "|") { pos++; v = bitor(v, p_add()) }
	return v
}
# evaluate s; sets evok
function evalx(s,   v) {
	evok = 0
	if (!lex(s) || ntk == 0) return 0
	pos = 1; bad = 0
	v = p_or()
	if (bad || pos <= ntk) return 0
	evok = 1; return v
}
function strip(name,   i, n, p, vp, d) {
	n = split(vendors, vp, " ")
	d = name
	for (i = 1; i <= n; i++)
		if (index(d, vp[i]) == 1) { d = substr(d, length(vp[i]) + 1); break }
	blk = d; sub(/_.*/, "", blk)
	n = split(prefixes, p, " ")
	for (i = 1; i <= n; i++)
		if (index(name, p[i]) == 1 && length(name) > length(p[i]))
			name = substr(name, length(p[i]) + 1)
	return name
}
function fmt(v) { return (v < 0) ? sprintf("%d", v) : sprintf("0x%x", v) }
function record(raw, expr, lit,   v, nm, key, okey) {
	# per-header write-log capacities, not hardware values
	if (raw ~ /_LOG_MAX$/) return 1
	v = evalx(expr)
	if (!evok) return 0
	nm = strip(raw)
	key = nm SUBSEP FILENAME
	if (!(key in val)) {
		val[key] = v
		nfn[nm] = nfn[nm] " " FILENAME
		nnames[nm]++
	}
	# offset-like: hex literal of 4+ digits, not a mask / shift / stride
	if (lit ~ /^[ \t(]*0[xX][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]+[uUlL]*[ \t)]*$/ &&
	    nm !~ /(_MASK|_SHIFT|_STRIDE)$/) {
		okey = blk SUBSEP v SUBSEP nm SUBSEP FILENAME
		if (!(okey in oseen)) {
			oseen[okey] = 1
			no++
			ob[no] = blk; ov[no] = v; on[no] = nm; of[no] = FILENAME
		}
	}
	return 1
}
function base(f,   n, a) { n = split(f, a, "/"); return a[n] }
FNR == 1 { nfiles++; fname[FILENAME] = base(FILENAME); incom = 0; inenum = 0; cnt = 0; cntok = 1 }
{
	line = $0
	if (incom) {
		if (!match(line, /\*\//)) next
		line = substr(line, RSTART + 2); incom = 0
	}
	while (match(line, /\/\*/)) {
		pre = substr(line, 1, RSTART - 1); rest = substr(line, RSTART + 2)
		if (match(rest, /\*\//)) line = pre " " substr(rest, RSTART + 2)
		else { line = pre; incom = 1; break }
	}
	sub(/\/\/.*/, "", line)
	if (match(line, /^[ \t]*#[ \t]*define[ \t]+[A-Za-z_][A-Za-z_0-9]*[ \t]+/)) {
		head = substr(line, 1, RLENGTH); expr = substr(line, RLENGTH + 1)
		sub(/^[ \t]*#[ \t]*define[ \t]+/, "", head); sub(/[ \t]+$/, "", head)
		record(head, expr, expr)
		next
	}
	if (match(line, /^[ \t]*(static[ \t]+)?const[ \t][^=;()]*[ \t*][A-Za-z_][A-Za-z_0-9]*[ \t]*=/)) {
		head = substr(line, 1, RLENGTH); expr = substr(line, RLENGTH + 1)
		sub(/[ \t]*=$/, "", head); sub(/.*[ \t*]/, "", head)
		sub(/[ \t]*;.*/, "", expr)
		record(head, expr, expr)
		next
	}
	if (line ~ /(^|[^A-Za-z_0-9])enum([^A-Za-z_0-9]|$)/ && line ~ /\{/) {
		inenum = 1; cnt = 0; cntok = 1
		line = substr(line, index(line, "{") + 1)
	}
	if (inenum) {
		endl = 0
		if (index(line, "}")) { line = substr(line, 1, index(line, "}") - 1); endl = 1 }
		n = split(line, parts, ",")
		for (i = 1; i <= n; i++) {
			m = parts[i]
			if (m ~ /^[ \t]*$/) continue
			if (match(m, /^[ \t]*[A-Za-z_][A-Za-z_0-9]*[ \t]*=/)) {
				nmx = substr(m, 1, RLENGTH); ex = substr(m, RLENGTH + 1)
				sub(/[ \t]*=$/, "", nmx); sub(/^[ \t]+/, "", nmx)
				cntok = record(nmx, ex, ex)
				if (cntok) cnt = evalx(ex) + 1
			} else if (match(m, /^[ \t]*[A-Za-z_][A-Za-z_0-9]*[ \t]*$/)) {
				nmx = m; gsub(/[ \t]/, "", nmx)
				if (cntok) record(nmx, cnt "", "")
				cnt++
			}
		}
		if (endl) inenum = 0
	}
}
END {
	shared = 0
	for (nm in nnames) if (nnames[nm] >= 2) {
		shared++
		n = split(nfn[nm], fl, " ")
		for (i = 1; i <= n; i++) for (j = i + 1; j <= n; j++) {
			a = val[nm SUBSEP fl[i]]; b = val[nm SUBSEP fl[j]]
			if (a != b) {
				printf "offsets_lint: %s %s=%s %s=%s\n", nm, fname[fl[i]], fmt(a), fname[fl[j]], fmt(b) | "sort -u"
				k++
			}
		}
	}
	for (i = 1; i <= no; i++) for (j = i + 1; j <= no; j++)
		if (ob[i] == ob[j] && ov[i] == ov[j] && on[i] != on[j] && of[i] != of[j]) {
			printf "offsets_lint: %s+%s %s=%s %s=%s\n", ob[i], fmt(ov[i]), on[i], fname[of[i]], on[j], fname[of[j]] | "sort -u"
			k++
		}
	close("sort -u")
	printf "offsets_lint: %d headers, %d shared names, %d conflicts\n", nfiles, shared, k
	exit (k > 0)
}' "${files[@]}"
