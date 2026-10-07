#!/usr/bin/env python3
"""Fetch the CTS checkout at 93bca01861b0e3ef3c387027a9791e6d065f900c and the
external sources its external/fetch_sources.py pins (default set, without
vulkan-validationlayers), plus the pinned Zephyr tree. Downloads only; no
extraction here. Files land under dl/ in the stage-1 deps directory."""
import subprocess, sys, os

HERE = os.path.dirname(os.path.abspath(__file__))
DL = "/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/dl"

CTS_SHA = "93bca01861b0e3ef3c387027a9791e6d065f900c"
ZEPHYR_SHA = "af17c0c8aa2a82a0b714a423915ad3610063ee68"

ITEMS = [
    ("VK-GL-CTS.tar.gz", f"https://codeload.github.com/KhronosGroup/VK-GL-CTS/tar.gz/{CTS_SHA}"),
    ("zephyr.tar.gz", f"https://codeload.github.com/zephyrproject-rtos/zephyr/tar.gz/{ZEPHYR_SHA}"),
    # external/fetch_sources.py PACKAGES (defaults)
    ("zlib-1.2.13.tar.gz", "https://github.com/madler/zlib/releases/download/v1.2.13/zlib-1.2.13.tar.gz"),
    ("libpng-1.6.50.tar.gz", "https://github.com/pnggroup/libpng/archive/refs/tags/v1.6.50.tar.gz"),
    ("renderdoc_app.h", "https://raw.githubusercontent.com/baldurk/renderdoc/v1.1/renderdoc/api/app/renderdoc_app.h"),
    ("spirv-tools-14521db.tar.gz", "https://codeload.github.com/KhronosGroup/SPIRV-Tools/tar.gz/14521db96e91200a0ef7cc50e6df331e71282a6b"),
    ("glslang-ebe60e3.tar.gz", "https://codeload.github.com/KhronosGroup/glslang/tar.gz/ebe60e331c21d5c7f09734fbdcebbc46ee5d8507"),
    ("spirv-headers-4965431.tar.gz", "https://codeload.github.com/KhronosGroup/SPIRV-Headers/tar.gz/496543121ce6419f23d6fa5d7194ba66c36212d2"),
    ("vulkan-docs-f84d432.tar.gz", "https://codeload.github.com/KhronosGroup/Vulkan-Docs/tar.gz/f84d432d5b8912362f96f581f29bbc4f3c8c7843"),
    ("amber-fee69c1.tar.gz", "https://codeload.github.com/google/amber/tar.gz/fee69c1bc200386987961fc384cc80be293393ff"),
    ("jsoncpp-89e2973.tar.gz", "https://codeload.github.com/open-source-parsers/jsoncpp/tar.gz/89e2973c754a9c02a49974d839779b151e95afd6"),
    ("vulkansc-pcutil-f67d9e8.tar.gz", "https://codeload.github.com/KhronosGroup/VulkanSC-pcutil/tar.gz/f67d9e8e1c04f179ebeb837d21ab06192ad43fa0"),
    ("vulkan-video-samples-v0.5.0.tar.gz", "https://codeload.github.com/KhronosGroup/Vulkan-Video-Samples/tar.gz/v0.5.0"),
    ("video_generator-v0.1.1.tar.gz", "https://codeload.github.com/Igalia/video_generator/tar.gz/v0.1.1"),
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
