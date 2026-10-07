#!/usr/bin/env python3
"""Run tools/cts/cts-build-zephyr.sh for rpi_4b (dEQP stage 1).

The script is the repository's own, invoked with the environment README
section 2d names. Output is streamed to logs/stage1.log; wall time and disk
use are recorded for the session note.
"""
import subprocess, os, time, shutil

ROOT = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b"
WS = ROOT + "/zephyr-ws"
YK = WS + "/yakogl"
VENV = ROOT + "/venv"
WORK = ROOT + "/work"
LOGDIR = ROOT + "/logs"
os.makedirs(LOGDIR, exist_ok=True)

env = dict(os.environ)
env.update({
    # ~/.cargo/bin before the kache cargo shim: capi/yakogl_capi.cmake needs a
    # cargo with rustc next to it (the same choice tools/hostharness/glmark2/
    # run.sh:169-174 makes by scanning PATH for a dir with both).
    "PATH": VENV + "/bin:/home/tasuku/.cargo/bin:" + env["PATH"],
    "CTS": ROOT + "/cts-src",
    "WORK": WORK,
    "BOARD": "rpi_4b",
    "MODULE": "gles2",
    "ZEPHYR_BASE": WS + "/zephyr",
    "ZEPHYR_SDK": ROOT + "/sdk/zephyr-sdk-1.0.1",
    "ZEPHYR_SDK_INSTALL_DIR": ROOT + "/sdk/zephyr-sdk-1.0.1",
    "RUSTUP_TOOLCHAIN": "nightly",
})
mods = subprocess.run(["bash", YK + "/tools/cts/zephyr-modules.sh", WS],
                      capture_output=True, text=True, env=env)
if mods.returncode != 0:
    raise SystemExit("zephyr-modules.sh failed: " + mods.stderr)
env["ZEPHYR_MODULES"] = mods.stdout.strip()
print("ZEPHYR_MODULES:", env["ZEPHYR_MODULES"], flush=True)

log = open(LOGDIR + "/stage1.log", "w")
t0 = time.time()
r = subprocess.run(["sh", YK + "/tools/cts/cts-build-zephyr.sh"],
                   cwd=YK, env=env, stdout=log, stderr=subprocess.STDOUT)
dt = time.time() - t0
log.close()
print("stage1 rc", r.returncode, "elapsed %.1f s" % dt, flush=True)
print(open(LOGDIR + "/stage1.log").read()[-3000:], flush=True)

def du(path):
    r = subprocess.run(["du", "-sh", path], capture_output=True, text=True)
    return r.stdout.strip()
for p in [WORK, WORK + "/cts", WORK + "/build", WORK + "/flags"]:
    if os.path.exists(p):
        print("du:", du(p), flush=True)
print("STAGE1 DONE rc", r.returncode, flush=True)
raise SystemExit(r.returncode)
