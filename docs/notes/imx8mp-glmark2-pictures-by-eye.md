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
column of the table names which one a verdict rests on: `dump`, `photograph`,
`both` or `photograph+probes`.

- **dump**: the UART frame dump over the board's serial console, decoded and
  checked as above. It gives a value (crc, probes, mean) and a pixel-exact
  picture to compare with llvmpipe.
- **photograph**: a camera photograph of the panel (tailvision camera,
  `target:imx8mp-camera`). It shows what the panel displays, but is perspective
  corrected, rotated and colour-shifted by the camera, so it is judged by content
  only and carries no value.
- **photograph+probes**: a camera photograph of the panel judged by content,
  plus the `glmark2-picture:` value line (probes, crc16, mean) that the same
  boot printed on the UART for that scene. It carries the value line, not a
  pixel picture, so the verdict is `right` or `wrong`, never `seen without a
  value`.

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
| [build] use-vbo=false | - | - | - | not yet looked at |
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

## What was looked at

Each of the 33 scenes has one entry here, in `tools/cts/imx8mp-glmark2-dumps.sh
scenes` order with its gm label: what a right picture of that scene shows against
the llvmpipe reference (from the judged Pi 4 pictures, the scene's options, and
the `imx8mp` column with its notes in `tools/cts/glmark2-pictures.txt` and
`tools/cts/RESULTS.md`), then the six board values as placeholders — `<verdict>`,
`<route>`, `<commit>`, `<record>`, `<crc32>` (the dump's frame crc32), `<picdiff>`
— which the board session replaces with the measured ones.

- **[build] use-vbo=false** (gm01-build): the rearing horse, lit grey on black, drawn from client-side arrays; the same pose and shading as llvmpipe; no missing or stray triangles, no holes, no misbuilt body; the reference moves at most a step, so compare within the picture tolerance, and the value judging is against the `imx8mp` column (probe c on the horse's body, corners black, mean 12 12 12); the UART route finding (dump size, time, rows lost and rerun) is made on this scene; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[build] use-vbo=true** (gm02-build): the same horse drawn from VBOs: the picture identical to gm01's and to llvmpipe's; no index or attribute mix-up, no holes — the only difference from gm01 may be speed, never pixels; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[texture] texture-filter=nearest** (gm03-texture): the wood crate (plank front and side panel) sampled by nearest texel: the same crate as llvmpipe; the expected difference is a sparse grid of single pixels at texel boundaries (the texel-selection ties of nearest filtering; the Pi 4 showed 0.1% of pixels); no blur, no swapped or missing face; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[texture] texture-filter=linear** (gm04-texture): the same crate bilinearly filtered: smooth grain, no texel blocks, the whole crate drawn against black (corner probes fall on the background, the lower-left one on the crate's dark edge); verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[texture] texture-filter=mipmap** (gm05-texture): the crate through its mipmap chain: the grain a touch softer than gm04's and even over the whole crate; no shimmer or aliasing in the far planks, no band, seam or tile of a wrong level, no garbage tile, no black face; this scene carries the i.MX8MP's known unexplained value change — the `imx8mp` column has probe tl 000000 where gm03 and gm04 have 181c18 (it arrived with `0d21ab47`, unexplained; `tools/cts/RESULTS.md`, i.MX8MP notes; the mipmap chain offer from GL is issue #1106) — so look at what the tl probe falls on and judge by content whether the picture is an even sub-level haze over the crate (what the Pi 4 judged right) or a dropped or wrong level; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[shading] shading=gouraud** (gm06-shading): the blue cat lit per vertex: the same soft, faintly faceted highlight along the back and the dark underside as llvmpipe, the eyes and ears where they belong; the difference confined to the silhouette and faint triangle edges; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[shading] shading=blinn-phong-inf** (gm07-shading): the glossy cat with the white specular spots on the ear, cheek, back and flank in the same places and sizes as llvmpipe; the difference confined to faint speckle at the highlight fringes where the specular power rounds; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[shading] shading=phong** (gm08-shading): the glossy cat lit per pixel: the bright specular spots on the flank, back, tail, ear and face in the same places and sizes, the smooth falloff around them; the difference confined to speckle at the highlight fringes and the silhouette; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[shading] shading=cel** (gm09-shading): the green cat in three flat tones: the pale highlight bands on the ear, cheek, flank and tail, the dark shadow band under the body and behind the tail, the mid-green body, with the same band edges as llvmpipe; no missing step, no gradient where a step should be; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[bump] bump-render=high-poly** (gm10-bump): the grey rock drawn from the high-polygon model: the same fine facets and relief, the lit upper right and the dark lower left, in the same pose as llvmpipe; no missing or stray triangles, no holes or cracks; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[bump] bump-render=normals** (gm11-bump): the low-polygon rock lit through its normal map: the same pitted relief over the flat faces, the lit upper right and the dark lower left; not a flat-shaded rock (the map not sampled), no inverted or noisy relief, no seam; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[bump] bump-render=height** (gm12-bump): the low-polygon rock with its relief from the height map: the same bumps and pits, the lit upper right and the dark lower left; not a flat-shaded rock, no stair steps or noise in the relief; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[effect2d] kernel=0,1,0;1,-4,1;0,1,0;** (gm13-effect2d): the Linaro logo through the 3x3 Laplacian: black except faint outlines of the lettering and the squares (dark grey, green and blue), as on llvmpipe; no unfiltered logo, no white or saturated edges, no shift; an almost black picture is expected here (column mean 0 0 1); the image must be built with the kernel's `;` written `\;` on the west command line (unescaped, CMake drops them and the image runs another kernel) — check that the picture is edge detection, not gm14's blur; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;** (gm14-effect2d): the Linaro logo on white through the 5x3 box blur: the grey lettering, the green squares and the red, pink and purple squares softened evenly in both directions, as on llvmpipe; no shift, no dark or bright border, no unblurred band; the corners near white, as the column's f8fcf8; built with `\;` like gm13; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[pulsar] light=false:quads=5:texture=false** (gm15-pulsar): the five spinning quads with their red, green, blue and white vertex-colour gradients blended over each other on black, unlit and untextured as the options say (light=false, quads=5, texture=false); no missing quad, no opaque quad, no wrong blend order; animated: the quads can stand a rotation step from llvmpipe's, so judge the quad set and the blends, not the exact edges, and the centre probe (column 700070) is a blend over the quads; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4** (gm16-desktop): the Linaro desktop with the four translucent "Tux" windows (green title bars, Tux on grey), the desktop blurred behind each window — the back Tux and the "L" soft through the front window, the purple square soft through the lower one; no unblurred, black or garbage window background; animated: the windows stand elsewhere than llvmpipe's, so compare by content — the blur is judged, not the window places; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[desktop] effect=shadow:windows=4** (gm17-desktop): the same desktop and four windows, each with its soft black drop shadow along the right and bottom edges, as on llvmpipe; no missing, hard-edged or misplaced shadow; animated: the windows move, so judge the shadow itself by content; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map** (gm18-buffer): the waving 200-column grid: blue triangles with their white wireframe; no spikes, torn or stale rows, no gaps where the mapped half of the buffer is rewritten; animated: the wave stands in another pose than llvmpipe's (this scene's probe c moved between the `imx8mp` column's runs too), so compare by content; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata** (gm19-buffer): the same grid and wireframe updated through glBufferSubData; no torn or stale vertices, no colour change; animated: compare by content; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map** (gm20-buffer): the same grid and wireframe from the interleaved buffer; colours and wire where they belong — no attribute mix-up; no torn rows; animated: compare by content; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[ideas] speed=duration** (gm21-ideas): the "ideas in motion" lettering and the lit purple tube logo, in the same pose and shading as llvmpipe; the background near-black (column mean 5 4 7) — a level of haze over it (the Pi 4 read 0x010101) is within tolerance and invisible; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[jellyfish] <default>** (gm22-jellyfish): the bell with its moving caustic light pattern, the translucent bell and tentacles blended over the blue gradient, the line tentacles; the corners are the gradient (column 508cf8 upper, 3058c8 lower); animated: the pose differs from llvmpipe's and the centre probe can fall on a tentacle in another pose (the Pi 4 read 60 levels off there), so compare by content; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[terrain] <default>** (gm23-terrain): the height-mapped landscape: grass and soil textures on the hills through their mipmap chain; no seams, holes, black or garbage tiles — a black terrain texture was a known defect (fixed on the host, never verified on a board, `docs/notes/stage2-bench-gap.md`), so look first at whether the hills are textured; the far hills blurred and the near ones sharp (the render-to-texture depth-of-field passes are there); the cream sky; on the GC7000UL this is a slow scene (4 FPS, 250 ms a frame): check it against the watchdog window before the run; the probes move between runs, so judge by content; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[shadow] <default>** (gm24-shadow): the horse on the grey floor with its cast shadow under and behind it, in the same place and shape as llvmpipe's; no acne on the floor or the horse, neither all-shadow nor no-shadow; a slow scene (2 FPS, 500 ms a frame): the watchdog window first; the shadow stands elsewhere between runs (its probe c moved in the column), so judge the shadow's shape, place and acne by content; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[refract] <default>** (gm25-refract): the glass bunny shows the lit environment refracted through its body: not black, not a copy of the frame, the background black as in the reference; the scene drew wrong on the AM62P and the Pi 4 earlier (fixed on the host, unverified on a board), so the first look is at whether the bunny is black, a mirror of the frame, or garbage; a slow scene (2 FPS): the watchdog window first; animated: compare by content; the centre probe reads through the bunny (column c 706cd000); verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[conditionals] fragment-steps=0:vertex-steps=0** (gm26-conditionals): the waving grey grid with its black cell borders and the light-to-dark shading in concentric arcs from the lower left, as on llvmpipe; no missing cell, no black or NaN patch, no wrong band; the pose is stable, so compare within the picture tolerance; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[conditionals] fragment-steps=5:vertex-steps=0** (gm27-conditionals): the grid with the five fragment conditionals: each cell filled with the same fine noisy grey pattern and the same darker diagonal patches as llvmpipe; no flat or black cell (a branch not taken), no NaN patch, no colour; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[conditionals] fragment-steps=0:vertex-steps=5** (gm28-conditionals): the grid with the five vertex conditionals: the same shaded arcs and cell borders as gm26 and as llvmpipe, the vertices where they belong; no torn or collapsed cells, no missing band; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[function] fragment-complexity=low:fragment-steps=5** (gm29-function): the grid with each cell filled by the function's concentric grey rings and the moire they make, the same ring pattern per cell as llvmpipe; no black, flat or NaN cell, no missing step; the expected difference is the aliasing of the fine rings: on those near-Nyquist rings the board probes (column tr 282c28, bl 606060, br 808480 — the values the AM62P and the Pi 4 read too, at their colour depths) differ from llvmpipe's (42, 5e, 9e), so the value judging is against the column; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[function] fragment-complexity=medium:fragment-steps=5** (gm30-function): the same ring-and-moire cells as gm29, the medium-complexity function; no black, flat or NaN cell, no wrong band; animated: the mesh's wave stands in another pose than llvmpipe's (the Pi 4 read 12% of pixels on the mesh), so compare by content; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5** (gm31-loop): the ring-and-moire cells with the vertices displaced by the vertex loop into the same wave as llvmpipe's; no black, flat or NaN cell, no torn cell from a wrong loop count; the ring aliasing is the expected difference, as for gm29; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5** (gm32-loop): the same cells and waved grid with the loop bound a constant; no black, flat or NaN cell, no missing step; animated: the pose can stand elsewhere than llvmpipe's, so compare by content; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.
- **[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5** (gm33-loop): the same cells with the loop bound a uniform: what the Pi 4 judged was that the uniform-bound picture's value equals the constant-bound one's, so the uniform bound is read — here the column already agrees (gm31 and gm33 share crc16 e370); look that this scene's picture is gm31's; no black, flat or NaN cell; the ring aliasing as for gm29; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>.

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
