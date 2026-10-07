#!/usr/bin/env python3
"""Extract the SDK 1.0.1 parts and the west.yml-pinned Zephyr modules.

Layout under $CCFARM_DEPS_DIR/yakogl/deqp-stage1-rpi_4b:
  sdk/zephyr-sdk-1.0.1/          minimal SDK + gnu/aarch64-zephyr-elf merged
  zephyr-ws/modules/...          fatfs, hal/nxp, hal/ti, lib/picolibc
"""
import tarfile, os, shutil

DL = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/dl"
ROOT = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b"
SDK = os.path.join(ROOT, "sdk", "zephyr-sdk-1.0.1")

def top_of(path):
    with tarfile.open(path) as tf:
        tops = {m.name.split('/')[0] for m in tf.getmembers() if m.name}
    assert len(tops) == 1, (path, tops)
    return tops.pop()

def extract_into(path, dest, drop_top):
    """Extract into dest; if drop_top, the single top dir is not kept."""
    tmp = dest + ".extract-tmp"
    if os.path.exists(tmp):
        shutil.rmtree(tmp)
    os.makedirs(tmp)
    with tarfile.open(path) as tf:
        try:
            tf.extractall(tmp, filter='data')
        except TypeError:
            tf.extractall(tmp)
    src = os.path.join(tmp, top_of(path)) if drop_top else tmp
    os.makedirs(dest, exist_ok=True)
    for entry in os.listdir(src):
        s = os.path.join(src, entry)
        d = os.path.join(dest, entry)
        if os.path.exists(d):
            raise SystemExit(f"{d} exists already")
        shutil.move(s, d)
    shutil.rmtree(tmp)
    print("extracted", os.path.basename(path), "->", dest, flush=True)

# 1. minimal SDK
extract_into(os.path.join(DL, "sdk-1.0.1-minimal.tar.xz"), SDK, True)

# 2. aarch64 toolchain: inspect its top dir first
t_top = top_of(os.path.join(DL, "sdk-1.0.1-aarch64.tar.xz"))
print("aarch64 tarball top dir:", t_top, flush=True)
if t_top == "zephyr-sdk-1.0.1":
    extract_into(os.path.join(DL, "sdk-1.0.1-aarch64.tar.xz"), SDK, True)
else:
    extract_into(os.path.join(DL, "sdk-1.0.1-aarch64.tar.xz"), SDK, False)

# 3. modules at their west.yml (Zephyr) paths
MODS = [
    ("fatfs-f4ead3b.tar.gz", "modules/fs/fatfs"),
    ("hal_nxp-1e91007b.tar.gz", "modules/hal/nxp"),
    ("hal_ti-d9d5b1c.tar.gz", "modules/hal/ti"),
    ("picolibc-0125493.tar.gz", "modules/lib/picolibc"),
]
for name, rel in MODS:
    extract_into(os.path.join(DL, name), os.path.join(ROOT, "zephyr-ws", rel), True)

# sanity checks
for p in ["sdk_version", "cmake", "hosttools", "gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc",
          "gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-g++",
          "gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-ar",
          "gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-ranlib"]:
    q = os.path.join(SDK, p)
    print(("OK  " if os.path.exists(q) else "MISS"), p, flush=True)
for name, rel in MODS:
    q = os.path.join(ROOT, "zephyr-ws", rel)
    print(("OK  " if os.path.isdir(q) else "MISS"), rel, flush=True)
print("EXTRACT2 DONE", flush=True)
