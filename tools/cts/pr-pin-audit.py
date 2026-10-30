#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""pr-pin-audit.py [--root DIR]: which partial-render firmware constants
does driver/tools/offsetof/ddk_abi_pins.h not pin (plan 148, acceptance c5)?

Constants: every name defined in the `pins!` block of
driver/rust/src/gpu/pvr/fwif.rs (`NAME: ty = value => "C expression";`, the
expression may sit on the next line).

A constant is a PARTIAL-RENDER constant when
  (a) one of its parts (split at `_`) is PR, PARTIAL or PRBUFFER or starts
      with PARTIAL, RTA, HWRT, SPM or PRBUFFER (HWRTDATA_FLAGS, RTA_CTL_FLAGS
      count; PRIVATE, PROTECT, STRIDE do not); or
  (b) it is referenced from PR code in render.rs, freelist.rs, ccb.rs
      (driver/rust/src/gpu/pvr/) or src/pvr/backend.rs.
PR code is, outside `#[cfg(test)]` items:
  - the brace block opened by an `if` whose condition names `pr` (the knob
    is read once into `let pr = r.cfg.partial_render != 0;`) or
    `partial_render`; an inline `if pr { .. }` counts for its own line;
  - the line of a `let pr = ...partial_render` and every line carrying the
    marker `PR-CODE`; the lines between `// PR-BEGIN` and `// PR-END`.
Comments (`//`) are not code: a name only in a comment is not a use.

Pinned: ddk_abi_pins.h has a `PIN(value, expr)` whose expr equals the
constant's C expression (whitespace normalised).

Output: one line per constant, `pinned|UNPINNED NAME file:line  expr`, where
file:line is the first PR use (or the definition for rule (a) only). Exit 1
if any is UNPINNED, 2 on unreadable input.
"""
import argparse
import os
import re
import sys

FWIF = "driver/rust/src/gpu/pvr/fwif.rs"
PINS = "driver/tools/offsetof/ddk_abi_pins.h"
USERS = ["driver/rust/src/gpu/pvr/render.rs", "driver/rust/src/gpu/pvr/freelist.rs",
         "driver/rust/src/gpu/pvr/ccb.rs", "src/pvr/backend.rs"]
WORDS = {"PR", "PARTIAL", "PRBUFFER"}
PREFIXES = ("PARTIAL", "RTA", "HWRT", "SPM", "PRBUFFER")
IF_PR = re.compile(r"\bif\b[^{]*(\bpr\b|partial_render)")


def by_name(name):
    parts = name.split("_")
    return bool(WORDS & set(parts)) or any(p.startswith(PREFIXES) for p in parts)


def norm(s):
    return re.sub(r"\s+", "", s)


def read(root, rel):
    try:
        with open(os.path.join(root, rel), encoding="utf-8") as f:
            return f.read()
    except OSError as e:
        sys.exit(f"pr-pin-audit: {e}")


def defs(text):
    """name -> (line, expr); an expression on the line after `=>` is joined."""
    out = {}
    lines = text.split("\n")
    for i, ln in enumerate(lines):
        m = re.match(r"^\s*([A-Z][A-Z0-9_]*)\s*:\s*[A-Za-z0-9_]+\s*=", ln)
        if not m:
            continue
        chunk = ln + (" " + lines[i + 1] if "\"" not in ln.split("=>")[-1] and i + 1 < len(lines) else "")
        e = re.search(r'=>\s*"([^"]*)"', chunk)
        out[m.group(1)] = (i + 1, e.group(1) if e else None)
    return out


def pr_lines(text):
    """Yield (lineno, code) for the lines that are PR code."""
    depth = 0
    pr_depths = []  # brace depth at which each open PR block started
    in_marked = False
    skip_depth = None  # depth of the #[cfg(test)] item being skipped
    pending_test = False
    for n, raw in enumerate(text.split("\n"), 1):
        code = re.sub(r"//.*", "", raw)
        if "PR-BEGIN" in raw:
            in_marked = True
        if "PR-END" in raw:
            in_marked = False
        if re.match(r"\s*#\[cfg\(test\)\]", raw):
            pending_test = True
        opens, closes = code.count("{"), code.count("}")
        if skip_depth is None and pending_test and opens:
            skip_depth, pending_test = depth, False
        gate = bool(IF_PR.search(code))
        inline = gate or "PR-CODE" in raw or bool(re.search(r"\blet\s+pr\s*=.*partial_render", code))
        if (pr_depths or in_marked or inline) and skip_depth is None:
            yield n, code
        if gate and opens > closes:
            pr_depths.append(depth)
        depth += opens - closes
        while pr_depths and depth <= pr_depths[-1]:
            pr_depths.pop()
        if skip_depth is not None and depth <= skip_depth and closes:
            skip_depth = None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
    a = ap.parse_args()
    consts = defs(read(a.root, FWIF))
    pins = {norm(m.group(1)) for m in re.finditer(r"^\s*PIN\(\s*[^,]+,\s*(.*?)\)\s*(?:/\*.*)?$",
                                                  read(a.root, PINS), re.M)}
    uses = {}  # name -> file:line of the first PR use
    for rel in USERS:
        if not os.path.exists(os.path.join(a.root, rel)):
            continue
        for n, code in pr_lines(read(a.root, rel)):
            for t in re.findall(r"\b[A-Z][A-Z0-9_]*\b", code):
                if t in consts:
                    uses.setdefault(t, f"{rel}:{n}")
    sel = {}
    for name, (line, _) in consts.items():
        if name in uses or by_name(name):
            sel[name] = uses.get(name, f"{FWIF}:{line}")
    bad = 0
    for name in sorted(sel, key=lambda k: consts[k][0]):
        expr = consts[name][1]
        ok = expr is not None and norm(expr) in pins
        bad += not ok
        print(f"{'pinned' if ok else 'UNPINNED'} {name} {sel[name]}  {expr}")
    print(f"pr-pin-audit: {len(sel)} constants, {len(sel) - bad} pinned, {bad} unpinned")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
