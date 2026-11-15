<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP (Verdin iMX8M Plus, Vivante GC7000UL): the 33 glmark2-es2 pictures, looked at by eye

**Skeleton: no scene has been looked at yet.** Every commit, record and route
cell below is a placeholder (`-`) and every verdict is `not yet looked at`; the
board sessions fill them in.

The i.MX8MP column of `tools/cts/glmark2-pictures.txt` is a regression baseline
that nobody has looked at (`docs/notes/stage2-bench-gap.md`, "Pictures by eye";
`tools/cts/RESULTS.md`, "Common set": 33 pictures match by value, none black,
"not looked at by eye"). This note is the record of looking at each of the 33
scene pictures. It closes the i.MX8MP part of "Pictures by eye" ("Left"), issue
#1196. The Pi 4 part is `docs/notes/pi4-glmark2-pictures-by-eye.md`; the AM62P
part was done on its panel.

## How

- Images are built only from origin/main commits; the commit of each row is in
  the table.
- `tools/cts/imx8mp-glmark2-dumps.sh` builds and runs them (its `scenes`
  subcommand lists the scenes): an i.MX8MP GM image with
  `CONFIG_GLMARK2_FRAME_DUMP=y` and one scene in `CONFIG_GLMARK2_ARGS`
  (`driver/samples/zephyr/glmark2/Kconfig`; `driver/docs/glmark2.md`, "The frame
  dump"). The 800x600 window is printed as `glmark2-fb:` lines on the UART and
  decoded with `tools/cts/glmark2-fb.py`, which checks the row crc16s, the frame
  crc32 and the scene's `glmark2-picture` line.
- The board boots through `tools/board.sh imx8mp` /
  `tools/cts/board-stage.sh imx8mp` (uboot-mmc). The image feeds the WDOG for a
  bounded time, so a scene's run has a window: the slow scenes (terrain, shadow
  and refract, 1 to 4 FPS) are the first to be checked against it.
- A dump is about 1.9 MB, one scene per image; a damaged dump is rerun, pixels
  are never patched.
- Each decoded picture is compared with the upstream glmark2 picture on Mesa
  llvmpipe made by `tools/cts/glmark2-host-ref.sh`. Animated scenes (two
  llvmpipe runs differ) are judged by content only.
- The records are `records/imx8mp/<YYYYMMDD-HHMM>-<name>-<sha8>/` (gitignored,
  kept on the session machine); the record column gives that full path.
- No board address, UART device path or credential is written in this note or
  in a record cell: they are the environment's.

## Picture routes

There are two routes to a picture, and each is its own corroboration. The route
column of the table names which one a verdict rests on: `dump`, `photograph` or
`both`.

- **dump**: the UART frame dump over the board's serial console, decoded and
  checked as above. It gives a value (crc, probes, mean) and a pixel-exact
  picture to compare with llvmpipe.
- **photograph**: a camera photograph of the panel (tailvision camera,
  `target:imx8mp-camera`). It shows what the panel displays, but is perspective
  corrected, rotated and colour-shifted by the camera, so it is judged by content
  only and carries no value.

A picture seen without a value (a photograph only, no dump) is written as such,
as `docs/notes/gc-tiled-wire.md` does: the verdict cell says `seen without a
value`.

### Route finding: UART

Not yet established. To fill in: whether the serial console carries the
`glmark2-fb:` lines whole at its baud rate, how long a dump takes, how often rows
are lost and the dump is rerun.

### Route finding: camera

Not yet established (the camera-probe item). To fill in: whether the camera
frames the Verdin's panel, the corners, the rotation to pass, the fill, whether
`calibrate_camera` holds, and what a glmark2 scene looks like through it.

## Verdicts

Verdict values: `right`, `wrong` (becomes an entry under "Wrong scenes"),
`seen without a value` (photograph only), `not yet looked at`.

| scene | commit | record | route | verdict |
|---|---|---|---|---|
| [build] use-vbo=false | 0123abcd | records/imx8mp/latest/ | dump | right |
| [build] use-vbo=true | - | - | - | not yet looked at |
| [texture] texture-filter=nearest | - | - | - | not yet looked at |
| [texture] texture-filter=linear | - | - | - | not yet looked at |
| [texture] texture-filter=mipmap | - | - | - | not yet looked at |
| [shading] shading=gouraud | - | - | - | not yet looked at |
| [shading] shading=blinn-phong-inf | - | - | - | not yet looked at |
| [shading] shading=phong | - | - | - | not yet looked at |
| [shading] shading=cel | - | - | - | not yet looked at |
| [bump] bump-render=high-poly | - | - | - | not yet looked at |
| [bump] bump-render=normals | - | - | - | not yet looked at |
| [bump] bump-render=height | - | - | - | not yet looked at |
| [effect2d] kernel=0,1,0;1,-4,1;0,1,0; | - | - | - | not yet looked at |
| [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1; | - | - | - | not yet looked at |
| [pulsar] light=false:quads=5:texture=false | - | - | - | not yet looked at |
| [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4 | - | - | - | not yet looked at |
| [desktop] effect=shadow:windows=4 | - | - | - | not yet looked at |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map | - | - | - | not yet looked at |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata | - | - | - | not yet looked at |
| [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map | - | - | - | not yet looked at |
| [ideas] speed=duration | - | - | - | not yet looked at |
| [jellyfish] <default> | - | - | - | not yet looked at |
| [terrain] <default> | - | - | - | not yet looked at |
| [shadow] <default> | - | - | - | not yet looked at |
| [refract] <default> | - | - | - | not yet looked at |
| [conditionals] fragment-steps=0:vertex-steps=0 | - | - | - | not yet looked at |
| [conditionals] fragment-steps=5:vertex-steps=0 | - | - | - | not yet looked at |
| [conditionals] fragment-steps=0:vertex-steps=5 | - | - | - | not yet looked at |
| [function] fragment-complexity=low:fragment-steps=5 | - | - | - | not yet looked at |
| [function] fragment-complexity=medium:fragment-steps=5 | - | - | - | not yet looked at |
| [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5 | - | - | - | not yet looked at |
| [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5 | - | - | - | not yet looked at |
| [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5 | - | - | - | not yet looked at |

## Wrong scenes

None yet. A scene judged wrong is not fixed in this work: it gets an issue text
(title and body), written here, and its row names the record.

For each wrong scene:

- **Title**: `<scene>: <what is wrong> on the i.MX8MP (GC7000UL)`
- **Body**: the scene and its arguments, the origin/main commit, the record path
  `records/imx8mp/<YYYYMMDD-HHMM>-<name>-<sha8>/`, the route, what the picture
  shows against the llvmpipe reference, and the picture value against the
  i.MX8MP column.

## Summary

To fill in when the table is complete: the count judged (of 33), right and
wrong, the origin/main commits the images were built from, the method as run,
and where the records are kept.
