<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP (Armadillo-X2, Vivante GC7000UL): the 33 glmark2-es2 pictures, looked at by eye

**4 of 33 scenes looked at (rows 1, 23, 24 and 25: build, terrain, shadow and
refract), all four right, measured on the Armadillo-X2 on 2026-11-28 from the
origin/main commit `92344e4d`; the UART route is established ("Route finding:
UART").** The other 29 rows are still the skeleton: their commit, record and
route cells are placeholders (`-`) and their verdict is `not yet looked at`;
the next board sessions fill them in. The camera route is not established.

The i.MX8MP column of `tools/cts/glmark2-pictures.txt` is a regression baseline
that nobody has looked at (`docs/notes/stage2-bench-gap.md`, "Pictures by eye";
`tools/cts/RESULTS.md`, "Common set": 33 pictures match by value, none black,
"not looked at by eye"). This note is the record of looking at each of the 33
scene pictures. It closes the i.MX8MP part of "Pictures by eye" ("Left"), issue
#1196. The Pi 4 part is `docs/notes/pi4-glmark2-pictures-by-eye.md`; the AM62P
part was done on its panel.

## Which board

The Verdin iMX8M Plus is away (off the farm since 2026-11-16, no return
date). The owner's decision of 2026-11-27: the i.MX8MP board work is done on the
Armadillo-X2 instead (same i.MX8MP SoC and GC7000UL GPU). Every row and every
record of this note was therefore measured on the X2, and each says so: the
records are under `records/imx8mp/` and carry `board=x2` in their `meta.txt`.
The X2 runs are kexec-dev runs out of Armadillo Base OS
(`docs/notes/x2-glmark2-slow-scenes.md`). The earlier stopped X2 attempts, and
why they stopped, are in `docs/notes/x2-glmark2-pictures-by-eye.md`.

## How

- Images are built only from origin/main commits; the commit of each row is in
  the table.
- `tools/cts/imx8mp-glmark2-dumps.sh` builds and runs them (its `scenes`
  subcommand lists the scenes): an i.MX8MP (X2) GM image with
  `CONFIG_GLMARK2_FRAME_DUMP=y` and one scene in `CONFIG_GLMARK2_ARGS`
  (`driver/samples/zephyr/glmark2/Kconfig`; `driver/docs/glmark2.md`, "The frame
  dump"). The 800x600 window is printed as `glmark2-fb:` lines on the UART and
  decoded with `tools/cts/glmark2-fb.py`, which checks the row crc16s, the frame
  crc32 and the scene's `glmark2-picture` line.
- The board is the Armadillo-X2, run through `tools/board.sh x2 run`
  (kexec-dev out of Armadillo Base OS; the image is jumped to and the result
  is read back after the reset). The console is the X2's UART2, through the
  farm's console resource; the screen is seen through the farm's HDMI KVM
  resource. The image feeds the WDOG for a bounded time, so a scene's run has
  a window: the slow scenes (terrain, shadow and refract, 1 to 4 FPS) are the
  first to be checked against it.
- A dump is about 1.9 MB, one scene per image; a damaged dump is rerun, pixels
  are never patched.
- On the X2 a leased task machine gets one jump a turn (its ssh key is in the
  board's tmpfs overlay and goes with the reset that ends a run; "Route
  finding: UART"), so a session there builds one image for up to four scenes
  and jumps once: `IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1
  IMX8MP_DUMP_X2_CONSOLE_RETURN=1 tools/cts/imx8mp-glmark2-dumps.sh build|run
  ... <scenes>`. The image prints each scene's dump in turn; the run writes
  one record per scene from the one capture (`set_record=` in its `meta.txt`
  names the jump's own record), and each scene then goes through
  `tools/cts/imx8mp-glmark2-session.sh <NN> --record-dir <its record>`. The
  images are built from a clean checkout of the origin/main commit
  (`IMX8MP_DUMP_TREE`), whatever the checkout the script runs from.
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

Established on the Armadillo-X2 (i.MX 8M Plus, GC7000UL; it stands in for the
Verdin iMX8MP), 2026-11-28, with one jump: the four-scene image of rows 1, 23,
24 and 25 built from origin/main `92344e4d` (image sha256 `24fee6b1...`,
`CONFIG_GC_BOARD_WDOG_SECONDS=1380`), record
`records/imx8mp/20261128-0430-gmset-01-23-24-25-24fee6b1/` (`board=x2`,
`board_measured=Armadillo-X2 ...`). Every number below was measured on the X2;
none is a Verdin number (the Verdin's console is another UART behind another
adapter and is not measured).

- **The UART carries a dump whole.** The X2's UART2 at 115200 8N1 carried four
  dumps one after the other, 2400 rows and 7.74 MB of `glmark2-fb:` lines, with
  no row damaged and none missing: every row crc16, the four frame crc32s
  (`96ef88ed`, `8e7a41e7`, `ef854f6b`, `dff39330`) and the four
  `glmark2-picture:` lines agree (`glmark2-fb.py`: "600 of 600 rows good" four
  times). Rows lost: 0 of 2400. Dumps rerun: 0 of 4.
- **How long.** `tools/cts/imx8mp-glmark2-uart-stats.py` on the record
  (seconds of the image's uptime, from the `*** Booting Zephyr` banner):

  ```
  [build] use-vbo=false: first 10.7s last 178.2s duration 167.4s bytes 1933771 11549/s damaged 0 missing 0 boot-to-end 178.2s margin 1201.8s -> ok
  [terrain] <default>: first 189.6s last 356.8s duration 167.2s bytes 1933769 11565/s damaged 0 missing 0 boot-to-end 356.8s margin 1023.2s -> ok
  [shadow] <default>: first 367.3s last 535.0s duration 167.7s bytes 1933768 11530/s damaged 0 missing 0 boot-to-end 535.0s margin 845.0s -> ok
  [refract] <default>: first 547.7s last 715.4s duration 167.7s bytes 1933769 11532/s damaged 0 missing 0 boot-to-end 715.4s margin 664.6s -> ok
  ```

  A dump is 1,933,770 bytes and takes 167 to 168 s: 11.5 kB/s, the line rate
  of 115200 baud, so the UART is the bound and nothing else is. The whole
  four-scene image ran 715 s to its last dump line and reset the SoC at 756 s
  (its 40 s hold after the result), 624 s inside its 1380 s window.
- **The slow scenes against the window.** From the scene's start to its
  dump's first line: build 10.7 s (boot and GPU init included, under 1 s),
  terrain 11.4 s, shadow 10.5 s, refract 12.7 s. The image's own frame lines
  (`glmark2-frame:`, the dump not in them): terrain 66 frames at 152 ms a
  frame, shadow 302 at 33 ms, refract 217 at 46 ms, build 301 at 33 ms. So at
  `92344e4d` none of the three is a 1 to 4 FPS scene any more, and a one-scene
  image of the slowest (terrain) needs about 1 + 12 + 168 + 60 (hold) = 241 s
  of its 600 s window (`WDOG_S`), a margin near 360 s. That figure is derived
  from this run's parts: no one-scene image was run. The same `uart-stats`
  with `--watchdog-s 600` shows where a 600 s window would end in a set: the
  third dump ends at 535 s (margin 65 s) and the fourth is 115 s over, which
  is why a set has its own window (60 + n * 290 + 60 + 100 s, at most 1380).
- **A dump image's FPS and verdict are not a scene's.** glmark2's own
  `FPS:`/`FrameTime:` line of a dumped scene has the dump's 167 s in it
  (terrain: `FPS: 1 FrameTime: 2696.389 ms`), and the score is the integer
  mean of those: this image printed `glmark2 Score: 0`, hence `glmark2: FAIL
  rc=-5` (the sample's "score is not 0" rule) beside `4 scenes ran, 0 failed,
  0 unsupported`, `GPU resets 0` and `GL errors raised 0`. A one-scene terrain
  dump image scores 0 the same way (66 frames in about 180 s). The picture is
  judged from the dump; the FAIL line is the dump's cost, not a wrong scene.
  It is an open tooling point: where ssh does return, `board.sh` reads that
  result and `imx8mp-glmark2-dumps.sh run` calls the run failed before it
  decodes (not met here, because ssh did not return; see below).
- **How the console reaches the runner.** The X2's console is handed out as
  the lease's serial console MCP server, which holds the serial bridge's one
  client: `tools/board.sh`'s own capture gets "busy". The route that worked:
  `tools/cts/imx8mp-glmark2-console-relay.py <the MCP server's URL of the
  turn> <file> --from-now` (it calls `serial_read` once a second and appends
  the bytes; read only), `tools/am62p_netboot_uart.py feed <file> <port>`,
  and `BOARD_UART=<the loopback address>:<port>` for `board.sh` (the "Feed bridge" of
  `am62p_netboot_uart.py`). The relay moved 7,776,056 bytes and reported
  `lost 0`. An agent cannot relay a dump through its own reads (1.9 MB a
  scene). Before the jump one newline was sent to the console, with Linux up
  for over 50 minutes (it cannot stop U-Boot then): it answered with the login
  prompt. Nothing else was sent.
- **One jump a turn, and what the runner says after it.** The jump was made by
  `tools/board.sh x2 run` (kexec-dev; the loader printed `crash: i.MX 8M Plus
  (fsl,imx8mp): gpio-mxc and irqsteer rows are let through`). The image reset
  the SoC itself, U-Boot printed `reset cause: watchdog`, and Linux reached
  its login prompt 18 s after the reset. The board then refused the task
  machine's ssh key (`Permission denied (publickey,...)`): connected installs
  it in the tmpfs overlay at the start of a turn, and the reset removes it.
  `board.sh` waited its 1500 s and wrote `rc_cycle=5` (not_back, no
  `back=1`); LPGPR1..3 were not read (the console's last `yk-record:` line is
  `594b120e fc3f0000 6210003f 00258320`). With
  `IMX8MP_DUMP_X2_CONSOLE_RETURN=1` the dumps script takes such a record
  when the captured console shows `board: resetting the SoC` and Linux's
  `login:` after it, and writes `return=console ...` into `meta.txt`. Nothing
  was retried and no second jump was made. So a session on a leased task
  machine is one set of up to four scenes a turn; 33 scenes are nine turns,
  or one session from a machine whose key the board keeps.
- **Rerun rule, unchanged.** A dump with a damaged or missing row, or without
  its picture line, is rerun (`imx8mp-glmark2-session.sh` exit 3), never
  patched; in a set, the set is rerun. None was needed here.
- **Redaction.** The capture holds U-Boot's and Linux's boot text after the
  reset, so `imx8mp-gles3-redact.py --check` found device-tree `name@address`
  words, the console's tty and the login prompt's host name; `--fix` rewrote
  them and the check is clean (34 files). The dump rows are not touched by
  it: the four dumps were decoded again afterwards and their crc32s agree.

### Route finding: camera

Not yet established (the camera-probe item). To fill in: whether the camera
frames the Verdin's panel, the corners, the rotation to pass, the fill, whether
`calibrate_camera` holds, and what a glmark2 scene looks like through it.

## Verdicts

Verdict values: `right`, `wrong` (becomes an entry under "Wrong scenes"),
`seen without a value` (photograph only), `not yet looked at`.

| scene | commit | record | route | verdict |
|---|---|---|---|---|
| [build] use-vbo=false | 92344e4d | records/imx8mp/20261128-0430-gm01-build-24fee6b1/ | dump | right |
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
| [terrain] <default> | 92344e4d | records/imx8mp/20261128-0430-gm23-terrain-24fee6b1/ | dump | right |
| [shadow] <default> | 92344e4d | records/imx8mp/20261128-0430-gm24-shadow-24fee6b1/ | dump | right |
| [refract] <default> | 92344e4d | records/imx8mp/20261128-0430-gm25-refract-24fee6b1/ | dump | right |
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

- **[build] use-vbo=false** (gm01-build): the rearing horse, lit grey on black, drawn from client-side arrays; the same pose and shading as llvmpipe; no missing or stray triangles, no holes, no misbuilt body; the reference moves at most a step, so compare within the picture tolerance, and the value judging is against the `imx8mp` column (probe c on the horse's body, corners black, mean 12 12 12); the UART route finding (dump size, time, rows lost and rerun) is made on this scene; verdict right, route dump, commit 92344e4d, record records/imx8mp/20261128-0430-gm01-build-24fee6b1/, frame crc32 96ef88ed, picdiff mean abs diff 0.89 0.89 0.89; 0.5% of pixels differ by more than 24 in any channel; bbox 217,49-560,557; black board 89.1% ref 89.1%; board probes c a1a1a1, tl 000000, tr 000000, bl 000000, br 000000; ref probes c a4a4a4, tl 000000, tr 000000, bl 000000, br 000000; 0 damaged row(s) left out. Seen (X2, 2026-11-28): the horse as on llvmpipe, a fraction of a rotation step apart: the difference is the silhouette and faint facet edges (0.5% of pixels), black is 89.1% of both pictures; the picture line is `probes c a1a1a1ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 2128, crc16q 466e; mean 12 12 12` — the column's probes and mean at 8 bits a channel (the column was read as RGB565 with alpha 00, so its crc16s are not comparable byte for byte). No missing or stray triangle, no hole.
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
- **[terrain] <default>** (gm23-terrain): the height-mapped landscape: grass and soil textures on the hills through their mipmap chain; no seams, holes, black or garbage tiles — a black terrain texture was a known defect (fixed on the host, never verified on a board, `docs/notes/stage2-bench-gap.md`), so look first at whether the hills are textured; the far hills blurred and the near ones sharp (the render-to-texture depth-of-field passes are there); the cream sky; on the GC7000UL this is a slow scene (4 FPS, 250 ms a frame): check it against the watchdog window before the run; the probes move between runs, so judge by content; verdict right, route dump, commit 92344e4d, record records/imx8mp/20261128-0430-gm23-terrain-24fee6b1/, frame crc32 8e7a41e7, picdiff mean abs diff 18.76 20.93 5.88; 35.7% of pixels differ by more than 24 in any channel; bbox 0,35-799,599; black board 0.0% ref 0.0%; board probes c 5f6b15, tl 545512, tr 758d22, bl 74851c, br 83ac1c; ref probes c 7c951f, tl 5d6011, tr 728a22, bl 60701a, br 6e8b22; 0 damaged row(s) left out. Seen (X2, 2026-11-28): the hills are textured, grass with the soil showing orange in the far folds, no black or garbage tile, no seam or hole; the far hills are blurred and the near ones sharp; the sky is cream; the camera stands elsewhere than llvmpipe's (35.7% of pixels, as animated scenes do), judged by content; mean 119 129 44 against the column's 119 129 45. The scene ran at 152 ms a frame (66 frames), not 250.
- **[shadow] <default>** (gm24-shadow): the horse on the grey floor with its cast shadow under and behind it, in the same place and shape as llvmpipe's; no acne on the floor or the horse, neither all-shadow nor no-shadow; a slow scene (2 FPS, 500 ms a frame): the watchdog window first; the shadow stands elsewhere between runs (its probe c moved in the column), so judge the shadow's shape, place and acne by content; verdict right, route dump, commit 92344e4d, record records/imx8mp/20261128-0430-gm24-shadow-24fee6b1/, frame crc32 ef854f6b, picdiff mean abs diff 0.24 0.24 0.24; 0.0% of pixels differ by more than 24 in any channel; bbox 0,81-799,593; black board 58.7% ref 58.7%; board probes c a0a0a0, tl 000000, tr 000000, bl 4c4c4c, br 4c4c4c; ref probes c a0a0a0, tl 000000, tr 000000, bl 4d4d4d, br 4d4d4d; 0 damaged row(s) left out. Seen (X2, 2026-11-28): the horse, the grey floor and the cast shadow in the same place and shape as llvmpipe's, pixel for pixel within a level (mean difference 0.24, no pixel more than 24 off); no acne, the floor outside the shadow evenly grey; probe c a0a0a0 is llvmpipe's (the column's 686c68 was another pose), mean 34 34 34 as the column. The scene ran at 33 ms a frame (302 frames), not 500.
- **[refract] <default>** (gm25-refract): the glass bunny shows the lit environment refracted through its body: not black, not a copy of the frame, the background black as in the reference; the scene drew wrong on the AM62P and the Pi 4 earlier (fixed on the host, unverified on a board), so the first look is at whether the bunny is black, a mirror of the frame, or garbage; a slow scene (2 FPS): the watchdog window first; animated: compare by content; the centre probe reads through the bunny (column c 706cd000); verdict right, route dump, commit 92344e4d, record records/imx8mp/20261128-0430-gm25-refract-24fee6b1/, frame crc32 dff39330, picdiff mean abs diff 4.97 4.75 5.61; 9.9% of pixels differ by more than 24 in any channel; bbox 184,126-598,521; black board 79.3% ref 79.3%; board probes c 7b4c8a, tl 000000, tr 000000, bl 000000, br 000000; ref probes c 533866, tl 000000, tr 000000, bl 000000, br 000000; 0 damaged row(s) left out. Seen (X2, 2026-11-28): the glass bunny with the environment refracted through it, blue and violet with the orange lights in the ear, head and haunch as on llvmpipe; not black, not a copy of the frame, no garbage; the background black (79.3% of both pictures); the bunny stands a rotation step from llvmpipe's (9.9% of pixels, all inside its outline), judged by content; mean 24 20 33 against the column's 22 18 29. The scene ran at 46 ms a frame (217 frames), not 500.
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

So far: 4 of 33 pictures judged (rows 1, 23, 24 and 25), 4 right, 0 wrong,
all from the frame dump (route `dump`), all measured on the Armadillo-X2 from
origin/main `92344e4d` in one jump on 2026-11-28; the llvmpipe references are
`records/host/20261128-0421-gmref-8c9abf51/` (Mesa 26.2.1 llvmpipe). The
records are under `records/imx8mp/` on the session machine (gitignored,
redacted). `92344e4d` is not the origin/main tip of that day: the tip
(`8c9abf51`) did not build (`capi/Cargo.toml` has the key `vivante-st` twice
since `110080eb`), so the newest commit before that one was used.

To fill in when the table is complete: the count judged (of 33), right and
wrong, the origin/main commits the images were built from, the method as run,
and where the records are kept.
