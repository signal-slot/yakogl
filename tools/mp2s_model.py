#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Exhaustive fault model of the STM32MP257F-DK standalone count (plan #362).

    tools/mp2s_model.py [--max-faults F] [--nmax N] [--mutant NAME] [-q]

Checks docs/notes/stm32mp257f-dk-standalone.md section 8 ("Why no step
strands the board") mechanically. The model is a state machine of:

  (a) U-Boot's redundant environment: two copies, each with a flag byte and a
      CRC. A save writes the inactive copy with flag+1 (mod 256); a load takes
      the valid copy with the higher flag (0x00 beats 0xff, copy 1 on a tie,
      as env_import_redund and tools/uboot_env.py do). With both copies bad
      the defaults apply and env_check saves them: boot_prefixes is ST's
      default and there is no mp2s_*.
  (b) the runner's single environment write for `default <N>` and for
      `nodefault`.
  (c) the product script's steps 1-7 of section 3: decrement, saveenv, then
      load; the used-up branch restores boot_prefixes from mp2s_prefixes,
      deletes mp2s_* and saves.
  (d) the boot outcome: our image (it ends in a reset: the next power-up), or
      ST's prefix and Linux.

Faults, at every write step (the runner's write and each saveenv), up to
--max-faults per trace (default 2): a power cut before the write, a torn
write (the new copy's CRC is bad, the power is cut), a power cut after it, a
failed write that returns an error with nothing written, a failed write that
leaves a torn copy (the runner then restores the pre-edit image). Both copies
may also be lost at the start of a boot. Starting points: both copies valid
with the active one in copy 1 or 2 and its flag at 0, 1, 0xfe, 0xff (the
wrap), and both bad. N = 1..--nmax (default 9). Scenarios per start:
`default N`, and `default N` then `nodefault` before the reboot.

A "boot" is a power-up that gets as far as the image's `go` or Linux: one
that a power cut ends is not counted. The trace ends once Linux was reached
and the stored environment holds no mp2s_left (a failed used-up save is
repeated by the next boot, so the trace goes on until it is stored).

Invariants:
  I1  Linux boots within N+1 boots of the `default` (+1 per torn save), and
      every trace reaches Linux (and settles) within a bound
  I2  the image never boots more than N + (torn saves) times
  I3  once Linux is back and stored, boot_prefixes is the pre-default value
      and no mp2s_* is left (ST's defaults after both copies were bad)
  I4  bootcmd, bootcount, altbootcmd, bootlimit, upgrade_available are never
      written
  F   `forever` never reaches Linux on its own; no counted default ever
      stores mp2s_left=forever (the BOARD_PERSON_PRESENT=1 gate is the
      runner's, outside the model)

Exit 0 with all verdicts ok, 1 with the shortest counterexample otherwise,
2 on usage. --mutant injects a known-bad variant (the selftest uses them):
  load_before_save  the image is loaded before the count is decremented/saved
  save_after_load   the decrement is saved only after the image ran
  no_restore        the used-up branch forgets to restore boot_prefixes
  runner_two_step   the runner writes boot_prefixes and then mp2s_* separately
  bootcount_write   the decrement also writes bootcount
"""
import argparse
import sys
from dataclasses import dataclass, replace

PRE = "ST-LIVE"        # boot_prefixes before `default`
DEF_BP = "ST-DEFAULT"  # boot_prefixes of the compiled-in defaults
FORBIDDEN = ("bootcmd", "bootcount", "altbootcmd", "bootlimit",
             "upgrade_available")
MUTANTS = ("load_before_save", "save_after_load", "no_restore",
           "runner_two_step", "bootcount_write")


def base_vars(bp):
    v = dict(boot_prefixes=bp, bootcmd="distro", bootcount="0",
             altbootcmd="alt", bootlimit="3", upgrade_available="0")
    return tuple(sorted(v.items()))


@dataclass(frozen=True)
class St:
    c: tuple           # two copies: None (bad CRC) or (flag, vars)
    boots: int = 0     # completed boots since the default
    images: int = 0
    torn: int = 0
    linux_at: int = 0  # boots at the first Linux, 0 = not yet
    defaults: bool = False
    bad: str = ""      # first I4 violation


def active(c):
    """env_import_redund: the index of the copy in use, or None."""
    a, b = c
    if a is None and b is None:
        return None
    if b is None:
        return 0
    if a is None:
        return 1
    fa, fb = a[0], b[0]
    if fa == 0xff and fb == 0:
        return 1
    if fb == 0xff and fa == 0:
        return 0
    return 1 if fb > fa else 0


def vars_of(st):
    i = active(st.c)
    return dict(st.c[i][1]) if i is not None else None


def put(st, newvars, torn=False):
    """env_save: the inactive copy, flag+1; torn = its CRC is bad."""
    i = active(st.c)
    old = dict(st.c[i][1])
    flag = (st.c[i][0] + 1) & 0xff
    c = list(st.c)
    c[1 - i] = None if torn else (flag, tuple(sorted(newvars.items())))
    bad = st.bad
    for k in FORBIDDEN:
        if newvars.get(k) != old.get(k) and not bad:
            bad = "I4: %s written (%r -> %r)" % (k, old.get(k), newvars.get(k))
    return replace(st, c=tuple(c), bad=bad,
                   torn=st.torn + (1 if torn else 0))


def writes(st, newvars, faults, label, runner=False):
    """One write step. Yields (state, faults left, event, status), status
    ok / failed (an error is returned, the caller goes on) / cut (power)."""
    yield put(st, newvars), faults, label + ": ok", "ok"
    if faults <= 0:
        return
    f = faults - 1
    yield st, f, label + ": failed before the write", "failed"
    if runner:
        # the runner puts the pre-edit image back on a failure
        yield (put(st, vars_of(st)), f,
               label + ": failed, torn copy, pre-edit image restored", "failed")
    else:
        yield put(st, newvars, torn=True), f, label + ": failed, torn copy", "failed"
    yield st, f, label + ": power cut before the write", "cut"
    yield put(st, newvars, torn=True), f, label + ": power cut, torn copy", "cut"
    yield put(st, newvars), f, label + ": power cut after the write", "cut"


def default_vars(v, n):
    v = dict(v)
    v.update(mp2s_ver="1", mp2s_left=str(n), mp2s_prefixes=v["boot_prefixes"],
             boot_prefixes="/yakogl/standalone/ " + v["boot_prefixes"])
    return v


def nodefault_vars(v):
    v = dict(v)
    if "mp2s_prefixes" in v:
        v["boot_prefixes"] = v["mp2s_prefixes"]
    for k in [k for k in v if k.startswith("mp2s_")]:
        del v[k]
    return v


def runner_default(st, n, faults, mut):
    v = vars_of(st)
    if mut == "runner_two_step":
        first = dict(v)
        first["boot_prefixes"] = "/yakogl/standalone/ " + v["boot_prefixes"]
        second = default_vars(v, n)
        for s1, f1, e1, k1 in writes(st, first, faults, "runner write 1/2", True):
            if s1.c != st.c and k1 == "ok":
                for s2, f2, e2, _ in writes(s1, second, f1, "runner write 2/2", True):
                    yield s2, f2, [e1, e2]
            else:
                yield s1, f1, [e1]
        return
    for s1, f1, e1, _ in writes(st, default_vars(v, n), faults,
                                "runner default write", True):
        yield s1, f1, [e1]


def runner_nodefault(st, faults):
    v = vars_of(st)
    if "mp2s_left" not in v:
        yield st, faults, ["runner nodefault: nothing set, no write"]
        return
    for s1, f1, e1, _ in writes(st, nodefault_vars(v), faults,
                                "runner nodefault write", True):
        yield s1, f1, [e1]


def boot(st, faults, mut):
    """One power-up. Yields (state, faults left, events, kind) with kind
    image / linux / cut / again (the same boot, with the copies lost)."""
    if faults > 0 and st.c != (None, None):
        yield replace(st, c=(None, None)), faults - 1, ["both copies lost"], "again"
    if active(st.c) is None:
        # the defaults and env_check's save; ST's root boot.scr boots Linux
        c = ((0, base_vars(DEF_BP)), None)
        yield (replace(st, c=c, defaults=True, boots=st.boots + 1), faults,
               ["both copies bad: defaults, env_check saves them, Linux"],
               "linux")
        return
    ram = vars_of(st)
    left = ram.get("mp2s_left")
    sb = st.boots + 1
    if left is None or ram.get("mp2s_ver") != "1":
        yield (replace(st, boots=sb), faults,
               ["script: no default set, distro loop goes on, Linux"], "linux")
        return
    if left == "forever":
        yield (replace(st, boots=sb, images=st.images + 1), faults,
               ["image boots (forever)"], "image")
        return
    n = int(left) if left.isdigit() else 0
    if n > 0:
        ram["mp2s_left"] = str(n - 1)
        if mut == "bootcount_write":
            ram["bootcount"] = "1"
        if mut in ("load_before_save", "save_after_load"):
            # the image resets before the decrement is saved
            yield (replace(st, boots=sb, images=st.images + 1), faults,
                   ["image boots (%s)" % mut], "image")
            return
        for s1, f1, e1, k in writes(st, ram, faults, "script decrement saveenv"):
            if k == "ok":
                yield (replace(s1, boots=sb, images=s1.images + 1), f1,
                       [e1, "image boots, boots left %d" % (n - 1)], "image")
            elif k == "failed":
                yield (replace(s1, boots=sb), f1,
                       [e1, "nothing loaded, Linux"], "linux")
            else:
                yield s1, f1, [e1], "cut"
        return
    # used up
    if mut != "no_restore":
        ram["boot_prefixes"] = ram.get("mp2s_prefixes", ram["boot_prefixes"])
    for k in [k for k in ram if k.startswith("mp2s_")]:
        del ram[k]
    for s1, f1, e1, k in writes(st, ram, faults, "script used-up saveenv"):
        if k == "cut":
            yield s1, f1, [e1], "cut"
        else:
            yield replace(s1, boots=sb), f1, [e1, "Linux"], "linux"


def settled(st):
    v = vars_of(st)
    return v is not None and "mp2s_left" not in v


def explore(st, faults, trace, n, cap, mut, out):
    """Depth-first over every boot and fault. out["viol"] collects
    (invariant, trace, message), out["leaves"] counts the traces."""
    def leaf(inv, t, msg):
        out["leaves"] += 1
        if inv:
            out["viol"].append((inv, t, msg))

    for s1, f1, ev, kind in boot(st, faults, mut):
        t = trace + ev
        if s1.bad:
            leaf("I4", t, s1.bad)
            continue
        if kind in ("again", "cut"):
            explore(s1, f1, t, n, cap, mut, out)
            continue
        if kind == "image" and s1.images > n + s1.torn:
            leaf("I2", t, "image booted %d times, N=%d, torn saves=%d"
                 % (s1.images, n, s1.torn))
            continue
        if kind == "linux" and not s1.linux_at:
            s1 = replace(s1, linux_at=s1.boots)
            if s1.linux_at > n + 1 + s1.torn:
                leaf("I1", t, "Linux at boot %d, bound N+1+torn = %d"
                     % (s1.linux_at, n + 1 + s1.torn))
                continue
        if s1.boots > cap:
            leaf("I1", t, "no Linux and stored default within %d boots" % cap)
            continue
        if kind == "linux" and settled(s1):
            v = vars_of(s1)
            want = DEF_BP if s1.defaults else PRE
            left = sorted(k for k in v if k.startswith("mp2s_"))
            if v["boot_prefixes"] != want or left:
                leaf("I3", t, "Linux back with boot_prefixes=%r (want %r), mp2s_* left: %s"
                     % (v["boot_prefixes"], want, left or "none"))
            else:
                leaf(None, t, "")
            continue
        explore(s1, f1, t, n, cap, mut, out)


def starts():
    for flag in (0, 1, 0xfe, 0xff):
        for idx in (0, 1):
            c = [None, None]
            c[idx] = (flag, base_vars(PRE))
            c[1 - idx] = ((flag - 1) & 0xff, base_vars(PRE))
            yield "active copy %d flag 0x%02x" % (idx + 1, flag), tuple(c)


def run(max_faults, nmax, mut):
    out = {"leaves": 0, "viol": []}
    for n in range(1, nmax + 1):
        cap = n + max_faults + 4
        for name, c in starts():
            st0 = St(c=c)
            for scen in ("default", "default+nodefault"):
                for s1, f1, ev in runner_default(st0, n, max_faults, mut):
                    head = ["[N=%d, %s, %s]" % (n, name, scen)] + ev
                    left = vars_of(s1).get("mp2s_left")
                    if left not in (None, str(n)):
                        out["leaves"] += 1
                        out["viol"].append(("F", head, "mp2s_left=%r after a counted default" % left))
                        continue
                    if scen == "default":
                        explore(s1, f1, head, n, cap, mut, out)
                    else:
                        for s2, f2, ev2 in runner_nodefault(s1, f1):
                            explore(s2, f2, head + ev2, n, cap, mut, out)
        explore(St(c=(None, None)), max_faults,
                ["[N=%d, both copies bad at the start]" % n], n, cap, mut, out)
    return out


def forever_check():
    """`forever`: the image boots every time, nothing is written, Linux is
    not reached on its own (no loss of the copies: that is the defaults')."""
    v = default_vars(dict(base_vars(PRE)), 1)
    v["mp2s_left"] = "forever"
    st = St(c=((1, tuple(sorted(v.items()))), (0, base_vars(PRE))))
    for _ in range(50):
        outs = list(boot(st, 0, None))
        if len(outs) != 1 or outs[0][3] != "image" or outs[0][0].c != st.c:
            return "forever left the image or wrote the environment"
        st = outs[0][0]
    return None


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--max-faults", type=int, default=2)
    ap.add_argument("--nmax", type=int, default=9)
    ap.add_argument("--mutant", choices=MUTANTS)
    a = ap.parse_args(argv)
    if a.max_faults < 0 or a.nmax < 1:
        ap.error("--max-faults >= 0 and --nmax >= 1")
    out = run(a.max_faults, a.nmax, a.mutant)
    fe = forever_check()
    if fe:
        out["viol"].append(("F", ["forever"], fe))
    by = {}
    for inv, t, m in out["viol"]:
        by.setdefault(inv, []).append((t, m))
    print("model: N=1..%d, up to %d fault(s) per trace%s: %d traces" % (
        a.nmax, a.max_faults, ", mutant " + a.mutant if a.mutant else "",
        out["leaves"]))
    desc = {"I1": "Linux within N+1(+torn) boots, every trace reaches Linux",
            "I2": "image boots at most N+torn times",
            "I3": "boot_prefixes restored, no mp2s_* left",
            "I4": "bootcmd/bootcount/altbootcmd/bootlimit/upgrade_available never written",
            "F": "forever never reaches Linux on its own, never entered from a counted default"}
    for inv in ("I1", "I2", "I3", "I4", "F"):
        r = by.get(inv)
        print("%-2s %s  %s%s" % (inv, "FAIL" if r else "ok  ", desc[inv],
                                  " (%d traces)" % len(r) if r else ""))
    if out["viol"]:
        inv, t, m = min(out["viol"], key=lambda x: len(x[1]))
        print("counterexample (%s: %s), the shortest of %d:"
              % (inv, m, len(out["viol"])))
        for k, e in enumerate(t):
            print("  %d. %s" % (k, e))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
