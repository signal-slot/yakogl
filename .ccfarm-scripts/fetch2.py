#!/usr/bin/env python3
"""Fetch the Zephyr SDK 1.0.1 aarch64 parts and the west.yml-pinned modules."""
import subprocess, sys, os

HERE = os.path.dirname(os.path.abspath(__file__))
DL = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/dl"

ITEMS = [
    ("sdk-1.0.1-minimal.tar.xz", "https://github.com/zephyrproject-rtos/sdk-ng/releases/download/v1.0.1/zephyr-sdk-1.0.1_linux-x86_64_minimal.tar.xz"),
    ("sdk-1.0.1-aarch64.tar.xz", "https://github.com/zephyrproject-rtos/sdk-ng/releases/download/v1.0.1/toolchain_gnu_linux-x86_64_aarch64-zephyr-elf.tar.xz"),
    ("fatfs-f4ead3b.tar.gz", "https://codeload.github.com/zephyrproject-rtos/fatfs/tar.gz/f4ead3bf4a6dab3a07d7b5f5315795c073db568d"),
    ("hal_nxp-1e91007b.tar.gz", "https://codeload.github.com/zephyrproject-rtos/hal_nxp/tar.gz/1e91007b164c52b74d58d4e75a856efbc5d4de05"),
    ("hal_ti-d9d5b1c.tar.gz", "https://codeload.github.com/zephyrproject-rtos/hal_ti/tar.gz/d9d5b1c9a72b21f9fc30f5815be2232b081e650c"),
    ("picolibc-0125493.tar.gz", "https://codeload.github.com/zephyrproject-rtos/picolibc/tar.gz/01254932e8e81085817ed61fd858648584ffe37c"),
]

for name, url in ITEMS:
    dest = os.path.join(DL, name)
    if os.path.exists(dest) and os.path.getsize(dest) > 0:
        print(f"skip (exists) {name}", flush=True)
        continue
    print(f"== {name}", flush=True)
    r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "fetch.py"), url, dest])
    if r.returncode != 0:
        print(f"FAILED {name}", flush=True)
        sys.exit(1)
print("ALL DONE", flush=True)
