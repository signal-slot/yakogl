#!/usr/bin/env python3
"""Assemble the west workspace and venv for the dEQP stage-1 build.

Under $CCFARM_DEPS_DIR/yakogl/deqp-stage1-rpi_4b:
  venv/                 WEST_VENV: west, cmake<4 (zlib in CTS pins cmake 2.4.4,
                        which CMake >= 4 rejects), Zephyr build requirements
  cts-src/              VK-GL-CTS tree at 93bca018, made a git repo (HEAD commit)
  zephyr-ws/            west workspace: .west/config, yakogl (repo copy),
                        zephyr (af17c0c8), modules/{fs/fatfs,hal/nxp,hal/ti,lib/picolibc}
"""
import subprocess, sys, os, shutil

ROOT = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b"
WS = os.path.join(ROOT, "zephyr-ws")
VENV = os.path.join(ROOT, "venv")
REPO_SRC = "/home/tasuku/.local/share/ccfarm-local/workspaces/01a11804-3533-777d-b93a-03515e4187da"

def run(cmd, **kw):
    print("+", " ".join(cmd), flush=True)
    r = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if r.returncode != 0:
        print("rc", r.returncode, flush=True)
        print(r.stdout[-4000:], flush=True)
        print(r.stderr[-4000:], flush=True)
        raise SystemExit(f"FAILED: {cmd[0]}")
    if r.stdout.strip():
        print(r.stdout.strip()[-2000:], flush=True)
    return r

# 1. venv
if not os.path.exists(VENV + "/bin/pip"):
    run([sys.executable, "-m", "venv", VENV])
run([VENV + "/bin/pip", "install", "-q", "--disable-pip-version-check",
     "cmake<4", "west", "pyelftools", "PyYAML", "pykwalify", "jsonschema",
     "packaging", "intelhex", "colorama", "ply", "python-dotenv",
     "psutil", "tqdm", "semver", "requests", "anytree"])
run([VENV + "/bin/python", "-c", "import yaml,elftools,west; import pykwalify; print('venv ok')"])
run([VENV + "/bin/cmake", "--version"])
run([VENV + "/bin/west", "--version"])

# 2. cts-src as a git repo at one commit (cts-build-zephyr.sh clones from it)
CTS = os.path.join(ROOT, "cts-src")
if not os.path.exists(CTS + "/.git"):
    run(["git", "-C", CTS, "init", "-q", "-b", "main"])
run(["git", "-C", CTS, "-c", "user.name=ccfarm", "-c", "user.email=ccfarm@invalid",
     "add", "-A"])
r = subprocess.run(["git", "-C", CTS, "rev-parse", "HEAD"], capture_output=True, text=True)
if r.returncode != 0:
    run(["git", "-C", CTS, "-c", "user.name=ccfarm", "-c", "user.email=ccfarm@invalid",
         "commit", "-q", "-m", "VK-GL-CTS 93bca01861b0e3ef3c387027a9791e6d065f900c (codeload tarball import)"])
r = run(["git", "-C", CTS, "rev-parse", "HEAD"])
print("CTS HEAD:", r.stdout.strip(), flush=True)

# 3. yakogl copy + .west/config
YK = os.path.join(WS, "yakogl")
if not os.path.exists(YK):
    shutil.copytree(REPO_SRC, YK, ignore=shutil.ignore_patterns(
        ".git", ".ccfarm-workspace.md", ".ccfarm-scripts", ".target", "build", "*.built"))
    print("copied yakogl ->", YK, flush=True)
cfg = os.path.join(WS, ".west", "config")
os.makedirs(os.path.dirname(cfg), exist_ok=True)
with open(cfg, "w") as f:
    f.write("[manifest]\npath = yakogl\nfile = west.yml\n\n[zephyr]\nbase = zephyr\n")
print("wrote", cfg, flush=True)

# 4. west list + zephyr-modules.sh from the copy
env = dict(os.environ, PATH=VENV + "/bin:" + os.environ["PATH"], ZEPHYR_BASE=os.path.join(WS, "zephyr"))
r = subprocess.run(["west", "list", "-f", "{name} {abspath}"], cwd=WS, capture_output=True, text=True, env=env)
print("west list:", r.returncode, flush=True)
print(r.stdout, flush=True)
print(r.stderr[-1500:], flush=True)
r = subprocess.run(["bash", os.path.join(YK, "tools/cts/zephyr-modules.sh"), WS],
                   capture_output=True, text=True, env=env)
print("zephyr-modules.sh rc", r.returncode, flush=True)
print(r.stdout, flush=True)
print(r.stderr, flush=True)
print("SETUP DONE", flush=True)
