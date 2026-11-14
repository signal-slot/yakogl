#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Verdict over a DK standalone console record (docs/notes/stm32mp257f-dk-standalone.md
sections 7 and "Board run").

  mp2s_record_check.py CONSOLE_LOG --count N [--image-line REGEX] [--linux-line REGEX]

For `default <image> N` the record must show boots left N-1 .. 0, each with the
image's line after `mp2s: go 0xa0000000`, then one boot with `default used up,
booting Linux`, `SCRIPT FAILED: continuing...` and a Linux userspace line.
Boots are split at BL2's banner / `Reset reason:` lines. A Power-on or Brownout
reset after the first boot is a supply suspect; an IWDG reset is a watchdog
reset (the image ends with PSCI SYSTEM_RESET, so neither belongs in the run).
Prints `VERDICT: PASS ...` or `VERDICT: FAIL <first broken step>`; exit 0/1.
"""
import argparse
import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[@-Z\\-_]")
BANNER = re.compile(r"NOTICE:\s+BL2: v\d")
RESET = re.compile(r"Reset reason: (.*?) \(0x[0-9a-fA-F]+\)")
MP2S = re.compile(r"mp2s: (.*)")
LEFT = re.compile(r"boots left (\d+)\b")
DEF_IMAGE = r"mp2_hello: stm32mp257f_dk, console"
DEF_LINUX = r"\blogin:|Reached target|systemd\[1\]: Startup finished"


def clean(text):
    text = ANSI.sub("", text).replace("\r\n", "\n").replace("\r", "\n")
    return text.split("\n")


class Boot:
    def __init__(self):
        self.reason = None
        self.lines = []  # (kind, text) in order

    def mp2s(self):
        return [t for k, t in self.lines if k == "mp2s"]


def split(lines, image_re, linux_re):
    boots, cur = [], None
    for ln in lines:
        m = RESET.search(ln)
        if BANNER.search(ln) or (m and (cur is None or cur.reason is not None)):
            cur = Boot()
            boots.append(cur)
        if cur is None:
            continue
        if m:
            cur.reason = m.group(1)
        s = MP2S.search(ln)
        if s:
            cur.lines.append(("mp2s", s.group(1).strip()))
        if image_re.search(ln):
            cur.lines.append(("image", ln))
        if "SCRIPT FAILED: continuing" in ln:
            cur.lines.append(("scriptfailed", ln))
        if linux_re.search(ln):
            cur.lines.append(("linux", ln))
    return boots


def index_of(b, pred):
    for i, (k, t) in enumerate(b.lines):
        if pred(k, t):
            return i
    return None


def left_is(k):
    def pred(kind, text):
        m = LEFT.search(text) if kind == "mp2s" else None
        return bool(m) and int(m.group(1)) == k
    return pred


def check(boots, count):
    """Return None when the record is right, else the first broken step."""
    first = next((i for i, b in enumerate(boots) if b.mp2s()), None)
    if first is None:
        return "no mp2s line in the record"
    for i, b in enumerate(boots):
        if i == 0:
            continue
        r = b.reason or ""
        if re.search(r"Power-on reset|Brownout reset", r):
            return f"boot {i + 1}: '{r}' in the middle of the run: supply suspect (bringup note), check the USB-C supply"
        if "IWDG" in r:
            return f"boot {i + 1}: '{r}': watchdog reset, the image hung or did not end with PSCI SYSTEM_RESET"
    run = boots[first:]
    for n, b in enumerate(run):
        label = f"boot {first + n + 1}"
        for s in b.mp2s():
            if s.startswith("refused:") or s.startswith("saveenv failed"):
                return f"{label}: mp2s {s}"
        if n < count:
            k = count - 1 - n
            li = index_of(b, left_is(k))
            if li is None:
                return f"{label}: no 'boots left {k}' line"
            gi = index_of(b, lambda kk, t: kk == "mp2s" and t.startswith("go 0xa0000000"))
            if gi is None:
                return f"{label}: no 'mp2s: go 0xa0000000' after 'boots left {k}'"
            ld = index_of(b, lambda kk, t: kk == "mp2s" and t.startswith("loaded "))
            if li > gi or (ld is not None and li > ld):
                return f"{label}: 'boots left {k}' comes after the load/go, it must be before"
            if not any(kk == "image" for kk, _ in b.lines[gi + 1:]):
                return f"{label}: image line not seen after 'go 0xa0000000'"
        elif n == count:
            ui = index_of(b, lambda kk, t: kk == "mp2s" and t.startswith("default used up, booting Linux"))
            if ui is None:
                return f"{label}: no 'default used up, booting Linux' (expected after {count} image boot(s))"
            si = index_of(b, lambda kk, t: kk == "scriptfailed")
            if si is None or si < ui:
                return f"{label}: no 'SCRIPT FAILED: continuing...' after 'default used up'"
            if not any(kk == "linux" for kk, _ in b.lines[si + 1:]):
                return f"{label}: Linux userspace line not reached"
    if len(run) < count + 1:
        return f"only {len(run)} boot(s) from the first mp2s boot, need {count + 1}"
    if len(run) > count + 1:
        return f"boot {first + count + 2}: extra boot after Linux"
    return None


def table(boots):
    out = ["boot  reset reason                              image  mp2s lines"]
    for i, b in enumerate(boots, 1):
        img = "yes" if any(k == "image" for k, _ in b.lines) else "no"
        m = b.mp2s()
        out.append(f"{i:<5} {(b.reason or '-')[:40]:<40}  {img:<5}  " + (" | ".join(m) if m else "-"))
    return out


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("log")
    ap.add_argument("--count", type=int, required=True)
    ap.add_argument("--image-line", default=DEF_IMAGE)
    ap.add_argument("--linux-line", default=DEF_LINUX)
    a = ap.parse_args(argv)
    if a.count < 1:
        ap.error("--count must be >= 1")
    try:
        with open(a.log, "rb") as f:
            text = f.read().decode("utf-8", "replace")
    except OSError as e:
        print(f"VERDICT: FAIL cannot read {a.log}: {e}")
        return 1
    boots = split(clean(text), re.compile(a.image_line), re.compile(a.linux_line))
    why = check(boots, a.count)
    for ln in table(boots):
        print(ln)
    if why:
        print(f"VERDICT: FAIL {why}")
        return 1
    print(f"VERDICT: PASS {a.count} image boot(s) then Linux, {len(boots)} boot(s) in the record")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
