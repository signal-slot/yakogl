#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""dk_verdict.py - the P6/P7 verdicts of one STM32MP257F-DK console log.

    dk_verdict.py [--expect CHECK,...] [--only CHECK,...] [LOGFILE]   (stdin without LOGFILE)

Prints one line per check, "verdict <check> <field>=<PASS|FAIL|value> ...",
then "N checks, M failed". Exit 0: every check printed PASS; 1: a check
failed (or none applied); 2: usage. Formats and sources: README.md.

A check applies when the log shows its program (see APPLIES); --expect forces
checks, so a missing program is a FAIL instead of a silent skip; --only runs
exactly the named checks.
"""
import re
import sys

CHECKS = ["return-trial", "hang-trial", "handover", "identify-wdog", "gpu-identity",
          "gpu-intr", "gpu-idle", "gc-irq", "gc-fence", "gc-clear", "gc-triangle"]

MODEL, REV = 0x8000, 0x6205
INTR = 0xFFFFFFFF
RECORD_MAGIC, KIND_APP = 0x594B0000, 2
STATUS_PASS = 1

# board-stm32mp2.sh plain(): a leading "HH:MM:SS " or "<secs>.<frac> " stamp.
STAMP = re.compile(r"^([0-9:]{8}|[0-9]{9,}\.[0-9]+) ")
# Zephyr LOG_INF: "[00:00:01.234,000] <inf> gpu_gc: " before the message.
ZLOG = re.compile(r"^\[[0-9:.,]+\] <[a-z]{3}> [A-Za-z0-9_]+: ")


def clean(text):
    lines = []
    for raw in text.replace("\0", "").replace("\r", "").split("\n"):
        line = STAMP.sub("", raw)
        line = ZLOG.sub("", line)
        lines.append(line)
    # The last run: from the last "MP2:A EL<n>" (the image's first line).
    start = 0
    for i, line in enumerate(lines):
        if re.match(r"^MP2:A EL\d", line):
            start = i
    return lines[start:]


def kv(line):
    return dict(re.findall(r"(\w+)=(\S+)", line))


def num(s):
    try:
        return int(s, 0)
    except (TypeError, ValueError):
        return None


class Log:
    def __init__(self, lines):
        self.lines = lines

    def find(self, pat, start=0):
        rx = re.compile(pat)
        for i in range(start, len(self.lines)):
            m = rx.search(self.lines[i])
            if m:
                return i, m
        return None, None

    def has(self, pat):
        return self.find(pat)[0] is not None

    def last(self, pat):
        rx = re.compile(pat)
        hit = None
        for i, line in enumerate(self.lines):
            m = rx.search(line)
            if m:
                hit = (i, m)
        return hit or (None, None)


def ps(ok):
    return "PASS" if ok else "FAIL"


END = r"^mp2: .+: PSCI SYSTEM_RESET at \d+ ms"


def check_return_trial(log):
    banner = (log.has(r"^MP2:A EL\d") and log.has(r"^MP2:A1$") and
              log.has(r"^MP2:B console up") and
              log.has(r"^mp2_(hello|identify|gpu): build "))
    end = log.has(END) and not log.has(r"^mp2: PSCI SYSTEM_RESET returned") \
        and not log.has(r"^mp2: fatal error") and not log.has(r"^mp2: lifetime over")
    return [("banner", ps(banner)), ("psci-reset", ps(end))]


def check_hang_trial(log):
    banner = log.has(r"^MP2:B console up") and log.has(r"^mp2_hello: build ")
    hi, _ = log.find(r"^mp2_hello: hanging on purpose")
    after = log.lines[hi + 1:] if hi is not None else []
    quiet = hi is not None and not any(
        re.search(END, x) or re.search(r"^mp2: PSCI SYSTEM_RESET returned", x) or
        re.search(r"^mp2_hello: PASS: ", x) for x in after)
    return [("banner", ps(banner)), ("hang-line", ps(hi is not None)),
            ("no-psci-reset", ps(quiet))]


def check_handover(log):
    i, _ = log.find(r"^mp2-handover: ")
    if i is None:
        return [("fields", "FAIL"), ("line", "absent")]
    line = log.lines[i]
    if re.search(r"^mp2-handover: v=1 missing", line):
        return [("fields", "FAIL"), ("line", "missing")]
    d = kv(line)
    ok = d.get("v") == "1"
    vals, out = {}, []
    for key in ("el", "m", "c", "i", "x0", "x1", "pc", "cntfrq", "mpidr", "psci"):
        vals[key] = num(d.get(key))
        if vals[key] is None:
            ok = False
        out.append((key, d.get(key, "absent")))
    if vals["el"] not in (1, 2) or not vals["cntfrq"] or not vals["psci"]:
        ok = False
    return [("fields", ps(ok))] + out


def check_identify_wdog(log):
    armed = log.has(r"^gc_wdog_arm: armed OP-TEE arm,smc-wdt fid 0x[0-9a-f]+, timeout \d+ s") \
        and not log.has(r"^mp2: OP-TEE watchdog arming FAILED")
    passed = log.has(r"^mp2_identify: PASS: watchdog armed \(\d+ s\), \d+ PETs") and \
        not log.has(r"^mp2_identify: FAIL: ")
    return [("armed", ps(armed)), ("pass-line", ps(passed))]


GPU_LINE = r"^mp2_gpu: model=(0x[0-9a-f]+) rev=(0x[0-9a-f]+) product=(0x[0-9a-f]+) " \
           r"eco=(0x[0-9a-f]+) customer=(0x[0-9a-f]+)"


def check_gpu_identity(log):
    _, m = log.find(GPU_LINE)
    if not m:
        return [("model", "FAIL:absent"), ("rev", "FAIL:absent"), ("customer", "absent")]
    model, rev, _, _, cust = m.groups()
    return [("model", ps(num(model) == MODEL) + ":" + model),
            ("rev", ps(num(rev) == REV) + ":" + rev), ("customer", cust)]


def gpu_verdict(log):
    _, m = log.find(r"^mp2_gpu: verdict (PASS|FAIL)\b(.*)$")
    return m


def check_gpu_intr(log):
    _, m = log.find(r"^mp2_gpu: gpu intr_enbl wrote (0x[0-9a-f]+) read (0x[0-9a-f]+), then 0")
    rb = bool(m) and num(m.group(1)) == INTR and num(m.group(2)) == INTR
    v = gpu_verdict(log)
    flag = bool(v and v.group(1) == "PASS" and "intr=ok" in v.group(2))
    return [("readback", ps(rb) + (":" + m.group(2) if m else ":absent")),
            ("intr", "ok" if flag else "FAIL")]


def check_gpu_idle(log):
    _, m = log.find(r"^mp2_gpu: reset: rcc (0x[0-9a-f]+) held, (0x[0-9a-f]+) released; "
                    r"idle (0x[0-9a-f]+) after (\d+) reads")
    v = gpu_verdict(log)
    flag = bool(v and v.group(1) == "PASS" and "idle=ok" in v.group(2))
    return [("reset-idle", ps(bool(m)) + (":" + m.group(3) if m else ":absent")),
            ("idle", "ok" if flag else "FAIL"),
            ("verdict", ps(bool(v and v.group(1) == "PASS")))]


def check_gc_irq(log):
    ci, _ = log.find(r"^irq \d+ connected")
    if ci is None:
        return [("connected", "FAIL"), ("route", "absent")]
    gi, m = log.find(r"^gc_irq: spi=\d+ .* route=(\S+)$", ci)
    if gi is None:
        return [("route", "FAIL:absent")]
    route = m.group(1)
    _, g = log.find(r"^gc_irq: igroupr=0x[0-9a-f]+ .* bit23=\d+ expect=1 (ok|mismatch)", gi)
    return [("route", ps(route == "ours") + ":" + route),
            ("igroupr", g.group(1) if g else "absent")]


def record_field(log):
    _, m = log.last(r"^yk-record: ([0-9a-f]{8}) ([0-9a-f]{8}) ([0-9a-f]{8}) ([0-9a-f]{8})$")
    if not m:
        return ("record", "FAIL:none")
    w = [int(g, 16) for g in m.groups()]
    if (w[0] & 0xFFFF0000) != RECORD_MAGIC:
        return ("record", "FAIL:magic")
    kind = (w[0] >> 8) & 0xFF
    return ("record", ps(kind == KIND_APP and (w[1] & 0xFF) == STATUS_PASS) + ":kind=%d" % kind)


def sample_model(log, name):
    _, m = log.find(r"^%s: GPU model (0x[0-9a-f]+) revision (0x[0-9a-f]+) " % name)
    return ("model", ps(bool(m) and num(m.group(1)) == MODEL and num(m.group(2)) == REV))


def sample_final(log, name, pat):
    return ("final", ps(log.has(pat) and not log.has(r"^%s: FAIL" % name)))


def check_gc_fence(log):
    _, m = log.find(r"^gc_fence: fence (\d+) irqs (\d+) elapsed \d+ us irq-latency \d+ us "
                    r"\(completed (\d+), wait (-?\d+)\)")
    retired = bool(m) and int(m.group(4)) == 0
    irq = retired and int(m.group(2)) > 0
    return [sample_model(log, "gc_fence"), ("retired", ps(retired)),
            ("irq", ps(irq) + (":%s" % m.group(2) if m else ":absent")),
            sample_final(log, "gc_fence", r"^gc_fence: PASS$"), record_field(log)]


def check_gc_clear(log):
    _, m = log.find(r"^gc_clear: \d+x\d+ wrong pixels (\d+) ")
    return [sample_model(log, "gc_clear"),
            ("readback", ps(bool(m) and int(m.group(1)) == 0) +
             (":%s" % m.group(1) if m else ":absent")),
            sample_final(log, "gc_clear", r"^gc_clear: PASS$"), record_field(log)]


def check_gc_triangle(log):
    out = [sample_model(log, "gc_triangle")]
    for key, what in (("apex-red", "red apex"), ("apex-green", "green left"),
                      ("apex-blue", "blue right"), ("outside", "outside"),
                      ("covered", "covered")):
        pat = r"^gc_triangle: %s\s+(PASS|FAIL)\b" % what
        found = [re.search(pat, x) for x in log.lines]
        found = [m.group(1) for m in found if m]
        out.append((key, ps(bool(found) and "FAIL" not in found)))
    out.append(sample_final(log, "gc_triangle", r"^gc_triangle: PASS \(0 failing checks\)$"))
    out.append(record_field(log))
    return out


FUNCS = {
    "return-trial": check_return_trial, "hang-trial": check_hang_trial,
    "handover": check_handover, "identify-wdog": check_identify_wdog,
    "gpu-identity": check_gpu_identity, "gpu-intr": check_gpu_intr,
    "gpu-idle": check_gpu_idle, "gc-irq": check_gc_irq, "gc-fence": check_gc_fence,
    "gc-clear": check_gc_clear, "gc-triangle": check_gc_triangle,
}

# What makes a check apply without --expect.
APPLIES = {
    "return-trial": lambda l: l.has(r"^mp2_(hello|identify|gpu): ") and
    not l.has(r"^mp2_hello: hanging on purpose"),
    "hang-trial": lambda l: l.has(r"^mp2_hello: hanging on purpose"),
    "handover": lambda l: l.has(r"^mp2-handover: "),
    "identify-wdog": lambda l: l.has(r"^mp2_identify: "),
    "gpu-identity": lambda l: l.has(r"^mp2_gpu: "),
    "gpu-intr": lambda l: l.has(r"^mp2_gpu: "),
    "gpu-idle": lambda l: l.has(r"^mp2_gpu: "),
    "gc-irq": lambda l: l.has(r"^irq \d+ connected") or l.has(r"^gc_irq: "),
    "gc-fence": lambda l: l.has(r"^gc_fence: "),
    "gc-clear": lambda l: l.has(r"^gc_clear: "),
    "gc-triangle": lambda l: l.has(r"^gc_triangle: "),
}


def run(text, expect, only):
    log = Log(clean(text))
    results = []
    for c in CHECKS:
        if (c in only) if only else (c in expect or APPLIES[c](log)):
            fields = FUNCS[c](log)
            bad = any(v.startswith("FAIL") or (k, v) == ("igroupr", "mismatch") or
                      (k in ("intr", "idle") and v != "ok") for k, v in fields)
            line = "verdict %s %s" % (c, " ".join("%s=%s" % kvp for kvp in fields))
            results.append((line, bad))
    return results


def main(argv):
    expect, only, path = set(), set(), None
    args = argv[1:]
    while args:
        a = args.pop(0)
        if a in ("--expect", "--only") and args:
            for c in args.pop(0).split(","):
                if c not in CHECKS:
                    sys.stderr.write("dk_verdict: unknown check %s (%s)\n" % (c, ", ".join(CHECKS)))
                    return 2
                (expect if a == "--expect" else only).add(c)
        elif a.startswith("-") and a != "-":
            sys.stderr.write(__doc__)
            return 2
        else:
            path = a
    if path and path != "-":
        with open(path, "rb") as fh:
            text = fh.read().decode("utf-8", "replace")
    else:
        text = sys.stdin.buffer.read().decode("utf-8", "replace")
    results = run(text, expect, only)
    for line, _ in results:
        print(line)
    failed = sum(1 for _, bad in results if bad)
    print("%d checks, %d failed" % (len(results), failed))
    return 1 if failed or not results else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
