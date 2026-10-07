#!/usr/bin/env python3
"""Extract the downloaded archives into the stage-1 layout.

Layout under $CCFARM_DEPS_DIR/yakogl/deqp-stage1-rpi_4b:
  cts-src/                 VK-GL-CTS tree at 93bca018 (from tarball)
    external/<name>/src    the fetch_sources.py default set, extracted in place
  zephyr-ws/zephyr/        Zephyr at af17c0c8aa2
"""
import tarfile, os, shutil, sys

DL = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/dl"
ROOT = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b"

def extract_single_top(path, dest):
    """Extract a tarball whose entries share one top-level directory into dest."""
    os.makedirs(dest, exist_ok=True)
    with tarfile.open(path) as tf:
        names = tf.getnames()
        tops = {n.split('/')[0] for n in names}
        assert len(tops) == 1, (path, tops)
        top = tops.pop()
        tmp = dest + ".extract-tmp"
        if os.path.exists(tmp):
            shutil.rmtree(tmp)
        os.makedirs(tmp)
        try:
            tf.extractall(tmp, filter='data')
        except TypeError:
            tf.extractall(tmp)
        src = os.path.join(tmp, top)
        for entry in os.listdir(src):
            s = os.path.join(src, entry)
            d = os.path.join(dest, entry)
            if os.path.exists(d):
                raise SystemExit(f"{d} exists already")
            shutil.move(s, d)
        shutil.rmtree(tmp)
    print("extracted", path, "->", dest, flush=True)

# 1. CTS main tree
extract_single_top(os.path.join(DL, "VK-GL-CTS.tar.gz"), os.path.join(ROOT, "cts-src"))

# 2. external sources, per fetch_sources.py PACKAGES (defaults, no VVL)
EXT = os.path.join(ROOT, "cts-src", "external")
jobs = [
    ("zlib-1.2.13.tar.gz", "zlib"),
    ("libpng-1.6.50.tar.gz", "libpng"),
    ("spirv-tools-14521db.tar.gz", "spirv-tools"),
    ("glslang-ebe60e3.tar.gz", "glslang"),
    ("spirv-headers-4965431.tar.gz", "spirv-headers"),
    ("vulkan-docs-f84d432.tar.gz", "vulkan-docs"),
    ("amber-fee69c1.tar.gz", "amber"),
    ("jsoncpp-89e2973.tar.gz", "jsoncpp"),
    ("vulkansc-pcutil-f67d9e8.tar.gz", "vulkansc-pcutil"),
    ("vulkan-video-samples-v0.5.0.tar.gz", "vulkan-video-samples"),
    ("video_generator-v0.1.1.tar.gz", "video_generator"),
]
for name, base in jobs:
    extract_single_top(os.path.join(DL, name), os.path.join(EXT, base, "src"))

# libpng postExtract: pnglibconf.h from scripts/pnglibconf.h.prebuilt
lp = os.path.join(EXT, "libpng", "src")
shutil.copy(os.path.join(lp, "scripts", "pnglibconf.h.prebuilt"), os.path.join(lp, "pnglibconf.h"))
print("libpng pnglibconf.h copied", flush=True)

# renderdoc single header
os.makedirs(os.path.join(EXT, "renderdoc", "src"), exist_ok=True)
shutil.copy(os.path.join(DL, "renderdoc_app.h"), os.path.join(EXT, "renderdoc", "src", "renderdoc_app.h"))
print("renderdoc_app.h copied", flush=True)

# 3. Zephyr
extract_single_top(os.path.join(DL, "zephyr.tar.gz"), os.path.join(ROOT, "zephyr-ws", "zephyr"))

print("EXTRACT ALL DONE", flush=True)
