<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 4: the 33 glmark2-es2 pictures, looked at by eye

The Pi 4's column of `tools/cts/glmark2-pictures.txt` is a regression baseline
that nobody has looked at. This note is the record of looking at each of the 33
scene pictures, without a camera, from the image's own frame dump. It closes the
Pi 4 part of "Pictures by eye" in `docs/notes/stage2-bench-gap.md` ("Left"),
issue #1196.

## How

- Images are built only from origin/main commits; the commit of each row is in
  the table.
- `tools/cts/pi4-glmark2-dumps.sh` builds and runs them: a Pi 4 GM image with
  `CONFIG_GLMARK2_FRAME_DUMP=y` and one scene in `CONFIG_GLMARK2_ARGS`. The
  800x600 window is printed as `glmark2-fb:` lines on the UART
  (`driver/docs/glmark2.md`, "The frame dump") and decoded with
  `tools/cts/glmark2-fb.py`, which checks the row crc16s, the frame crc32 and
  the scene's `glmark2-picture` line.
- A dump is about 1.9 MB and takes a few minutes at 115200 baud, one scene per
  image.
- A damaged dump is rerun; pixels are never patched.
- Each decoded picture is compared with the upstream glmark2 picture on Mesa
  llvmpipe made by `tools/cts/glmark2-host-ref.sh`. Animated scenes (two
  llvmpipe runs differ) are compared by content only.
- The records are `records/pi4/<YYYYMMDD-HHMM>-<name>-<sha8>/` (gitignored); the
  record column gives the directory name.

## Verdicts

| scene | verdict | what was looked at | origin/main commit | record |
|---|---|---|---|---|
| [build] use-vbo=false | pending | | | |
| [build] use-vbo=true | pending | | | |
| [texture] texture-filter=nearest | pending | | | |
| [texture] texture-filter=linear | pending | | | |
| [texture] texture-filter=mipmap | pending | | | |
| [shading] shading=gouraud | pending | | | |
| [shading] shading=blinn-phong-inf | pending | | | |
| [shading] shading=phong | pending | | | |
| [shading] shading=cel | pending | | | |
| [bump] bump-render=high-poly | pending | | | |
| [bump] bump-render=normals | pending | | | |
| [bump] bump-render=height | pending | | | |
| [effect2d] kernel=0,1,0;1,-4,1;0,1,0; | pending | | | |
| [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1; | pending | | | |
| [pulsar] light=false:quads=5:texture=false | pending | | | |
| [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4 | pending | | | |
| [desktop] effect=shadow:windows=4 | pending | | | |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map | pending | | | |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata | pending | | | |
| [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map | pending | | | |
| [ideas] speed=duration | pending | | | |
| [jellyfish] <default> | pending | | | |
| [terrain] <default> | pending | | | |
| [shadow] <default> | pending | | | |
| [refract] <default> | pending | | | |
| [conditionals] fragment-steps=0:vertex-steps=0 | pending | | | |
| [conditionals] fragment-steps=5:vertex-steps=0 | pending | | | |
| [conditionals] fragment-steps=0:vertex-steps=5 | pending | | | |
| [function] fragment-complexity=low:fragment-steps=5 | pending | | | |
| [function] fragment-complexity=medium:fragment-steps=5 | pending | | | |
| [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5 | pending | | | |
| [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5 | pending | | | |
| [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5 | pending | | | |
