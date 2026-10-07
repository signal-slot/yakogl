#!/usr/bin/env python3
"""Make the extracted Zephyr tree a git repo with refs/heads/manifest-rev at
HEAD, so west manifest imports (west list, zephyr_module.py) work. The tree is
the af17c0c8aa2a82a0b714a423915ad3610063ee68 content from the codeload tarball;
west reads the imported west.yml from manifest-rev, it does not compare SHAs."""
import subprocess, os

WS = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/zephyr-ws"
Z = WS + "/zephyr"
GIT = ["git", "-C", Z, "-c", "user.name=ccfarm", "-c", "user.email=ccfarm@invalid"]

def run(cmd, **kw):
    print("+", " ".join(cmd), flush=True)
    r = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if r.returncode != 0:
        print("rc", r.returncode, r.stdout[-2000:], r.stderr[-2000:], flush=True)
        raise SystemExit("FAILED: " + " ".join(cmd))
    if r.stdout.strip():
        print(r.stdout.strip()[:1500], flush=True)
    return r

if not os.path.exists(Z + "/.git"):
    run(GIT[:3] + ["init", "-q", "-b", "manifest-rev"])
run(GIT + ["add", "-A"])
run(GIT + ["commit", "-q", "-m",
           "Zephyr af17c0c8aa2a82a0b714a423915ad3610063ee68 (codeload tarball import)"])
run(GIT + ["branch", "-f", "manifest-rev", "HEAD"])
run(GIT + ["rev-parse", "manifest-rev"])
r = run(GIT + ["ls-tree", "refs/heads/manifest-rev", "west.yml"])

# now west list + zephyr-modules.sh from the yakogl copy
env = dict(os.environ,
           PATH="/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/venv/bin:" + os.environ["PATH"],
           ZEPHYR_BASE=Z)
r = subprocess.run(["west", "list", "-f", "{name} {abspath}"], cwd=WS, capture_output=True, text=True, env=env)
print("west list rc", r.returncode, flush=True)
print(r.stdout, flush=True)
print(r.stderr[-1500:], flush=True)
r = subprocess.run(["bash", WS + "/yakogl/tools/cts/zephyr-modules.sh", WS],
                   capture_output=True, text=True, env=env)
print("zephyr-modules.sh rc", r.returncode, flush=True)
print(r.stdout, flush=True)
print(r.stderr, flush=True)
print("FIX DONE", flush=True)
