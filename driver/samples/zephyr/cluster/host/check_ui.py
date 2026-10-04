#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Build-time checks of the cluster sample's UI against what the image embeds.
CMakeLists.txt runs this right after the cargo build; a failure fails the
build.

  check_ui.py <cluster-ui-dir> <cargo-target-dir> [--variant]

1. Every text size the UI declares has its own embedded glyph set. The UI's
   sizes are fractional (82.5 px) and both the compiler (embed_glyphs.rs,
   `as i16`) and the FemtoVG path (bitmapfont.rs, FontLength = i16) truncate;
   the FemtoVG path then draws with the largest embedded size not above the
   request, scaled up. A size whose truncation is not embedded is drawn from
   a smaller one (CLUSTER1: 82.5 px from 65 px glyphs, 64.69 px from 34 px).
   The sizes are read from the .slint files (every `size`, `font-size`,
   `value-size` and `unit-size` given in px) and each BitmapFont in the
   generated code must hold every truncation.
2. The gradient-stroked Paths are the ones host/ref's blind spot names. The
   host reference (Slint's software renderer) strokes a Path with a single
   colour (internal/renderers/software/lib.rs:3186, brush.color()), so the
   probes skip the pixels such a Path covers (host/ref/src/main.rs,
   gradient_strokes()). A new gradient stroke or fill in the UI fails here,
   so the list is updated with it rather than silently probed.

--variant: the UI is a CONFIG_V3D_GL_CLUSTER_OMIT copy (host/ui_omit.py)
with parts left out. Check 1 still holds for the sizes it uses; its two
known members (17.81 px, 82.5 px) are required only when they are in it,
and check 2's count is printed but not required (a variant's frame is not
compared with host/ref).
"""
import glob
import os
import re
import sys

SIZE_RE = re.compile(r"(?:^|[\s{;>])(size|font-size|value-size|unit-size)\s*:\s*([0-9.]+)px")
GRADIENT_PATH_RE = re.compile(r"\b(stroke-brush|stroke|fill)\s*:\s*@(linear|radial|conic)-gradient")
# The gradient-stroked Path properties host/ref's gradient_strokes() covers:
# app-window.slint's left-dial power arcs and right-dial speed arc.
KNOWN_GRADIENT_PATHS = 2


def main():
    if len(sys.argv) not in (3, 4) or (len(sys.argv) == 4 and sys.argv[3] != "--variant"):
        print(__doc__)
        return 2
    ui_dir, target = sys.argv[1], sys.argv[2]
    variant = len(sys.argv) == 4
    sizes = []
    gradient_paths = []
    for path in sorted(glob.glob(os.path.join(ui_dir, "*.slint"))):
        for n, line in enumerate(open(path), 1):
            for m in SIZE_RE.finditer(line):
                sizes.append(float(m.group(2)))
            for m in GRADIENT_PATH_RE.finditer(line):
                gradient_paths.append(f"{os.path.basename(path)}:{n}: {line.strip()}")
    # the instrument must see what is there: the literal 17.81 px (READY) and
    # the speed digits' 82.5 px are known members
    if not variant and (17.81 not in sizes or 82.5 not in sizes):
        print(f"check_ui: the size scan missed a known size (read {sorted(set(sizes))})")
        return 1
    if variant:
        print(f"check_ui: a variant UI (parts left out): the known sizes 17.81 px and 82.5 px are "
              f"{'present' if 17.81 in sizes and 82.5 in sizes else 'not all in it'}; "
              f"{len(set(sizes))} distinct sizes read")
        if not sizes:
            print("check_ui: a variant with no text sizes at all: the scan cannot be told from a blind one")
    want = sorted({int(s) for s in sizes})

    outs = glob.glob(os.path.join(target, "*", "release", "build", "slint_cluster-*", "out", "zephyr-cluster.rs"))
    if not outs:
        print(f"check_ui: no generated zephyr-cluster.rs under {target}")
        return 1
    gen = max(outs, key=os.path.getmtime)
    text = open(gen).read()
    starts = [m.start() for m in re.finditer(r"BitmapFont\s*=\s*sp\s*::\s*BitmapFont\s*\{", text)]
    if not starts:
        print(f"check_ui: {gen} holds no BitmapFont")
        return 1
    bad = 0
    for i, st in enumerate(starts):
        blk = text[st:(starts[i + 1] if i + 1 < len(starts) else len(text))]
        weight = re.search(r"weight\s*:\s*(\d+)u16", blk).group(1)
        have = {int(x) for x in re.findall(r"pixel_size\s*:\s*(\d+)i16", blk)}
        missing = [s for s in want if s not in have]
        print(f"check_ui: weight {weight}: {len(have)} sizes embedded, the UI's {len(want)} truncated sizes "
              f"{'all present' if not missing else 'MISSING ' + str(missing)}")
        bad += bool(missing)

    print(f"check_ui: {len(gradient_paths)} gradient-stroked Path properties (host/ref's blind spot names "
          f"{KNOWN_GRADIENT_PATHS})")
    for g in gradient_paths:
        print(f"  {g}")
    if len(gradient_paths) != KNOWN_GRADIENT_PATHS and variant:
        print("check_ui: a variant UI: the count is not required (its frame is not compared with host/ref)")
    elif len(gradient_paths) != KNOWN_GRADIENT_PATHS:
        print("check_ui: the UI's gradient strokes changed: update host/ref's gradient_strokes() and this count")
        bad += 1
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
