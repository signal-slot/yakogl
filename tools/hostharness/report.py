#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Evaluate host-harness runs (run.sh output directories).

  report.py status   <run>              per-run outcome table
  report.py compare  <runA> <runB>      A vs B, same job and mode; exit 1 on any
                                        unexpected difference
  report.py modes    <run>              cross-mode checks inside one build
  report.py coverage <run> [<run>...]   entry points never called
Options: --allow FILE (default: allow.txt next to this script), --verbose
"""
import argparse
import collections
import hashlib
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
INT_MIN, INT_MAX = -2147483648, 2147483647


# --------------------------------------------------------------------- runs

def load(run):
    logs = os.path.join(run, "logs")
    res = {}
    if not os.path.isdir(logs):
        sys.exit(f"report.py: {run} has no logs/")
    for f in os.listdir(logs):
        if not f.endswith(".rc"):
            continue
        job, mode = f[:-3].rsplit(".", 1)
        base = os.path.join(logs, f[:-3])
        res[(job, mode)] = base
    return res


def read(path, mode="r"):
    try:
        with open(path, mode, errors="replace") if mode == "r" else open(path, mode) as fh:
            return fh.read()
    except FileNotFoundError:
        return "" if mode == "r" else b""


def outcome(base):
    """One word for how a run ended, plus details."""
    rc = read(base + ".rc").strip()
    log = read(base + ".log")
    err = read(base + ".err")
    det = []
    m = re.search(r"ERROR: AddressSanitizer: ([\w-]+)", err)
    p = re.search(r"yakogl: panic: panicked at (\S+?):?\n", log)
    ub = sorted(set(re.findall(r"(\S+:\d+:\d+): runtime error: (.*)", err)))
    if m:
        # the access kind and size, without addresses
        acc = re.search(r"\n(READ|WRITE) of size (\d+)", err)
        word = f"asan:{m.group(1)}" + (f":{acc.group(1)}{acc.group(2)}" if acc else "")
    elif p:
        word = "panic:" + p.group(1)
    elif rc == "124":
        word = "timeout"
    elif rc != "0":
        word = f"exit{rc}"
        if "hh-crash signal" in log:
            word = "signal" + re.search(r"hh-crash signal (\d+)", log).group(1)
    else:
        word = "ok"
    g = re.search(r"^gl_test rc=(-?\d+)", log, re.M)
    if g and g.group(1) != "0":
        first = re.search(r"^gl_test: .*(?:fail|FAIL|error).*$", log, re.M)
        det.append(f"gl_test rc={g.group(1)}" + (f" ({first.group(0)[:90]})" if first else ""))
        word += f"+rc{g.group(1)}"
    if ub:
        det.append(f"ubsan x{len(ub)}: " + "; ".join(f"{a} {b}" for a, b in ub[:3]))
        word += "+ubsan"
    hh = re.findall(r"^HH-ERROR (.*)$", log, re.M)
    if hh:
        word += f"+hh{len(hh)}"
        det.append("HH-ERROR " + "; ".join(sorted(set(re.sub(r"#\d+", "#N", h) for h in hh))[:4]))
    if "hh-end" not in log and rc == "0":
        word += "+noend"
    return word, det


# --------------------------------------------------------------- allow-list

class Rule:
    def __init__(self, rid, scope, kind, pattern, note):
        self.id, self.kind, self.note = rid, kind, note
        self.scope = re.compile(scope)
        self.sel = None
        if kind.startswith("sub"):
            if " ;; " in pattern:
                sel, pattern = pattern.split(" ;; ", 1)
                self.sel = re.compile(sel)
            pat, _, rep = pattern.partition("=>")
            self.rx, self.rep = re.compile(pat.strip()), rep.strip()
        else:
            self.rx = re.compile(pattern)
        self.delta = int(kind.split(":")[1]) if kind.startswith("delta:") else None


def load_allow(paths):
    rules = []
    for path in paths:
        rules += load_allow_file(path)
    return rules


def load_allow_file(path):
    rules = []
    if not os.path.exists(path):
        sys.exit(f"report.py: no allow-list {path}")
    for ln in open(path):
        ln = ln.rstrip("\n")
        if not ln.strip() or ln.lstrip().startswith("#"):
            continue
        f = ln.split("\t")
        if len(f) < 5:
            sys.exit(f"report.py: bad allow-list line: {ln}")
        rules.append(Rule(*f[:5]))
    return rules


def ints(s):
    return [int(x) for x in re.findall(r"-?\d+", s)]


def pair_ok(a, b, rules):
    """Rule id that explains line a (A) vs line b (B), or None."""
    na, nb = a, b
    used = []
    for r in rules:
        if r.kind == "sub":
            if r.sel and not (r.sel.match(na) and r.sel.match(nb)):
                continue
            na2, nb2 = r.rx.sub(r.rep, na), r.rx.sub(r.rep, nb)
            if (na2, nb2) != (na, nb):
                used.append(r.id)
            na, nb = na2, nb2
    if na == nb:
        return ",".join(used) if used else None
    for r in rules:
        ma, mb = r.rx.match(na), r.rx.match(nb)
        if not (ma and mb):
            continue
        if r.kind == "line":
            return r.id
        if r.kind == "intsat":
            # same line up to integer values where one side holds the x86
            # "integer indefinite" (INT_MIN) and the other a saturated/0 value
            ta, tb = re.sub(r"-?\d+", "#", ma.group("v")), re.sub(r"-?\d+", "#", mb.group("v"))
            if na[:ma.start("v")] != nb[:mb.start("v")] or ta != tb:
                continue
            va, vb = ints(ma.group("v")), ints(mb.group("v"))
            if all(x == y or (INT_MIN in (x, y) and {x, y} <= {INT_MIN, INT_MAX, 0}) for x, y in zip(va, vb)):
                return r.id
        if r.delta is not None:
            # same line except the number in group v, which differs by exactly delta
            rest_a = na[:ma.start("v")] + na[ma.end("v"):]
            rest_b = nb[:mb.start("v")] + nb[mb.end("v"):]
            va, vb = ints(ma.group("v")), ints(mb.group("v"))
            if rest_a == rest_b and len(va) == len(vb) == 1 and abs(va[0] - vb[0]) == r.delta:
                return r.id
    return None


def hunks(fa, fb):
    """(a_lines, b_lines, a_start) of every change, via diff(1)."""
    out = subprocess.run(["diff", "-U0", "--speed-large-files", fa, fb], capture_output=True, text=True,
                         errors="replace").stdout
    res, cur = [], None
    for ln in out.splitlines():
        if ln.startswith("@@"):
            m = re.match(r"@@ -(\d+)", ln)
            cur = ([], [], int(m.group(1)))
            res.append(cur)
        elif cur is not None and ln.startswith("-") and not ln.startswith("---"):
            cur[0].append(ln[1:])
        elif cur is not None and ln.startswith("+") and not ln.startswith("+++"):
            cur[1].append(ln[1:])
    return res


def classify(job, fa, fb, rules, filt=None):
    """None if identical; else (unexpected hunks, allowed rule ids, first unexpected hunk)."""
    if filt is None and os.path.getsize(fa) == os.path.getsize(fb):
        if digest(fa) == digest(fb):
            return None
    if filt is not None:
        fa, fb = filt(fa), filt(fb)
        if digest(fa) == digest(fb):
            return None
    rs = [r for r in rules if r.scope.search(job)]
    bad, allowed, first = 0, collections.Counter(), None
    for a, b, start in hunks(fa, fb):
        ids = []
        ok = False
        if len(a) == len(b):
            ids = [pair_ok(x, y, rs) for x, y in zip(a, b)]
            ok = all(ids)
        if not ok:
            lr = [r for r in rs if r.kind == "line"]
            if lr and all(any(r.rx.match(x) for r in lr) for x in a + b):
                ok, ids = True, [next(r.id for r in lr if r.rx.match(x)) for x in a + b]
        if ok:
            for i in ids:
                allowed.update(i.split(","))
        else:
            bad += 1
            if first is None:
                first = (start, a[:4], b[:4])
    if bad == 0 and not allowed:
        return None
    return bad, allowed, first


_dig = {}


def digest(path):
    if path not in _dig:
        h = hashlib.blake2b()
        with open(path, "rb") as fh:
            for blk in iter(lambda: fh.read(1 << 20), b""):
                h.update(blk)
        _dig[path] = h.digest()
    return _dig[path]


# ----------------------------------------------------------------- commands

def family(job):
    return job if not job.startswith("gltest.") else "gltest." + ("all" if "all" in job else "case")


def cmd_status(a):
    runs = load(a.run)
    modes = sorted({m for _, m in runs})
    jobs = sorted({j for j, _ in runs})
    width = max(len(j) for j in jobs)
    print(f"{'job':{width}}  " + "  ".join(f"{m:>6}" for m in modes))
    details = []
    for j in jobs:
        cells = []
        for m in modes:
            if (j, m) not in runs:
                cells.append(f"{'-':>6}")
                continue
            w, det = outcome(runs[(j, m)])
            cells.append(f"{short(w):>6}")
            if w != "ok":
                details.append(f"  {j}.{m}: {w}" + ("".join(f"\n      {d}" for d in det)))
        print(f"{j:{width}}  " + "  ".join(cells))
    print("legend: ok | A=asan P=panic S=signal E=exit T=timeout | r=gl_test rc!=0 | u=ubsan | h=HH-ERROR")
    print("\n".join(details))


def comparable(w):
    """The part of an outcome both builds must share: UBSan exists only for
    C code, and the size of a sanitizer-reported access depends on how the
    code copies (memcpy vs byte loop), not on the bug."""
    parts = [p for p in w.split("+") if p != "ubsan"]
    parts[0] = re.sub(r":(READ|WRITE)\d+$", r":\1", parts[0])
    return "+".join(parts)


def short(w):
    s = {"ok": "ok"}.get(w)
    if s:
        return s
    head = w.split("+")[0]
    c = {"asan": "A", "panic": "P", "timeout": "T", "ok": ""}.get(head.split(":")[0],
                                                                    "S" if head.startswith("signal") else "E")
    for part in w.split("+")[1:]:
        c += "r" if part.startswith("rc") else "u" if part == "ubsan" else "h" if part.startswith("hh") else "?"
    return c or "ok"


def cmd_compare(a):
    rules = load_allow(a.allow)
    A, B = load(a.a), load(a.b)
    keys = sorted(set(A) | set(B))
    modes = sorted({m for _, m in keys})
    jobs = sorted({j for j, _ in keys})
    width = max(len(j) for j in jobs)
    unexpected = 0
    notes, allowed_all = [], collections.Counter()
    early = []
    rows = []
    for j in jobs:
        cells = []
        for m in modes:
            k = (j, m)
            if k not in A or k not in B:
                cells.append("miss" if (k in A) != (k in B) else "-")
                if (k in A) != (k in B):
                    unexpected += 1
                    notes.append(f"{j}.{m}: only in {'A' if k in A else 'B'}")
                continue
            wa, da = outcome(A[k])
            wb, db = outcome(B[k])
            ca, cb = comparable(wa), comparable(wb)
            for side, w, d in (("A", wa, da), ("B", wb, db)):
                if "+rc" in w:
                    early.append(f"{j}.{m} [{side}] {w}: " + "; ".join(d))
            r = classify(j, A[k] + ".log", B[k] + ".log", rules)
            same_status = ca == cb
            if r is None:
                cell = "=" if same_status else "st"
            else:
                bad, allowed, first = r
                allowed_all.update(allowed)
                if bad:
                    cell = f"D{bad}"
                else:
                    cell = "a"
            if not same_status and cell in ("=", "a"):
                cell = "st"
            if wa != "ok" and wa == wb:
                cell += "!"
            if cell.startswith(("D", "st")):
                unexpected += 1
                msg = f"{j}.{m}: A={wa} B={wb}"
                if r and r[0]:
                    s, la, lb = r[2]
                    msg += f"\n      {r[0]} unexpected hunk(s); first at A line {s}:" + \
                           "".join(f"\n      < {x[:160]}" for x in la) + "".join(f"\n      > {x[:160]}" for x in lb)
                notes.append(msg)
            cells.append(cell)
        rows.append((j, cells))
    print(f"A = {a.a}\nB = {a.b}\n")
    print(f"{'job':{width}}  " + " ".join(f"{m:>6}" for m in modes))
    for j, cells in rows:
        print(f"{j:{width}}  " + " ".join(f"{c:>6}" for c in cells))
    print("\nlegend: = identical | a = differs only by allow-listed artefacts | Dn = n unexpected diff hunks |"
          " st = same log, different outcome | ! = both ended abnormally the same way (see status)")
    if allowed_all:
        print("\nallow-listed differences seen:")
        for rid, n in sorted(allowed_all.items()):
            note = next((r.note for r in rules if r.id == rid), "")
            print(f"  {rid:24} x{n:<7} {note}")
    if early:
        print(f"\ngl_test early failures ({len(early)}; compared like any other run, listed so they are not missed):")
        for e in early[:60]:
            print("  " + e)
    if notes:
        print(f"\nUNEXPECTED ({unexpected}):")
        for n in notes:
            print("  " + n)
    print(f"\nsummary: {len(keys)} runs, {unexpected} unexpected")
    return 1 if unexpected else 0


VISIBLE = re.compile(r"^(~|gl_test|== |scen |hostile |HH-ERROR)")


def visible_filter(path):
    out = path + ".visible2"
    if not os.path.exists(out) or os.path.getmtime(out) < os.path.getmtime(path):
        with open(path, errors="replace") as fi, open(out, "w") as fo:
            for ln in fi:
                if VISIBLE.match(ln):
                    # allocation record numbers depend on the platform mode
                    fo.write(re.sub(r"#\d+", "#N", ln) if ln.startswith("HH-ERROR") else ln)
    return out


# gl_test prints raw pixel values, colour maps and checks of what it read
# back; without a GPU that is the fill pattern itself (the read-back is
# compared, hashed, in the glReadPixels line): its own lines are left out
RAW_PIXELS = re.compile(r"^gl_test[: ]")


def fill_filter(path):
    out = path + ".fill2"
    if not os.path.exists(out) or os.path.getmtime(out) < os.path.getmtime(path):
        with open(path, errors="replace") as fi, open(out, "w") as fo:
            for ln in fi:
                if not ln.startswith("hh-config") and not RAW_PIXELS.match(ln):
                    fo.write(ln)
    return out


CHECKS = [
    ("fill 0x00 vs 0xA5 (lag 0): reads of uninitialised GPU memory", "z0", "a0", fill_filter),
    ("fill 0x00 vs 0xA5 (lag 2)", "z2", "a2", fill_filter),
    ("CPU view vs none (lag 0): app-visible results", "l0", "l0nc", visible_filter),
    ("CPU view vs none (lag 2): app-visible results", "l2", "l2nc", visible_filter),
    ("lag 0 vs lag 2: app-visible results", "l0", "l2", visible_filter),
    ("no map_phys vs map_phys: app-visible results", "l0", "l0mp", visible_filter),
    ("map_phys without vs with CPU view: app-visible results", "l0mp", "l0mpc", visible_filter),
    ("harness allocator vs linux.c: app-visible results", "l0", "base", visible_filter),
]


def cmd_modes(a):
    rules = load_allow(a.allow)
    R = load(a.run)
    jobs = sorted({j for j, _ in R})
    found = 0
    for title, m1, m2, filt in CHECKS:
        lines = []
        for j in jobs:
            if (j, m1) not in R or (j, m2) not in R:
                continue
            w1, _ = outcome(R[(j, m1)])
            w2, _ = outcome(R[(j, m2)])
            r = classify(j, R[(j, m1)] + ".log", R[(j, m2)] + ".log", [x for x in rules if x.kind == "line"], filt)
            if r is None and w1 == w2:
                continue
            msg = f"{j}: {m1}={w1} {m2}={w2}"
            if r and r[0]:
                s, la, lb = r[2]
                msg += f" ({r[0]} hunks; first at line {s})" + "".join(f"\n      < {x[:150]}" for x in la) + \
                       "".join(f"\n      > {x[:150]}" for x in lb)
            elif r is None:
                msg += " (same visible log)"
            else:
                continue
            lines.append(msg)
        print(f"== {title}: {len(lines)} job(s) differ")
        for ln in lines:
            print("  " + ln)
        found += len(lines)
    return 1 if found else 0


def cmd_coverage(a):
    exported = None
    before, after = collections.Counter(), collections.Counter()
    for run in a.runs:
        exp = [x.strip() for x in open(os.path.join(run, "gen", "exported.txt")) if x.strip()]
        exported = exp if exported is None else exported
        for (j, m), base in load(run).items():
            for ln in read(base + ".calls").splitlines():
                n, c = ln.split()
                after[n] += int(c)
                if j.startswith("gltest."):
                    before[n] += int(c)
    nb = [n for n in exported if not before[n]]
    na = [n for n in exported if not after[n]]
    print(f"exported gl*/egl* entry points: {len(exported)}")
    print(f"never called by gl_test.c (gltest runs): {len(nb)}")
    print("  " + " ".join(nb))
    print(f"never called by any harness program: {len(na)}")
    print("  " + " ".join(na))
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--allow", action="append", help="allow-list file (repeatable; default: allow.txt)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("status"); s.add_argument("run")
    s = sub.add_parser("compare"); s.add_argument("a"); s.add_argument("b")
    s = sub.add_parser("modes"); s.add_argument("run")
    s = sub.add_parser("coverage"); s.add_argument("runs", nargs="+")
    a = ap.parse_args()
    if not a.allow:
        a.allow = [os.path.join(HERE, "allow.txt")]
    sys.exit({"status": cmd_status, "compare": cmd_compare, "modes": cmd_modes, "coverage": cmd_coverage}[a.cmd](a) or 0)


if __name__ == "__main__":
    main()
