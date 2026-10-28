<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Board results

The newest result of each suite on each board, with the main commit its image
was built from. A new run replaces its row in the same push as the numbers
(the old row stays in git history). The summary in Japanese is
[STATUS.md](STATUS.md); what is open is in
[the GitHub issues](https://github.com/signal-slot/yakogl/issues).

Run records (`records/<board>/...`, consoles and per-case files) are
gitignored. Every image is booted by the board's own bootloader (U-Boot from
eMMC on the AM62P and the i.MX8MP, the Pi firmware's tryboot on the Pi 4) and
run with `tools/board.sh`.

Earlier measurements are in git history. Until 2026-10-16 this file was the
whole measurement log; code comments that cite a RESULTS.md section by name
or date ("1r", "2026-10-04", "What the 43 MiB are", ...) cite that log, which
is read with `git show ab43a20c5:tools/cts/RESULTS.md`. Paths written as
`ab43a20c5:<path>` name a record removed in the same clean-up, read the same
way.

## ES 2.0: dEQP-GLES2 (stage 1, met)

One main commit, `e9901869a`, on the three boards (2026-10-15/16; issue #870,
closed). Mustpass list: 17,165 cases.

| Board | Pass | Fail | NotSupported | QualityWarning / CompatibilityWarning | GPU resets |
|---|---:|---:|---:|---|---:|
| AM62P, 17 images at 1x (re-run at `177c62fd`, 2026-10-28, `records/am62p/deqp-177c62fd/`, 17,371 cases begun: the list plus 206 outside it) | 17,330 | **0** | 34 | 5 / 2 | 0 |
| AM62P, ms4 image (`multisample.*` on the 4x config) | 16 | **0** | 0 | 0 / 0 | 0 |
| AM62P combined (ms4 replaces the 16 1x `multisample.*`) | 17,140 | **0** | 18 | 5 / 2 | 0 |
| i.MX8MP, 29 entries (re-run at `d0c5dc48`, 2026-10-23, `records/imx8mp/deqp-d0c5dc48/`, 17,371 cases begun: the list plus 206 outside it) | 17,318 | **0** | 34 | 19 (other) | 0 |
| Pi 4, 29 rows (every list case has a result; re-run at `46f93767`, 2026-10-24, `records/pi4/deqp-46f93767/`) | 17,126 | **0** | 34 | 3 / 2 | 0 (0 recoveries in 29 records) |

- The 34 NotSupported are the same on the three boards and accepted
  (`docs/es2-cts-failures.md` section C, `tools/cts/es2-accepted.txt`):
  `clip_control` 8, `negative_api.texture.compressedtexsubimage2d_*` 9,
  `multisampled_render_to_texture.readpixels` 1 and `multisample.*` 16 (no
  multisample surface at 1x; the AM62P runs them on its ms4 image).
- The AM62P 1x row is from `177c62fd` (2026-10-28, same figures as `996bc35f`,
  `25cce714` and `f41a19e8`, `records/am62p/deqp-177c62fd/`, 17 images,
  every case has a result, every image's own counters read `complete=1`,
  no row needed a re-arm). Against
  `a917d123` (17,331 pass) one case warns instead of passing:
  `flush_finish.flush_wait`. On the 17,165-case list it reads 17,124 pass,
  0 fail, 34 not supported, 5 QualityWarning and 2 CompatibilityWarning —
  the list figure of `e9901869a`. The ms4 row is still from `e9901869a`; the
  combined row below is this run's 17,124 plus the 16 ms4 passes, so its
  figures are unchanged.
- The CompatibilityWarnings are `flush_finish.flush` and
  `flush_finish.flush_wait`, dEQP's timing check. The AM62P 1x run at
  `f41a19e8` warned on both, as at `e9901869a` and `d61a4c5d`; the AM62P
  run at `a917d123` warned only on `flush_finish.flush` (`flush_wait`
  passed); the Pi 4 run at `46f93767` warned on both (at `89b4a1c7` only on
  `flush`).
- The i.MX8MP's 19 warnings appear on NXP's own driver too.
- The EGL double buffer (`1eb9783d`) costs the dEQP images nothing: the glue
  creates no window surface. `driver/samples/zephyr/deqp/src/main.cpp` (the
  AM62P, the i.MX8MP and, through `v3d_deqp`, the Pi 4) runs with
  `--deqp-surface-type=pbuffer`, and the platform port registers no window
  factory (`tools/cts/cts-yakogl-target.patch`). A pbuffer is never
  `EGL_BACK_BUFFER`-buffered: `EglSurface::back` is set only in
  `create_window_surface`, so its swap neither presents nor allocates the two
  extra buffers, and `eglSwapBuffers` takes the `finish_previous` path it
  took before. The glue needs no `EGL_SINGLE_BUFFER`. All three trees above
  (`f41a19e8`, `d0c5dc48`, `46f93767`) contain `1eb9783d` and have 0 failures.
  A per-image wall-time comparison is not made: the
  run records are not in the tree.

## Common set (stage 2 input)

Each row names the main commit it ran from (`tools/cts/common-tree.txt`: glmark2 GM,
gl_test GT). Since 2026-10-19 the common set holds tests and benchmarks only:
the Slint demo (SO, SV) is kept working but is no row, and the AGL cluster
sample (CL) is removed. The AM62P and Pi 4 runs below were made with the
five-row set and only their GM and GT rows are results here; the i.MX8MP row
is from the two-row set.

| Board | glmark2-es2 | gl_test | GPU resets |
|---|---|---|---:|
| AM62P | 467, 33 scenes, 0 failed, 0 GL errors; main `c5dcf5e8` plus the PowerVR depth-texture fix of #40 (image built from `c999f2f7`, 2026-10-27, `records/am62p/common-c999f2f7/20261027-2252-GM-0de4a19e/`); **shadow 10.64 ms a frame (95 FPS), refract 37.03 ms (28 FPS), terrain 90.91 ms (11 FPS)** (353.23, 375.53, 93.95 at `445c5653`); per-scene ms/frame below; pictures against `tools/cts/glmark2-pictures.txt` column `am62p` (the `445c5653` run's): 33 scenes, match 29, mismatch 4, all four probe c of animated scenes (buffer map x2, terrain, refract), shadow a match, none black | 50 of 50 (at `445c5653`, not rerun; case 215 `shadowmap` PASS at `c999f2f7` and at main plus the case, below) | 0 |
| i.MX8MP | 42, 33 scenes, 0 failed, 0 GL errors (#814 fixed by #922); main `445c5653`, 2026-10-20, 1 screen buffer (the default); re-run from main `a917d123` (holds the GC mipmap offer `dc033dc44`, #1106) the same day: score 42 (`records/imx8mp/common-a917d123`, GM `verdict=done`, words `fc3f002a`), per-scene record `records/imx8mp/20261020-2100-glmark2-a917d123/` (66 boots, ms a frame = 1000 / the integer FPS): **terrain 250 ms (4 FPS), shadow 500 ms (2 FPS), refract 500 ms (2 FPS)**, the other 30 scenes 16.95 to 35.71 ms; 1934.56 ms summed (1932.65 at `445c5653`), so the offer moved no scene at this resolution; all 33 pictures match the `imx8mp` column, none black, terrain and refract means equal to the `445c5653` ones (not looked at by eye); per-scene FPS and picture values from the SNVS record (below: terrain 4, shadow 2, refract 2 FPS), pictures in `tools/cts/glmark2-pictures.txt` column `imx8mp` (33 from this run, none black, not looked at by eye); 49 at 3 buffers, 46 at 2 (`bebce725`, below) | 49 of 49 run, case 16 skipped (#8); cases 10 and 17 nondeterministic (#9, #7: 11 boots of `work/egl-double-buffer`'s code, case 17 failed 3, case 10 failed 1; main `0247ac665`, 10 boots: case 17 failed 3, case 10 failed 3, case 44 once) | 0 where counted |
| Pi 4 | 195, 33 scenes, 0 failed, 0 GL errors; main `c4af4917`, 2026-10-22 (the TFU no-wait, FS dump off, uniforms from the host arrays, lazy raster, depth storage defer, copy reason counters, UIF walker and buffer retired reuse; the main tip `71c73b0f` differs from it only in `tools/check.sh` and its selftest); GM `verdict=done`, record `records/pi4/20261022-0308-glmark2-c4af4917/`: **terrain 161.79 ms (7 FPS), shadow 7.34 ms (137 FPS), refract 17.27 ms (58 FPS)** (179.26, 7.32, 18.72 at `445c5653`); per-scene ms/frame below; pictures 31 of 33 match the `pi4` column (desktop blur and jellyfish, both animated, differ in one probe each), none black | 12 of 12 (at `445c5653`; not rerun) | 0 |

- i.MX8MP glmark2 at 1, 3 and 2 screen buffers (frame-floor step 2, GM half;
  `docs/notes/imx8mp-frame-floor.md`): the GM row of
  `tools/cts/common-images.sh` built from main `bebce725`, and the same row
  built by step 2's `west build` with
  `-DCONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS=3` and `=2` (each `.config`
  checked; every image says `YAKOGL_SAMPLE_TREE=bebce725`). Each image ran once
  by `tools/board.sh imx8mp run` (`uboot-mmc`), then its 11 per-scene FPS
  pages by `tools/board.sh imx8mp gm <image> <dir> fps`, one client on the
  board at a time, 2026-10-18, `records/imx8mp/gm-arms-bebce725/` (score runs
  at its top, pages in `fps-GM/`, `fps-GM-3buf/`, `fps-GM-2buf/`, each with
  its `glmark2-pages.txt`). Every run and every page `verdict=done`,
  `resets=absent`; every score run 33 ran, 0 failed, 0 unsupported, GL errors
  raised 0, 0 draws that sampled an incomplete texture. No hang.

  | Arm | Score run dir | glmark2 score | Record words |
  |---|---|---:|---|
  | 1 buffer (default) | `20261018-0634-GM-d40ebfc9` | 42 | `594b120e fc3f002a 6184003f 00258320` |
  | 3 buffers | `20261018-0641-GM-3buf-2537aa2d` | 49 | `594b120e fc3f0031 6184003f 00258320` |
  | 2 buffers | `20261018-0648-GM-2buf-19806152` | 46 | `594b120e fc3f002e 6184003f 00258320` |

  Per-scene FPS (upstream's integer FPS, from the GM_PAGE records; the panel
  runs at 58.76 Hz, which upstream rounds up to 59):

  | Scene | 1 buffer | 3 buffers | 2 buffers |
  |---|---:|---:|---:|
  | `[build] use-vbo=false` | 30 | 41 | 40 |
  | `[build] use-vbo=true` | 59 | 65 | 59 |
  | `[texture] texture-filter=nearest` | 59 | 65 | 59 |
  | `[texture] texture-filter=linear` | 59 | 65 | 59 |
  | `[texture] texture-filter=mipmap` | 59 | 64 | 59 |
  | `[shading] shading=gouraud` | 59 | 63 | 59 |
  | `[shading] shading=blinn-phong-inf` | 59 | 62 | 59 |
  | `[shading] shading=phong` | 59 | 62 | 59 |
  | `[shading] shading=cel` | 59 | 62 | 59 |
  | `[bump] bump-render=high-poly` | 59 | 63 | 59 |
  | `[bump] bump-render=normals` | 59 | 65 | 59 |
  | `[bump] bump-render=height` | 59 | 64 | 59 |
  | `[effect2d] kernel=0,1,0;1,-4,1;0,1,0;` | 40 | 48 | 44 |
  | `[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | 28 | 28 | 28 |
  | `[pulsar] light=false:quads=5:texture=false` | 55 | 62 | 58 |
  | `[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4` | 30 | 33 | 33 |
  | `[desktop] effect=shadow:windows=4` | 30 | 58 | 49 |
  | `[buffer] ...:interleave=false:...:update-method=map` | 30 | 33 | 33 |
  | `[buffer] ...:interleave=false:...:update-method=subdata` | 30 | 33 | 33 |
  | `[buffer] ...:interleave=true:...:update-method=map` | 30 | 35 | 35 |
  | `[ideas] speed=duration` | 30 | 46 | 41 |
  | `[jellyfish] <default>` | 30 | 56 | 45 |
  | `[terrain] <default>` | 1 | 1 | 1 |
  | `[shadow] <default>` | 2 | 2 | 2 |
  | `[refract] <default>` | 1 | 1 | 1 |
  | `[conditionals] fragment-steps=0:vertex-steps=0` | 55 | 61 | 59 |
  | `[conditionals] fragment-steps=5:vertex-steps=0` | 50 | 60 | 57 |
  | `[conditionals] fragment-steps=0:vertex-steps=5` | 53 | 61 | 58 |
  | `[function] fragment-complexity=low:fragment-steps=5` | 51 | 60 | 57 |
  | `[function] fragment-complexity=medium:fragment-steps=5` | 49 | 60 | 56 |
  | `[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5` | 52 | 60 | 58 |
  | `[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5` | 49 | 60 | 56 |
  | `[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5` | 49 | 60 | 56 |

  The default reads like the baseline (42). At 3 buffers every light scene
  is above 58 FPS (`build`, `texture`, `shading`, `bump` 62 to 65, the
  shader scenes 60 to 61), but by little: without the latch wait a light
  frame still costs 15 to 16 ms. The scenes the default rounds up to two
  periods (30) gain most (`jellyfish` 56, `desktop` shadow 58). At 2 buffers
  no scene is above the panel's 59, and none is rounded up to two periods.
  `terrain`, `shadow`, `refract` and the 5x5 `effect2d` do not move. The
  panel picture of the 3- and 2-buffer arms was not checked (no camera).

- i.MX8MP common set at 1, 3 and 2 screen buffers (frame-floor step 2):
  the rows of `tools/cts/board-stage.sh imx8mp common`, run on 2026-10-18
  from main `45dc44a8` (default 1 buffer; `records/imx8mp/common-45dc44a8/`),
  and from two local commits that differ from it only by the Kconfig default
  (`default 3`: `common-47c9c974/`, `default 2`: `common-fa755643/`; each
  `.config` checked). Every row `verdict=done`, `resets=absent`, `uboot-mmc`.

  | Row | 1 buffer (default) | 3 buffers | 2 buffers |
  |---|---|---|---|
  | GM | score 42 (`594b120e,fc3f002a,6184003f,00258320`) | 49 (`...fc3f0031...`) | 46 (`...fc3f002e...`) |
  | GT | 49 of 49 run, case 16 skipped, none failing | same | same |

  GM at every arm: 33 ran, 0 failed, **GL errors raised 0**, 0 incomplete-texture
  draws. #814: GM raised 0 GL errors in all three arms here, as in
  `imx8mp-db-baseline` (`499b946e`) and the earlier arms (`bebce725`).
- i.MX8MP common set from main `445c5653` with the per-scene record
  (2026-10-20, `tools/cts/board-stage.sh imx8mp common`, `uboot-mmc`,
  `records/imx8mp/common-445c5653/`). Every row `verdict=done`,
  `resets=absent`: GM `594b120e,fc3f002a,6184003f,00258320` (score 42, 33
  ran, 0 failed, GL errors raised 0, 0 incomplete-texture draws; the same
  words as `a8fdc9f0` and `c5f10ea9`), GT `594b010e,ffff0107,ffff0031,00100031`
  (49 of 49 run, case 16 skipped). The stage's first GM run (`-1700`) was cut
  off by a worker restart (`no_record`); the row is GM `-1709`. The per-scene
  record (`tools/board.sh imx8mp gm <GM image> <dir> pictures`, 66 boots,
  every boot `verdict=done`, resumed once after a worker restart,
  `records/imx8mp/gm-445c5653/pictures/glmark2-pages.txt`): 33 scenes with an
  FPS and a picture value, **1932.65 ms/frame summed (3183 at `c5f10ea9`)**.
  The move is in **terrain, 1000 -> 250 ms (1 -> 4 FPS), and refract, 1000
  -> 500 ms (1 -> 2 FPS)**. Shadow stays at 500 ms (2 FPS), and every other
  scene equals the `c5f10ea9` record within one FPS step (the light scenes at
  the panel's 59). The one GL or GC7000 change between the two commits is
  `dc033dc44` (the GL core offers the GC7000's rendered-into RGBA8 chains to
  the back end, #1106). Terrain and refract are the scenes that call
  `glGenerateMipmap` on a render target, so this is the first board reading
  of that change, although the record cannot show that the GPU chain was
  taken (the i.MX8MP has no console). ms is 1000 /
  integer FPS; no `--gm-diff` ran, because the `c5f10ea9` record is not on
  the node that ran this one.
  Pictures (`--gm-pictures ... imx8mp` against the `a8fdc9f0` column): `33
  scenes, match 28, mismatch 5`, none black or absent. Four are probe c of
  animated scenes (buffer map off 72, terrain 56, shadow 24, refract 32). The
  fifth is **`[texture] texture-filter=mipmap`, probe tl `000000` where
  `a8fdc9f0` had `181c18`**, the change that arrived with `0d21ab47`. It was
  bisected on the board at `c5f10ea9` (`a8fdc9f0` and `4ba42a28` give the
  old value; `0d21ab47`, `1fea2e25` and `9e7b809a` the new one). The only
  library change in `0d21ab47` is `ds_cache::resolve_depth`, which this
  scene never runs, so the mechanism is open. dEQP-GLES2 `texture.mipmap.*`
  read 98 pass, 0 fail, 14 quality warnings at `c5f10ea9`. Which picture is
  right is not known (no camera). The `imx8mp` column is now this run's, with
  that scene marked unexplained in the column's comment.
- The i.MX8MP gl_test cell is from `work/egl-double-buffer` (the
  EGL-double-buffered window); its glmark2 read 42 on 2026-10-17 from the code rebased
  onto `f40a2aba0` (#922's carve-out, 0 GL errors), 43 the day before from
  the same code on `0247ac665`, the whole jellyfish on the panel (no band)
  both times; gl_test (before the rebase) 49 of 49 on 7 of 11 boots, the other 4 lost case 17 (3)
  or case 10 (1); main at `0247ac665` the same day: 49 of 49 on 6 of 10
  boots, case 17 failed 3, case 10 failed 3, case 44 once. Case 17 (ctx2, two
  contexts on pbuffers) is nondeterministic on main too (#7 reopened) and not
  this tree's; #9 covers case 10.
- The README glmark2 table's runs: the GM row of the common set on the
  i.MX8MP from main `445c5653` (2026-10-20); the AM62P column is the run of
  main `c5dcf5e8` plus #40's fix (2026-10-27, the row above, image
  `c999f2f7`); the Pi 4 column is the run of main `c4af4917` (2026-10-22,
  below). Per scene, ms/frame. The AM62P and Pi 4 columns are
  `glmark2-ms.py` / `common-tally.py --gm-table` (from `FrameTime`). The
  i.MX8MP column is 1000 / the integer FPS of its SNVS record, with the FPS
  in brackets. The earlier columns are the AM62P's 461 run of `445c5653`
  (this table before) and the Pi 4's 193 run of `445c5653`.

  | Scene | AM62P | AM62P `445c5653` | Pi 4 `c4af4917` | Pi 4 `445c5653` | i.MX8MP (FPS) |
  |---|---:|---:|---:|---:|---:|
  | build use-vbo=false | 5.79 | 10.00 | 4.75 | 6.42 | 33.33 (30) |
  | build use-vbo=true | 1.14 | 1.14 | 3.64 | 3.61 | 16.95 (59) |
  | texture nearest | 1.07 | 1.07 | 3.78 | 3.74 | 16.95 (59) |
  | texture linear | 1.08 | 1.08 | 3.72 | 3.70 | 16.95 (59) |
  | texture mipmap | 1.09 | 1.09 | 3.70 | 3.67 | 16.95 (59) |
  | shading gouraud | 1.42 | 1.43 | 3.69 | 3.66 | 16.95 (59) |
  | shading blinn-phong-inf | 1.65 | 1.65 | 4.43 | 4.42 | 16.95 (59) |
  | shading phong | 2.07 | 2.07 | 4.49 | 4.46 | 16.95 (59) |
  | shading cel | 2.43 | 2.42 | 4.43 | 4.43 | 16.95 (59) |
  | bump high-poly | 3.03 | 3.03 | 4.66 | 4.62 | 16.95 (59) |
  | bump normals | 1.18 | 1.19 | 4.42 | 4.42 | 16.95 (59) |
  | bump height | 1.42 | 1.42 | 4.48 | 4.46 | 16.95 (59) |
  | effect2d edge (3x3) | 5.81 | 5.81 | 4.46 | 4.46 | 25.00 (40) |
  | effect2d 5x5 | 8.99 | 8.99 | 4.47 | 4.47 | 35.71 (28) |
  | pulsar | 1.13 | 1.13 | 4.54 | 4.52 | 18.18 (55) |
  | desktop blur | 21.31 | 21.32 | 11.18 | 13.06 | 33.33 (30) |
  | desktop shadow | 6.92 | 6.92 | 6.51 | 6.87 | 33.33 (30) |
  | buffer map | 16.18 | 16.15 | 10.12 | 10.14 | 33.33 (30) |
  | buffer subdata | 11.41 | 16.37 | 8.97 | 10.03 | 33.33 (30) |
  | buffer interleaved map | 14.55 | 14.45 | 8.80 | 8.76 | 33.33 (30) |
  | ideas | 5.58 | 5.72 | 12.40 | 11.85 | 33.33 (30) |
  | jellyfish | 6.31 | 6.30 | 4.50 | 4.50 | 33.33 (30) |
  | **terrain** | **90.91** | 93.95 | 161.79 | 179.26 | 250.00 (4) |
  | **shadow** | **10.64** | 353.23 | 7.34 | 7.32 | 500.00 (2) |
  | **refract** | **37.03** | 375.53 | 17.27 | 18.72 | 500.00 (2) |
  | conditionals 0/0 | 1.06 | 1.06 | 4.37 | 4.36 | 17.54 (57) |
  | conditionals frag 5 | 1.84 | 1.84 | 4.36 | 4.36 | 19.61 (51) |
  | conditionals vtx 5 | 1.06 | 1.06 | 4.36 | 4.36 | 17.54 (57) |
  | function low | 1.27 | 1.28 | 4.36 | 4.37 | 19.61 (51) |
  | function medium | 1.80 | 1.79 | 4.37 | 4.37 | 19.61 (51) |
  | loop fragment-loop=false | 1.35 | 1.35 | 4.37 | 4.37 | 17.54 (57) |
  | loop uniform=false | 1.35 | 1.36 | 4.37 | 4.37 | 19.61 (51) |
  | loop uniform=true | 3.89 | 3.89 | 4.37 | 4.37 | 19.61 (51) |
  | sum | 273.76 | 967.09 | 347.45 | 370.50 | 1932.65 |

- The AM62P row is from main `c5dcf5e8` plus the PowerVR depth-texture
  fix of #40 (the image's tree `c999f2f7`, 2026-10-27,
  `records/am62p/common-c999f2f7/20261027-2252-GM-0de4a19e/`, the GM
  image of `cts-common.sh am62p build`, `tools/board.sh am62p run` through
  the feed bridge of a console lease, `uboot-mmc`). `verdict=done`, resets
  0, GL errors 0, 0 refused draws. Record words GM
  `594b1201,fc3f01d3,6184003f,00258320`. **Score 467** (461 at `445c5653`
  and `2483a146`; 465 at `cd738b1a`; 289 at `1d54a4eb`, the in-place swap;
  462 at `e9f562d8` and `bc43ea09`, after the EGL double buffer). Against
  `445c5653`: shadow 353.23 -> 10.64 ms, refract 375.53 -> 37.03 ms, terrain
  93.95 -> 90.91 ms, build use-vbo=false 10.00 -> 5.79 ms, the other 29
  scenes within 0.5 percent; summed 967.09 -> 273.76 ms. The backend's exit
  line: `ds slots peak 5/32, ..., depth sampled from the buffer 2026
  (refused 0)`. Pictures against the `am62p` column (the `445c5653` run's):
  `33 scenes, match 29, mismatch 4`, all four probe c of animated scenes
  (buffer map off 40 and 11, terrain 81, refract 48), shadow a match
  (probes c `a0a0a0`, bl `4d4d4d`, br `4c4c4c`, mean 34 as recorded), none
  black. gl_test's 50 of 50 is the `445c5653` run's (`records/am62p/common-445c5653/`,
  GT `20261020-1723-GT-4c3e6ee4`), not rerun; case 215 `shadowmap` ran on its
  own at `c999f2f7` (`records/am62p/shadowmap-c999f2f7/`, PASS, 27.75 ms a
  frame) and at main plus the case (`records/am62p/shadowmap-base/`, PASS,
  355.20 ms a frame).
- The Pi 4 row is from main `c4af4917` (2026-10-22,
  `records/pi4/20261022-0308-glmark2-c4af4917/`, GM run
  `20261022-0308-GM-2e41f5e9`): the GM image of `cts-common.sh pi4 build`
  (the sample's `rpi_4b.conf`: `CONFIG_YAKOGL_V3D_TFU=y`, 96 MiB
  carve-out), one tryboot with the lab's 17-field `hdmi_timings`, window
  800x600, EGL double buffer, one GL client on the board. The image was
  staged and the console read by hand, the same steps as `board.sh pi4 run`
  (the session could not hand `board.sh` its console and panel settings);
  `board.sh pi4 tally` recomputes the verdict from the run's files: `rc=0
  verdict=done boot_verdict=complete result=record:GLMARK2 resets=0 record=594b1201,fc3f00c3,6184003f,00258320`.
  GT was not rerun (12 of 12 at `445c5653`). **Score 195** (193 at
  `445c5653` and `dcbfb30b`). 0 GL errors, 0 refused draws, 0
  incomplete-texture draws, 0 GPU resets. Against `445c5653`: terrain
  179.26 -> 161.79 ms (-9.7 %; GL time 173.9 -> 156.1 ms, its uniforms step
  7.8 ms -> 9 us, no `fs uniform[` line printed), refract 18.72 -> 17.27 ms
  (-7.7 %; GL time 11.59 -> 1.81 ms, swap 7.17 -> 15.42 ms: the wait moved
  to the swap and the frame is now its jobs' GPU time, 16.8 ms at frame 10),
  shadow 7.32 -> 7.34 ms (its jobs take 7.2 ms of GPU), build use-vbo=false
  6.42 -> 4.75, desktop blur 13.06 -> 11.18, buffer subdata 10.03 -> 8.97,
  desktop shadow 6.87 -> 6.51, ideas 11.85 -> 12.40 (+4.6 %, one run; 11.34
  to 12.04 before); every other scene within 1.1 %. Summed 370.50 -> 347.45
  ms. Carve-out high-water 69,092,096 of 100,134,912 bytes, in refract
  (76,772,096 at `dcbfb30b`: the deferred R32F depth storage, 7,680,000
  bytes). Terrain: 9.0 jobs a frame (560 over 62), each frame's swap waits
  (`end waited for in 62 of 62`); 3 TFU chains a frame, all queued behind a
  job, 0 waited; the GPU work of frame 10 is 29.1 ms (8 jobs, 4 TFU jobs
  1.8 ms), so about 130 ms of the 161.8 ms is CPU time outside the timed
  draw steps (sum 0.17 ms) (#1209). Pictures: none black, none absent; 31 of
  33 match the `pi4` column (desktop blur probe tr off 63, jellyfish probe c
  off 33, both animated).
- AM62P glmark2 shadow and refract (353 and 375 ms a frame at `445c5653`, #40, #1006) were the CPU resolve of the 1600x1200 depth texture every frame (`docs/notes/pvr-depth-sample.md`): with the TPU sampling the depth buffer (the run above) shadow is 10.64 ms (33x) and refract 37.03 ms (10x); terrain stays slow (90.9 ms, CPU GL time). gl_test case 215 `shadowmap` (the scene's two passes a frame on a 1600x1200 depth texture, 20 frames) read on the board 355.20 ms a frame (5.4 Mpx/s) from main `c5dcf5e8` plus the case alone (`records/am62p/shadowmap-base/`), 27.75 ms (69.1 Mpx/s) from `c999f2f7` (`records/am62p/shadowmap-c999f2f7/`), the GPU's busy time 6.35 and 6.37 ms of it (TA|3D), the four picture points equal in both, `depth sampled from the buffer 20 (refused 0)` in the second. The `bc43ea09` image includes the PowerVR `glGenerateMipmap` on the GPU (`d3b8d4b2`). Before it, at `ab43a20c` (`records/am62p/gm-kt-e9d63d06`), terrain took 1,466.6 ms a frame and refract 2,041.9 ms, all CPU time.
- AM62P pictures by eye (the 33 scenes of GM `bc43ea09` `-1739`, the column
  `am62p` until `445c5653`, whose run now gives it and matches it in 29 of
  33 scenes, the 4 others being animated probe c values). Looked at on
  the AM62P panel through the farm camera while the GM
  image ran, in sequence: build (horse), texture (cube), shading (horse, then
  cat; gouraud blue, cel with the green rim), bump (moon, three variants),
  effect2d (edge map, then the white blur), pulsar (coloured quads), desktop
  (windows with penguins), buffer (wave), terrain (green hills), shadow (horse
  with its shadow on the floor), refract (bunny); the shader scenes
  (conditionals, function, loop) showed the rotating plane. The camera's
  exposure saturates bright scenes (66 ms shutter, locked), so colour and
  shading of the shader scenes, desktop and terrain were judged from the probes
  (none black, means 54 to 56 on the shader scenes), not by eye. Ideas and
  jellyfish were not identified with certainty in a frame (a sphere over a plane
  was seen between buffer and terrain), so those two are by value only.
  Verdict lines of the tally against the independent `imx8mp` column (a different
  GPU; tolerance 8): `33 scenes, match 23, mismatch 10`; the ten are animated
  scenes whose last frame is at another time on each board (texture nearest probe
  c off 13, pulsar br 9, desktop shadow tr 128, buffer c 28 / 40 / 68, jellyfish c
  48, terrain c 15, shadow c 28, refract c 146) (none is a static scene). The earlier
  run of the same image (`-1731`) against the `am62p` column: `33 scenes, match
  29, mismatch 4` (desktop blur bl 246, buffer c 24 / 36 / 36, all animated). The
  `[conditionals] vertex-steps=5` scene that was unexplained in an earlier
  session now matches the `imx8mp` column (tr `fdfdfd`), so that earlier black tr
  was the plane's position at the last frame, not a wrong picture. The tally's
  `--gm-pictures ... am62p` reads `33 scenes, match 33`; that is the column
  against the run it was made from, not a check.
- i.MX8MP glmark2 GL errors (#814): the cause is the GC7000's 64 MiB GPU
  carve-out, from which every GC allocation (the board layer's 800x600 window
  and 1280x800 screen copy included) is taken. A diagnostic run from branch
  `b421b447` (main `ab43a20c` plus the 240 MiB carve-out of the glmark2
  image's overlay and the first-GL-errors instrumentation), `board-stage.sh
  imx8mp common` twice, read GM `594b120e fc3f002a 6184003f 00258320`: score
  42, 33 ran, 0 failed, **0 GL errors**, in both runs; GT 48 of 49 (case 44, #6) then 49 of 49. The glmark2 cell above is
  now a run from main (`bebce725`, GL errors raised 0 in all three arms).

## Mali: NanoPC-T6 (RK3588, Valhall v10, panthor), gl_test sweep

`panthor-gltest --sweep` on the board's own Linux (panthor, not Zephyr), the
newest record: `tools/cts/log/2026-10-27-frameboard/` (board clock
2026-10-27, 11:56 UTC; plan #153 frameboard, after the face-culling
(`272033a3`) and RGBA16F-target merges). Code: the
integration tip `0543b60b` plus the record's predictions commit, cross-built
on the host (`yakogl-build=376b9cf8`).

| Runs | Cases | PASS | REFUSED | NO-CASE | SKIP | FAIL | Kernel messages |
|---|---:|---:|---:|---:|---:|---:|---:|
| sweep 1 and 2, equal (the 256 verdict lines are identical) | 256 | **102** | 59 | 86 | 8 | 1 | 0 |

- The PASS set is the previous record's 98 (`2026-10-24-refusals`) to the
  string, plus:
  - 4 `cull` (face culling, #32, #33)
  - 54 `es3chain` (RGBA16F colour targets)
  - 73 `branchlane` (was refused as arithmetic other than +, -, *)
  - 114 `cubelod` (was refused as a constant varying lane)

  Eleven rows moved from one refusal to another (63, 68, 71, 74, 78, 81,
  100, 101, 116, 142, 148). Every earlier sweep record's PASS set is a
  subset of this one (`tools/mali/sweep_passsets.py` over the 48 records of
  `0dce654a8:tools/mali/board-ref/` and this one: 49 records).
- One FAIL, a case new in gl_test (NO-CASE before): `verdict case 214
  splitkick: FAIL rc -1, first bad check 5 got 0xffc0c0c0 want 0xff808080`.
  A 4x frame split into two kicks: the second kick reloads the resolved
  colour, not the four samples, so the diagonal is blended twice.
- Cases 4, 52 and 54 also PASS in two runs each on their own. Cases 8, 16,
  27, 28, 29, 30, 31, 32, 33, 36, 37, 38, 39, 41, 52, 56, 87, 108, 113, 114,
  121, 122, 169, 186, 187, 190, 196, 197, 198, 207, 208, 209, 210, 211, 212
  and 213 PASS.

## ES 3.0: dEQP-GLES3 (stage 4)

Mustpass list: 42,810 cases. Images report ES 3.0 (`REPORT_ES2` off).

**AM62P, whole tree** on `ad2b5ed` (main `e51de0af` plus docs; the same
`src/`, `driver/` and `capi/` as main `a63fcc5b`), 2026-10-14:

| Pass | Fail | NotSupported | QualityWarning | No result |
|---:|---:|---:|---:|---:|
| 36,552 | 2,880 | 2,195 | 185 | 998 |

- Every case that is not Pass: `docs/es3/am62p-gles3-notpass.tsv`.
- The 998 without a result: 987 were image 7 stopping at
  `shaders.builtin_functions.precision.clamp.highp_fragment.vec3`
  (`ResourceError`), and 11 slow `texture.filtering.3d` cases did not start.
  Images 20 and 21 hang the GPU in `texture.filtering.3d`
  (`docs/notes/pvr-3d-mipmap-hang.md`).
- Image 7 after `ffee428a`, on the board 2026-10-25 (main `be84094b`): the
  `clamp.highp_fragment.vec3` case passes and the 48 `common.round` /
  `roundeven` cases pass. The old image still hit the 600 s watchdog after 976
  cases (`precision` runs 0.7 to 1.1 s a case), so the table is now 31 images
  (`precision` weighted 6x, README section 2e). On that table images 7 to 10
  (420, 330, 366 and 1,119 cases) and image 6 (1,633 cases) each ended `done`
  with every begun case given a result; image 7 returned after 531 s of the 600
  s. Not yet run: the other 26 images of the 31-image table.
- Failing there, not investigated: `common.modf` (24 cases) and the
  `precision` `mat4`, `mat4x2` and `mat4x3` cases of `matrixcompmult`,
  `outerproduct` and `transpose`.
- Rows re-run later replace their groups (2026-10-14, 0 resets):
  on `cd738b1a` transform_feedback 882 / 882, ubo 1,099 / 1,099,
  occlusion_query 62 / 62, `fbo.blit.conversion` 325 pass / 0 fail / 112
  NotSupported, `fragment_ops.scissor` 26 / 26; on `4e62d171`
  `fragment_ops.blend.fbo_srgb` 1,152 / 1,152.

**AM62P, gl_test ES 3.0 cases** on `cd738b1a`: 196 `occl`, 200 `intrt`, 201
`usamp`, 202 `blitconv`, 203 `tfcap`, 204 `srgb` and 205 `mappbo` all pass, 0
resets.

**Pi 4, first nine images** on `925dbca9` (rerun of the integration tip, tryboot, console
through the UART bridge in `$BOARD_UART`; `tools/cts/pi4-gles3-tree.txt`, 91 images; recipe
`docs/es3/board-recipe-pi4.md`; commit `96810ab2`): 9 of 91 images (A1, B1, C1, D1,
FB1, FB2, FB3, FB4, FO1), 4,917 cases, 3,943 pass / 643 fail / 331
NotSupported / 0 QualityWarning / 0 without a result, 0 GPU recoveries; `buffer` 695,
`clipping` 602 and `fragment_ops` 677 pass in full. Rows 9-90, the ES 2.0 regression
rows and the ES 3.0 gl_test cases were not run on this commit. Every case that is not
Pass: `docs/es3/pi4-gles3-fails.tsv` (974 rows: `fbo.render` 305, `fbo.color`
206, `fbo.blit` 194, `fbo.msaa` 66, `draw_buffers_indexed` 76 NotSupported,
`fbo.completeness` 45, `fbo.invalidate` 38, `attribute_location.mixed_max_attributes`
20, `fbo.depth` 10, `implementation_limits` 11). Against the earlier seven-image
run (`cd738b1a`, replaced): `attribute_location` 48 -> 20, `draw.random` 14 -> 0,
`fbo.blit` 308 -> 48 Fail, `fbo.color` 215 -> 173 Fail, `fbo.completeness` 134 -> 38.
gl_test on the Pi 4 (last run on `cd738b1a`, not repeated): 202, 203 and 205 pass;
196 `occl` (all results 0) and 204 `srgb` (render half refused) fail.

**Host, ES 3.0 back-end capability (HOST-ONLY, no GPU result)**: the GC7000UL
samples eight more storage classes in the library (`541975a2`; descriptor
`dc83be67`, `docs/es3/gc-formats.md`). Measured at `541975a2` with
`tools/cts/host-deqp-run.sh --gpu=vivante --module=gles3`: `texture.format`
546 cases, 498 fail / 48 NotSupported, upload refusals 200 -> 64 (compile 0,
image 434); `texture.specification` 1,163 cases, 6 pass / 1,157 fail, upload
refusals 448 -> 141 (compile 0, image 1,013, framebuffer incomplete 3).
Per-group host verdicts on the three GPUs: `docs/es3/host-gles3-tip.tsv`
(`70142d35`).

**i.MX8MP**: not run on the board (recipe `docs/es3/board-recipe-imx8mp.md`).
