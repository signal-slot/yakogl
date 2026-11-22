#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Differential selftest (no board, no QEMU): tools/mp2s_model.py (the fault
# model, design note section 8) and tools/mp2s_script_sim.py (the real
# driver/tools/mp2-standalone/mp2-standalone.cmd interpreted) must agree on
# every boot of `default N` under a power cut at every saveenv.
#
# For every N in 1..9, every start (flags 5,6 and the wrap 255,0), the clean
# run and every cut (saveenv i, before/torn/after) the simulator offers, both
# tools are driven boot by boot to Linux with the env restored, and the
# per-boot outcome is compared: image started / Linux / cut, the stored
# mp2s_left after the boot, and the final env (boot_prefixes, mp2s_*).
# Not comparable (listed, not compared): the model's failed-write and
# both-copies-lost faults (the sim's saveenv-fails is a whole-run switch);
# the model's N=0 and forever.
# Prints "mp2s_model_sim_diff_selftest: N checks, F failed".
# Exit 0 all agree, 1 a disagreement, 2 usage/setup.
set -u
here=$(cd "$(dirname "$0")" && pwd)
script=$here/../driver/tools/mp2-standalone/mp2-standalone.cmd
[ -f "$script" ] || { echo "no $script" >&2; exit 2; }
command -v python3 >/dev/null || { echo "no python3" >&2; exit 2; }
python3 -I - "$here" "$script" <<'PY'
import sys, types
here, script = sys.argv[1:3]
sys.path.insert(0, here)
import mp2s_model as M
import mp2s_script_sim as S

STOCK = dict(S.STOCK_ENV)
PRE = STOCK["boot_prefixes"]
OUR = S.OUR_PREFIX
MOUR = "/yakogl/standalone/"


def norm(bp, pre, ours):
    if bp == pre:
        return "PRE"
    if bp == ours + " " + pre:
        return "OURS+PRE"
    return bp


def norm_sim(env):
    out = dict((k, v) for k, v in env.items() if k.startswith("mp2s_"))
    out["boot_prefixes"] = norm(env.get("boot_prefixes"), PRE, OUR)
    return out


def norm_model(v):
    out = dict((k, x) for k, x in v.items() if k.startswith("mp2s_"))
    out["boot_prefixes"] = norm(v["boot_prefixes"], M.PRE, MOUR)
    return out


def sim_args(flags, n):
    return types.SimpleNamespace(
        left=str(n), env=None, set=[], unset=[], no_script=False,
        no_image=False, image=None, load_fails=False, image_returns=False,
        saveenv_fails=False, start_flags="%d,%d" % flags, max_boots=20,
        until_restored=False, check_restored=False, script=script)


def sim_trace(n, flags, cut):
    a = sim_args(flags, n)
    ast, slen, stock, start = S.build(a)
    sim = S.Sim(ast, slen, stock, start, a, cut=cut, quiet=True)
    tr = []
    for k in range(1, 21):
        sim.boots = k
        why = sim.boot(k)
        env = sim.stored_env()
        kind = {"reset": "image"}.get(why, why)
        tr.append((kind, env.get("mp2s_left")))
        if kind == "linux" and sim.restored(env):
            break
        if kind == "stuck":
            break
    return tr, norm_sim(sim.stored_env()), sim.saves


def model_trace(n, flags, cut):
    stock = M.base_vars(M.PRE)
    start = M.default_vars(dict(stock), n)
    st = M.St(c=((flags[0], stock), (flags[1], tuple(sorted(start.items())))))
    saves = 0
    tr = []
    for _ in range(20):
        # a boot saves iff it offers power-cut outcomes under one fault
        opts = [o for o in M.boot(st, 1, None) if o[3] == "cut"]
        nxt = None
        if opts and cut and cut[0] == saves + 1:
            want = {"before": "power cut before the write",
                    "torn": "power cut, torn copy",
                    "after": "power cut after the write"}[cut[1]]
            for o in opts:
                if o[2][-1].endswith(want):
                    nxt = o
        if opts:
            saves += 1
        if nxt is None:
            outs = list(M.boot(st, 0, None))
            assert len(outs) == 1, outs
            nxt = outs[0]
        st, _, _, kind = nxt
        tr.append((kind, M.vars_of(st).get("mp2s_left")))
        if kind == "linux" and M.settled(st):
            break
    return tr, norm_model(M.vars_of(st)), saves


checks = failed = 0
for flags in ((5, 6), (255, 0)):
    for n in range(1, 10):
        nsave = sim_trace(n, flags, None)[2]
        cuts = [None] + [(i, k) for i in range(1, nsave + 1)
                         for k in ("before", "torn", "after")]
        for cut in cuts:
            checks += 1
            name = "N=%d flags=%d,%d cut=%s" % (
                n, flags[0], flags[1],
                "none" if not cut else "saveenv %d %s" % cut)
            sa = sim_trace(n, flags, cut)
            mo = model_trace(n, flags, cut)
            if sa[0] != mo[0] or sa[1] != mo[1]:
                print("DISAGREE %s:\n  sim   %s %s\n  model %s %s"
                      % (name, sa[0], sa[1], mo[0], mo[1]))
                failed += 1
            else:
                print("ok %s: %d boots" % (name, len(sa[0])))
print("not comparable: model failed-write faults (sim's saveenv-fails is a whole-run switch)")
print("not comparable: model both-copies-lost faults (the sim has no such cut)")
print("not comparable: model N=0 and forever (sim --left takes 1-9; forever is the model's own check)")
print("compared (N, cut) cases: %d (N 1-9, two flag starts, clean run included)" % checks)
print("mp2s_model_sim_diff_selftest: %d checks, %d failed" % (checks, failed))
sys.exit(1 if failed else 0)
PY
