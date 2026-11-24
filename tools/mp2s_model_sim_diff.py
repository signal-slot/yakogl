#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Differential check: tools/mp2s_model.py against tools/mp2s_script_sim.py.

    tools/mp2s_model_sim_diff.py [--script FILE] [--nmax N] [--model-mutant NAME]

The model's boot() mirrors the product script by hand; the simulator
interprets the script's own text. For each N in 1..--nmax this drives the
model along the one trace the simulator's `faults` mode runs (the clean run,
and a power cut before / torn / after at each of the N+1 saveenv calls) and
compares, per case:

  - the number of saveenv calls of the clean run (N+1),
  - the boot that reaches Linux (the cut boot is not counted by the model),
  - how many times the image ran,
  - that the environment ends restored.

Any difference is a drift between the model and the script: exit 1 with the
case. Exit 0: they agree on every case. Exit 2: usage or a sim error.
--script lets the selftest feed a mutated script; --model-mutant a mutated
model. Standard library only.
"""
import argparse
import importlib.util
import os
import re
import subprocess
import sys

sys.dont_write_bytecode = True  # the gate's rows leave the tree read-only

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_SCRIPT = os.path.join(HERE, "..", "driver", "tools", "mp2-standalone",
                              "mp2-standalone.cmd")
CUTS = {"before": "power cut before the write",
        "torn": "power cut, torn copy",
        "after": "power cut after the write"}


def load_model():
    spec = importlib.util.spec_from_file_location(
        "mp2s_model", os.path.join(HERE, "mp2s_model.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def model_trace(m, n, cut, mut):
    """Drive the model with no fault, except `cut` = (saveenv index, kind)
    taken at that saveenv. Returns (Linux boot, image runs, restored, saves), None if it never settles."""
    st = m.St(c=((5, m.base_vars(m.PRE)), (4, m.base_vars(m.PRE))))
    st = next(m.runner_default(st, n, 0, None))[0]
    saves = 0
    faults = 1 if cut else 0
    for _ in range(4 * n + 10):
        pick = None
        for s1, f1, ev, kind in m.boot(st, faults, mut):
            text = " ".join(ev)
            if "both copies lost" in text:
                continue
            is_save = "saveenv" in text
            if f1 < faults:  # a fault variant: only the chosen power cut
                if (is_save and kind == "cut" and saves + 1 == cut[0]
                        and CUTS[cut[1]] in text):
                    pick = (s1, kind, True)
                    break
                continue
            if pick is None:
                pick = (s1, kind, is_save)
            if not is_save:
                break
        if pick is None:
            raise SystemExit("diff: model has no step at saveenv %d" % (saves + 1))
        st, kind, saved = pick
        if saved:
            saves += 1
        if kind == "cut":
            faults = 0
            continue
        if kind == "linux" and not st.linux_at:
            st = m.replace(st, linux_at=st.boots)
        if kind == "linux" and m.settled(st):
            v = m.vars_of(st)
            ok = v["boot_prefixes"] == m.PRE and not [k for k in v if k.startswith("mp2s_")]
            return st.linux_at, st.images, ok, saves
    return None  # never reached Linux with a stored, settled environment


LINE = re.compile(r"^sim: fault: (?:none|saveenv (\d+) of (\d+), cut (\w+)): "
                  r"Linux at boot (\S+), image runs (\d+)(?:, saves (\d+))?: (ok|FAIL.*)$")


def sim_cases(script, n):
    p = subprocess.run([sys.executable, "-I", "-B", os.path.join(HERE, "mp2s_script_sim.py"),
                        "faults", script, "--left", str(n)],
                       capture_output=True, text=True)
    if p.returncode == 2:
        raise SystemExit("diff: sim error: " + (p.stdout + p.stderr).strip())
    cases = {}
    for line in p.stdout.splitlines():
        mm = LINE.match(line)
        if not mm:
            continue
        i, total, kind, boot, runs, saves, verdict = mm.groups()
        key = (int(i), kind) if i else None
        cases[key] = dict(boot=None if boot == "-" else int(boot), runs=int(runs),
                          saves=int(saves) if saves else None, verdict=verdict)
    return cases


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--script", default=DEFAULT_SCRIPT)
    ap.add_argument("--nmax", type=int, default=9)
    ap.add_argument("--model-mutant")
    a = ap.parse_args(argv)
    if a.nmax < 1 or a.nmax > 9:
        ap.error("--nmax 1..9")
    m = load_model()
    if a.model_mutant and a.model_mutant not in m.MUTANTS:
        ap.error("unknown model mutant")
    diffs = ncases = 0
    for n in range(1, a.nmax + 1):
        sim = sim_cases(a.script, n)
        plan = [None] + [(i, k) for i in range(1, n + 2) for k in CUTS]
        for key in plan:
            res = model_trace(m, n, key, a.model_mutant)
            s = sim.get(key)
            ncases += 1
            if res is None:
                print("DIFF N=%d cut=%s: the model never settles back on Linux" % (n, key))
                diffs += 1
                continue
            lb, im, ok, saves = res
            if s is None:
                print("DIFF N=%d cut=%s: the sim has no such case" % (n, key))
                diffs += 1
                continue
            sim_boot = s["boot"] - 1 if key and s["boot"] else s["boot"]
            bad = []
            if sim_boot != lb:
                bad.append("Linux boot: model %s, sim %s" % (lb, sim_boot))
            if s["runs"] != im:
                bad.append("image runs: model %s, sim %s" % (im, s["runs"]))
            if not (ok and s["verdict"] == "ok"):
                bad.append("restored: model %s, sim verdict %s" % (ok, s["verdict"]))
            if key is None and not (s["saves"] == saves == n + 1):
                bad.append("saveenv calls: model %d, sim %s, want %d" % (saves, s["saves"], n + 1))
            if bad:
                diffs += 1
                print("DIFF N=%d cut=%s: %s" % (n, key or "none", "; ".join(bad)))
    print("diff: N=1..%d, %d cases, %d differences" % (a.nmax, ncases, diffs))
    return 1 if diffs else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
