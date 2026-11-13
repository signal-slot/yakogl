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
  record column gives the directory name; every record cell is the full
  `records/pi4/<YYYYMMDD-HHMM>-<name>-<sha8>/` path (records are kept on the session machine).

## Verdicts

| scene | verdict | what was looked at | origin/main commit | record |
|---|---|---|---|---|
| [build] use-vbo=false | right | The rearing horse, lit grey on black, in the same pose and shading as llvmpipe; no missing or stray triangles from the client-side arrays, no holes. Animated by a step at most, compared within the picture tolerance (0.0% of pixels more than 24 levels off, mean difference 0.1 level). | 86b4af69 | records/pi4/20261113-0847-gm01-build-409fcbdf |
| [build] use-vbo=true | right | The same horse drawn from VBOs, identical to llvmpipe's and to the use-vbo=false picture (0.0% of pixels off, mean difference 0.03 level); no index or attribute mix-up. The first dump lost rows 68-107 on the UART and was rerun once (records/pi4/20261113-0851-gm02-build-ff0460dd). | 86b4af69 | records/pi4/20261113-0856-gm02-build-ff0460dd |
| [texture] texture-filter=nearest | right | The wood crate (the plank front and the side panel) sampled by nearest texel: the same crate as llvmpipe, the only difference a sparse grid of single pixels at texel boundaries (0.1% of pixels, the texel-selection ties of nearest filtering); no blur, no swapped or missing face. | 86b4af69 | records/pi4/20261113-0901-gm03-texture-b8500398 |
| [texture] texture-filter=linear | right | The same crate bilinearly filtered: smooth grain, no texel blocks, every probe within one level of llvmpipe's and no pixel more than 24 levels off. | 86b4af69 | records/pi4/20261113-0905-gm04-texture-633f6a6e |
| [texture] texture-filter=mipmap | right | The crate through its mipmap chain: the grain a touch softer than the linear picture, as a level between 0 and 1 should be, and even over the whole crate; no shimmer or aliasing in the far planks, no band, seam or tile of a wrong level, no garbage tile, no black face. The difference against llvmpipe is a uniform sub-level haze (mean 0.3 level, 0.0% of pixels more than 24 off), so the level selection and the trilinear blend agree with llvmpipe's. | 86b4af69 | records/pi4/20261113-0910-gm05-texture-cc51aac0 |
| [shading] shading=gouraud | right | The blue cat lit per vertex: the same soft, faintly faceted highlight along the back and the dark underside as llvmpipe, the eyes and ears where they belong. The difference is the silhouette and faint triangle edges (0.1% of pixels more than 24 levels off, mean difference under half a level). | 86b4af69 | records/pi4/20261113-0915-gm06-shading-49fcad29 |
| [shading] shading=blinn-phong-inf | right | The glossy cat with the white specular spots on the ear, cheek, back and flank in the same places and sizes as llvmpipe; the difference is faint speckle at the highlight fringes where the specular power rounds (0.2% of pixels more than 24 off), the probes within four levels. | 86b4af69 | records/pi4/20261113-0919-gm07-shading-f2558063 |
| [shading] shading=phong | right | The glossy cat lit per pixel: the bright specular spots on the flank, back, tail, ear and face in the same places and sizes as llvmpipe, the smooth falloff around them; the difference is speckle at the highlight fringes and the silhouette (0.4% of pixels more than 24 off), the probes within four levels. The first dump lost rows 96-156 on the UART and was rerun once (records/pi4/20261113-0924-gm08-shading-1d2e143b). | 86b4af69 | records/pi4/20261113-0933-gm08-shading-1d2e143b |
| [shading] shading=cel | right | The green cat in three flat tones: the pale highlight bands on the ear, cheek, flank and tail, the dark shadow band under the body and behind the tail, and the mid-green body, with the same band edges as llvmpipe (0.0% of pixels more than 24 levels off, mean difference 0.02 level); no missing step, no gradient where a step should be. | 86b4af69 | records/pi4/20261113-0929-gm09-shading-ca9dabaf |
| [bump] bump-render=high-poly | pending | | | |
| [bump] bump-render=normals | pending | | | |
| [bump] bump-render=height | pending | | | |
| [effect2d] kernel=0,1,0;1,-4,1;0,1,0; | pending | | | |
| [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1; | pending | | | |
| [pulsar] light=false:quads=5:texture=false | pending | | | |
| [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4 | pending | | | |
| [desktop] effect=shadow:windows=4 | pending | | | |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map | right | The waving 200-column grid: blue triangles with their white wireframe, the same ridges as llvmpipe; no spikes, torn or stale rows, or gaps where the mapped half of the buffer is rewritten. Animated, compared by content (0.0% of pixels more than 24 levels off). | 7fa0abf8 | records/pi4/20261109-0247-gm18-buffer-46775c01 |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata | right | The same grid and wireframe as llvmpipe, updated through glBufferSubData; no torn or stale vertices. The ridges stand one animation step from llvmpipe's (9.3% of pixels off, all on the mesh). | 7fa0abf8 | records/pi4/20261109-0251-gm19-buffer-e3b74fdd |
| [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map | right | The same grid and wireframe from the interleaved buffer; no attribute mix-up (colours and wire where they belong), no torn rows. One animation step from llvmpipe (9.5% of pixels off, all on the mesh). | 7fa0abf8 | records/pi4/20261109-0256-gm20-buffer-26f83bec |
| [ideas] speed=duration | right | "ideas in motion" lettering and the lit purple tube logo, in the same pose and shading as llvmpipe. The background reads 0x010101 where llvmpipe's is 0x000000 (one level, invisible, within the picture tolerance). | 7fa0abf8 | records/pi4/20261109-0301-gm21-ideas-db464883 |
| [jellyfish] <default> | right | The bell with its moving caustic light pattern, the translucent bell and tentacles blended over the blue gradient, the line tentacles. Animated, compared by content; the centre probe falls on a tentacle in another pose (so the pi4 column's centre probe is 60 levels off). The first dump lost rows 36 and 348-350 on the UART and was rerun once (records/pi4/20261109-0305-gm22-jellyfish-b2dc908e). | 7fa0abf8 | records/pi4/20261109-0310-gm22-jellyfish-b2dc908e |
| [terrain] <default> | right | The height-mapped landscape: grass and soil textures on the hills through their mipmap chain, no seams, holes, black or garbage tiles; the far hills blurred and the near ones sharp, so the render-to-texture passes (depth of field) are there; the cream sky. Animated, another pose than llvmpipe, so compared by content (its picture value is off the pi4 column by up to 23 levels for the same reason). | 7fa0abf8 | records/pi4/20261109-0237-gm23-terrain-53ad9f83 |
| [shadow] <default> | right | The horse on the grey floor with its cast shadow under and behind it, in the same place and shape as llvmpipe's; no acne on the floor or the horse, neither all-shadow nor no-shadow (0.1% of pixels more than 24 levels off). Decoded from the console before the board layer's "board: record" replay, which reprints the dump's last rows cut short. | 7fa0abf8 | records/pi4/20261109-0225-gm24-shadow-99f45384 |
| [refract] <default> | right | The glass bunny shows the lit environment refracted through its body, as on llvmpipe: not black, not a copy of the frame, the background black as in the reference. Animated by a step, compared by content (5.0% of pixels off, all inside the bunny). | 7fa0abf8 | records/pi4/20261109-0242-gm25-refract-d5d024b0 |
| [conditionals] fragment-steps=0:vertex-steps=0 | pending | | | |
| [conditionals] fragment-steps=5:vertex-steps=0 | pending | | | |
| [conditionals] fragment-steps=0:vertex-steps=5 | pending | | | |
| [function] fragment-complexity=low:fragment-steps=5 | pending | | | |
| [function] fragment-complexity=medium:fragment-steps=5 | pending | | | |
| [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5 | pending | | | |
| [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5 | pending | | | |
| [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5 | pending | | | |
