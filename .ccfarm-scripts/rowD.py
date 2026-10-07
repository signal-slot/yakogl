#!/usr/bin/env python3
"""Prove stage 1 usable: build row D of tools/cts/pi4-full-tree.txt with
tools/cts/pi4-deqp-images.sh (README section 2d), into a new directory under
TMPDIR (TMPDIR is unset on this host and /tmp is outside the allowed
directories, so it points at a scratch dir in the deps cache).

Prints the D.built line and the check-deqp-images.py verdict from D.log.
"""
import subprocess, os, time, shutil, sys

ROOT = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b"
WS = ROOT + "/zephyr-ws"
YK = WS + "/yakogl"
VENV = ROOT + "/venv"
WORK = ROOT + "/work"
LOGDIR = ROOT + "/logs"
TMP = ROOT + "/tmp"
OUT = TMP + "/deqp-D"

os.makedirs(TMP, exist_ok=True)
if os.path.exists(OUT):
    shutil.rmtree(OUT)

env = dict(os.environ)
env.update({
    "PATH": VENV + "/bin:/home/tasuku/.cargo/bin:" + env["PATH"],
    "TMPDIR": TMP,
    "ZEPHYR_WS": WS,
    "DEQP_WORK": WORK,
    "WEST_VENV": VENV,
    "ZEPHYR_SDK_INSTALL_DIR": ROOT + "/sdk/zephyr-sdk-1.0.1",
    "RUSTUP_TOOLCHAIN": "nightly",
})

log = open(LOGDIR + "/rowD.log", "w")
t0 = time.time()
r = subprocess.run(["bash", YK + "/tools/cts/pi4-deqp-images.sh",
                    YK + "/tools/cts/pi4-full-tree.txt", OUT, "D"],
                   cwd=YK, env=env, stdout=log, stderr=subprocess.STDOUT)
dt = time.time() - t0
log.close()
print("pi4-deqp-images.sh rc", r.returncode, "elapsed %.1f s" % dt, flush=True)

built = OUT + "/D.built"
if os.path.exists(built):
    print("D.built:", open(built).read().strip(), flush=True)
else:
    print("NO D.built", flush=True)

dlog = OUT + "/D.log"
if os.path.exists(dlog):
    s = open(dlog, errors="replace").read()
    lines = s.splitlines()
    print("--- D.log tail ---", flush=True)
    print("\n".join(lines[-25:]), flush=True)
    for i, l in enumerate(lines):
        if "check-deqp-images" in l:
            print("\n".join(lines[i:i+6]), flush=True)
            break

du = subprocess.run(["du", "-sh", OUT], capture_output=True, text=True).stdout.strip()
print("du:", du, flush=True)
print("ROWD DONE rc", r.returncode, flush=True)
raise SystemExit(r.returncode)
