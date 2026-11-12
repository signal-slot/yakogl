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
| AM62P, 17 images at 1x (re-run at `36c4305a`, 2026-11-05, `records/am62p/deqp-36c4305a/`, 17,371 cases begun: the list plus 206 outside it) | 17,332 | **0** | 34 | 5 / 0 | 0 |
| AM62P, ms4 image (`multisample.*` on the 4x config) | 16 | **0** | 0 | 0 / 0 | 0 |
| AM62P combined (ms4 replaces the 16 1x `multisample.*`) | 17,142 | **0** | 18 | 5 / 0 | 0 |
| i.MX8MP, 29 entries (re-run at `0fbe61a3`, 2026-10-31, `records/imx8mp/deqp-0fbe61a3/`, 17,371 cases begun: the list plus 206 outside it) | 17,318 | **0** | 34 | 19 (other) | 0 |
| Pi 4, 29 rows (every list case has a result; re-run at `46f93767`, 2026-10-24, `records/pi4/deqp-46f93767/`) | 17,126 | **0** | 34 | 3 / 2 | 0 (0 recoveries in 29 records) |

- The 34 NotSupported are the same on the three boards and accepted
  (`docs/es2-cts-failures.md` section C, `tools/cts/es2-accepted.txt`):
  `clip_control` 8, `negative_api.texture.compressedtexsubimage2d_*` 9,
  `multisampled_render_to_texture.readpixels` 1 and `multisample.*` 16 (no
  multisample surface at 1x; the AM62P runs them on its ms4 image).
- The AM62P 1x row is from `36c4305a` (2026-11-05,
  `records/am62p/deqp-36c4305a/`, 17 images, every case has a result, every
  image's own summary counts every case begun with a result; image 12 was
  re-armed once after an interrupted run). The numbers equal `18462c90`'s
  (2026-11-01). Against
  `177c62fd` (17,330 pass, 5 QualityWarning and 2 CompatibilityWarning) the
  two `flush_finish` timing cases pass without dEQP's warning: 17,332 pass,
  0 fail, 34 not supported, 5 QualityWarning and 0 CompatibilityWarning
  over all 17,371 cases begun. On the 17,165-case list it reads 17,126
  pass. The ms4 row is still from `e9901869a`; the combined row above is
  this run's 17,126 plus the 16 ms4 passes.
- The cases dEQP's timing check warns on are `flush_finish.flush` and
  `flush_finish.flush_wait` (the CompatibilityWarnings of the older runs).
  The AM62P 1x runs at `e9901869a`, `d61a4c5d`, `f41a19e8` and `177c62fd`
  warned on both, the run at `a917d123` only on `flush_finish.flush`; the
  run at `316b4ad7` above neither, both pass. The Pi 4 run at `46f93767`
  warned on both (at `89b4a1c7` only on `flush`).
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
| AM62P | 468, 33 scenes, 0 failed, 0 GL errors; `6a7021ec` (#42's path B, off by default, and the case 216 kick count; built before its rebase onto main `561bad55`, as `e4819b69` on main `61ad38a1`), 2026-11-01 (`records/am62p/common-e4819b69/20261101-1935-GM-4bcd9ff3/`, `verdict=done`, resets 0; the first run since `ce3354ac`, the HWRT second pair); **shadow 10.62 ms a frame (95 FPS), refract 35.09 ms (29 FPS), terrain 83.06 ms (13 FPS)** (10.63, 37.05, 90.94 at `86d0b482`); per-scene ms/frame below; pictures against `tools/cts/glmark2-pictures.txt` column `am62p` (the `445c5653` run's): 33 scenes, match 28, mismatch 5, each one probe of an animated scene (desktop blur tr, buffer map and interleaved map c, terrain c off 10, refract c — sampled at a different frame count), shadow a match, none black | 49 of 50 (rerun at `6a7021ec`, GT `20261101-1943-GT-0225c49e`, as at `86d0b482`): case 20 `etc` fails its first check alone — it wants 3 compressed formats, the build stores 5, the intended ES 3.0 list on the PowerVR (`tests/es3_caps.rs` `known_gaps`); the test's expectation is stale, its other 16 checks pass (below); `445c5653` read 50 of 50; case 215 `shadowmap` PASS on its own, below) | 0 |
| i.MX8MP | 42, 33 scenes, 0 failed, 0 GL errors (#814 fixed by #922); main `445c5653`, 2026-10-20, 1 screen buffer (the default); re-run from main `a917d123` (holds the GC mipmap offer `dc033dc44`, #1106) the same day: score 42 (`records/imx8mp/common-a917d123`, GM `verdict=done`, words `fc3f002a`); re-run from main `f87dde43` on 2026-11-07 (`tools/cts/board-stage.sh imx8mp common`, `uboot-mmc`, `records/imx8mp/common-f87dde43/`): score 42, 33 scenes, 0 failed, 0 GL errors, words `fc3f002a` again, gl_test 49 of 49 run, case 16 skipped, both rows `verdict=done`, resets absent (no per-scene record: the board has no console); per-scene record `records/imx8mp/20261020-2100-glmark2-a917d123/` (66 boots, ms a frame = 1000 / the integer FPS): **terrain 250 ms (4 FPS), shadow 500 ms (2 FPS), refract 500 ms (2 FPS)**, the other 30 scenes 16.95 to 35.71 ms; 1934.56 ms summed (1932.65 at `445c5653`), so the offer moved no scene at this resolution; all 33 pictures match the `imx8mp` column, none black, terrain and refract means equal to the `445c5653` ones (not looked at by eye); per-scene FPS and picture values from the SNVS record (below: terrain 4, shadow 2, refract 2 FPS), pictures in `tools/cts/glmark2-pictures.txt` column `imx8mp` (33 from this run, none black, not looked at by eye); 49 at 3 buffers, 46 at 2 (`bebce725`, below) | 49 of 49 run, case 16 skipped (#4); cases 10 and 17 nondeterministic (#5, #3: 11 boots of `work/egl-double-buffer`'s code, case 17 failed 3, case 10 failed 1; main `0247ac665`, 10 boots: case 17 failed 3, case 10 failed 3, case 44 once) | 0 where counted |
| Pi 4 | 195, 33 scenes, 0 failed, 0 GL errors; main `c4af4917`, 2026-10-22 (the TFU no-wait, FS dump off, uniforms from the host arrays, lazy raster, depth storage defer, copy reason counters, UIF walker and buffer retired reuse; the main tip `71c73b0f` differs from it only in `tools/check.sh` and its selftest); GM `verdict=done`, record `records/pi4/20261022-0308-glmark2-c4af4917/`: **terrain 161.79 ms (7 FPS), shadow 7.34 ms (137 FPS), refract 17.27 ms (58 FPS)** (179.26, 7.32, 18.72 at `445c5653`); per-scene ms/frame below; pictures 31 of 33 match the `pi4` column (desktop blur and jellyfish, both animated, differ in one probe each), none black | 12 of 12 (at `445c5653`; not rerun) | 0 |
| NanoPC-T6 (Mali) | no score (1 of 33 scenes has an FPS), 33 of 33 scenes ran, 32 failed, 80 GL error lines, 0 out of GPU memory; main `a57d8104`, 2026-11-11 (the same outcome as the first record, main `da65355e` 2026-11-10), on the board's own Armbian trixie (kernel 6.18.55-current-rockchip64, panthor), `tools/cts/mali-glmark2-session.sh` over the `tools/cts/mali-glmark2-build.sh` tree (`dev=no`; one glmark2-es2 process a scene, 800x600 on an EGL pbuffer of the `tools/mali/panthor-gles` pair, libGLESv2.so.2 sha256 `283d99138184a8692cb09a24f194d2dc3b01bc43f43e63947bba1ee8a3383583`); **desktop shadow 0.96 ms a frame (1042 FPS), picture match; 31 scenes refused at `glCompileShader` by the online compiler (plan #309, #33); terrain aborts in glmark2 itself after its refusals**; per scene under "Mali: NanoPC-T6 ... glmark2-es2" below | see "Mali: NanoPC-T6 (RK3588, Valhall v10, panthor), gl_test sweep" | 0 (0 panthor kernel lines in every scene; same boot_id before and after; /boot and loaded modules unchanged, U-Boot env not compared: no fw_printenv on the board) |

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
  contexts on pbuffers) is nondeterministic on main too (#3 reopened) and not
  this tree's; #5 covers case 10.
- The README glmark2 table's runs: the GM row of the common set on the
  i.MX8MP from main `445c5653` (2026-10-20); the README's AM62P score is the
  run of main `86d0b482` (2026-10-28) until a main commit is measured, and
  the AM62P column below is the run of `6a7021ec` (2026-11-01, the row
  above); the Pi 4 column is the run of
  main `c4af4917` (2026-10-22, below). Per scene, ms/frame. The AM62P and Pi 4 columns are
  `glmark2-ms.py` / `common-tally.py --gm-table` (from `FrameTime`). The
  i.MX8MP column is 1000 / the integer FPS of its SNVS record, with the FPS
  in brackets. The earlier columns are the AM62P's 467 run of `86d0b482`
  (this table before) and the Pi 4's 193 run of `445c5653`.

  | Scene | AM62P | AM62P `86d0b482` | Pi 4 `c4af4917` | Pi 4 `445c5653` | i.MX8MP (FPS) |
  |---|---:|---:|---:|---:|---:|
  | build use-vbo=false | 5.79 | 5.81 | 4.75 | 6.42 | 33.33 (30) |
  | build use-vbo=true | 1.14 | 1.14 | 3.64 | 3.61 | 16.95 (59) |
  | texture nearest | 1.07 | 1.07 | 3.78 | 3.74 | 16.95 (59) |
  | texture linear | 1.08 | 1.08 | 3.72 | 3.70 | 16.95 (59) |
  | texture mipmap | 1.09 | 1.09 | 3.70 | 3.67 | 16.95 (59) |
  | shading gouraud | 1.42 | 1.42 | 3.69 | 3.66 | 16.95 (59) |
  | shading blinn-phong-inf | 1.66 | 1.66 | 4.43 | 4.42 | 16.95 (59) |
  | shading phong | 2.07 | 2.07 | 4.49 | 4.46 | 16.95 (59) |
  | shading cel | 2.42 | 2.43 | 4.43 | 4.43 | 16.95 (59) |
  | bump high-poly | 3.03 | 3.03 | 4.66 | 4.62 | 16.95 (59) |
  | bump normals | 1.19 | 1.19 | 4.42 | 4.42 | 16.95 (59) |
  | bump height | 1.41 | 1.42 | 4.48 | 4.46 | 16.95 (59) |
  | effect2d edge (3x3) | 5.81 | 5.82 | 4.46 | 4.46 | 25.00 (40) |
  | effect2d 5x5 | 8.99 | 8.98 | 4.47 | 4.47 | 35.71 (28) |
  | pulsar | 1.13 | 1.14 | 4.54 | 4.52 | 18.18 (55) |
  | desktop blur | 21.58 | 21.31 | 11.18 | 13.06 | 33.33 (30) |
  | desktop shadow | 6.80 | 6.92 | 6.51 | 6.87 | 33.33 (30) |
  | buffer map | 16.20 | 16.20 | 10.12 | 10.14 | 33.33 (30) |
  | buffer subdata | 11.43 | 11.42 | 8.97 | 10.03 | 33.33 (30) |
  | buffer interleaved map | 14.51 | 14.56 | 8.80 | 8.76 | 33.33 (30) |
  | ideas | 5.60 | 5.77 | 12.40 | 11.85 | 33.33 (30) |
  | jellyfish | 6.30 | 6.30 | 4.50 | 4.50 | 33.33 (30) |
  | **terrain** | **83.06** | 90.94 | 161.79 | 179.26 | 250.00 (4) |
  | **shadow** | **10.62** | 10.63 | 7.34 | 7.32 | 500.00 (2) |
  | **refract** | **35.09** | 37.05 | 17.27 | 18.72 | 500.00 (2) |
  | conditionals 0/0 | 1.06 | 1.07 | 4.37 | 4.36 | 17.54 (57) |
  | conditionals frag 5 | 1.83 | 1.84 | 4.36 | 4.36 | 19.61 (51) |
  | conditionals vtx 5 | 1.06 | 1.06 | 4.36 | 4.36 | 17.54 (57) |
  | function low | 1.27 | 1.27 | 4.36 | 4.37 | 19.61 (51) |
  | function medium | 1.79 | 1.79 | 4.37 | 4.37 | 19.61 (51) |
  | loop fragment-loop=false | 1.35 | 1.35 | 4.37 | 4.37 | 17.54 (57) |
  | loop uniform=false | 1.36 | 1.36 | 4.37 | 4.37 | 19.61 (51) |
  | loop uniform=true | 3.88 | 3.88 | 4.37 | 4.37 | 19.61 (51) |
  | sum | 264.09 | 274.07 | 347.45 | 370.50 | 1932.65 |

- The AM62P row is from `6a7021ec` (2026-11-01, `records/am62p/common-e4819b69/`,
  GM `20261101-1935-GM-4bcd9ff3`, GT `20261101-1943-GT-0225c49e`), built
  as `e4819b69` before the rebase onto main `561bad55` (main's commits
  between change the V3D compiler and back end, the Mali back end and
  tools, docs and gl_test case 217, none of the code these images run): main
  `61ad38a1` plus #42's path B (the mipmap chain's CPU tail, off by default
  since its board run, below), the case 216 kick count and a `pr-session.sh`
  fix; with the tail off its mipmap chain is main's. The GM and GT images of
  `tools/cts/board-stage.sh am62p common`, run by `tools/board.sh am62p run`
  (`uboot-mmc`), one GL client at a time. GM `verdict=done`, resets 0, GL
  errors 0, 0 draws that sampled an incomplete texture. Record words GM
  `594b1201,fc3f01d4,6184003f,00258320` (`86d0b482`'s but the score, 468
  for 467). It is the first AM62P glmark2 row since `ce3354ac` (the HWRT
  second pair), whose row was never updated. Against `86d0b482`: terrain
  90.94 -> 83.06 ms (121 frames, GL time 81.9 ms a frame), refract 37.05 ->
  35.09 ms, shadow 10.63 -> 10.62 ms, ideas 5.77 -> 5.60 ms, desktop blur
  21.31 -> 21.58 ms, the other 28 scenes within 0.15 ms; summed 274.07 ->
  264.09 ms. The backend's exit line: `ds slots peak 5/32, ds bytes peak
  15564800, ds given up 0, ds refused 0 (slots full 0, no buffer 0), depth
  sampled from the buffer 2088 (refused 0)`. Pictures against the `am62p`
  column (the `445c5653` run's): `33 scenes, match 28, mismatch 5`, each one
  probe of an animated scene — desktop blur tr off 63, buffer map c off 18,
  buffer interleaved map c off 12, terrain c off 10 (mean 119 129 44, the
  column's 120 129 44), refract c off 49; every still probe of every scene
  matches, shadow a match (probes c `a0a0a0`, bl `4d4d4d`, br `4c4c4c`,
  mean 34), none black. gl_test 49 of 50 as at `86d0b482`: case 20 `etc`
  fails its first check alone — `3 compressed formats` wanted, 5 seen. The
  gl_test image reports ES 3.0 (`driver/samples/zephyr/gl_test/prj.conf
  CONFIG_YAKOGL_REPORT_ES2=n`) and an ES 3.0 report on the PowerVR stores
  the sRGB ETC2 pair too (`src/gl/texture.rs` `stored_compressed_formats`),
  the list `tests/es3_caps.rs` `known_gaps` states as intended; the
  expectation in `driver/tools/pvrlab/gl_test.c` is what is stale, not the
  back end. Case 215 `shadowmap` ran on its own at `c999f2f7`
  (`records/am62p/shadowmap-c999f2f7/`, PASS, 27.75 ms a frame).
- AM62P #42 path B (`docs/notes/am62p-pvr-mipmap-kicks.md`: the GPU draws
  level 1 and the levels over 64x64 texels, waits for them, and the GL core
  filters the rest with the GPU's sample), 2026-11-01, standard images of
  `tools/cts/pr-session.sh --knob off`, one U-Boot one-shot boot each.
  gl_test case 216 `mipchains` (terrain's four chains a frame, 20 frames):
  main `61ad38a1` plus the kick count 46.15 ms a frame, 43.5 Mpx/s, 40
  kicks a frame, GPU TA|3D 7.75 ms (`records/am62p/mipchains-42/`, run
  `20261101-1810-pr-mip216-before-gltest-53f60fa3`); with path B on 82.95
  ms, 24.2 Mpx/s, 15 kicks, TA|3D 7.00 ms (run
  `20261101-1839-pr-mip216-after-gltest-4352c07f`); the four quadrant
  probes the same in both (PASS; the case has no record, `record=absent`).
  glmark2 with path B on (run `20261101-1917-pr-tail-on-glmark2-fd1287ab`):
  467, 33 scenes, 0 failed, 0 GL errors, terrain 123.8 ms a frame (81
  frames), refract 42.8 ms, summed 312.86 ms; terrain's mean 119 129 44,
  the same as with the tail off, its probes moved with the animation. The
  kicks fell as predicted and every chain-heavy scene got slower: the wait
  for the chain's kicks and the CPU tail's uncached reads (the note). So
  the tail is off by default (`Backend::mip_cpu_tail`), and this row runs
  every level on the GPU.
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
- AM62P glmark2 shadow and refract (353 and 375 ms a frame at `445c5653`, #40, #1006) were the CPU resolve of the 1600x1200 depth texture every frame (`docs/notes/pvr-depth-sample.md`): with the TPU sampling the depth buffer (main `86d0b482`, 2026-10-28) shadow is 10.63 ms (33x) and refract 37.05 ms (10.1x); terrain stays slow (90.9 ms then, 83.1 ms at `6a7021ec`, the row above; CPU GL time). gl_test case 215 `shadowmap` (the scene's two passes a frame on a 1600x1200 depth texture, 20 frames) read on the board 355.20 ms a frame (5.4 Mpx/s) from main `c5dcf5e8` plus the case alone (`records/am62p/shadowmap-base/`), 27.75 ms (69.1 Mpx/s) from `c999f2f7` (`records/am62p/shadowmap-c999f2f7/`), the GPU's busy time 6.35 and 6.37 ms of it (TA|3D), the four picture points equal in both, `depth sampled from the buffer 20 (refused 0)` in the second. The `bc43ea09` image includes the PowerVR `glGenerateMipmap` on the GPU (`d3b8d4b2`). Before it, at `ab43a20c` (`records/am62p/gm-kt-e9d63d06`), terrain took 1,466.6 ms a frame and refract 2,041.9 ms, all CPU time.
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
newest record: `tools/cts/log/2026-11-07-t6-step4a/` (board clock 2026-11-07,
15:25 UTC; plan #309 fix-matattr-63, after the step-4 lowerings merged).
Code: the integration tip `44f7b508` plus the record's predictions commit
`e8199bf4`, built on the board itself (`yakogl-build=v0.1.0-479-ge8199bf`).

| Runs | Cases | PASS | REFUSED | NO-CASE | SKIP | FAIL | FAULT | Kernel messages |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| sweep 1 and 2, equal (the 256 verdict lines are identical) | 256 | **147** | 15 | 83 | 9 | 2 | 0 | 0 |

- The PASS set is the previous record's 132 (`2026-11-01-final3`) to the
  string, plus 15, each `verdict case <n> <name>: PASS`: 46 es3size, 48
  es3vol, 49 es3facing, 63 matattr, 68 armselect, 71 deadloop, 74 ifoutput,
  76 boolany, 81 branchquad, 89 flatlast, 98 uniloop, 104 divloop, 141
  es3instdiv, 142 unibranch, 148 texbranch (was REFUSED compile, or for 63,
  98, 104 and 141 the FAIL/FAULT lines of final3). Every earlier PASS set
  (frameboard, tip, mid, final, final3) is a subset of this one
  (`tools/mali/sweep_passsets.py --delta tools/cts/log`: exit 0).
- FAIL (#32): `verdict case 55 es3mrt: FAIL rc -1, first bad check -1 got
  0x00000000 want 0x00000000` (GL error 0x500 from
  `glFramebufferTexture2D(GL_COLOR_ATTACHMENT1)`, before any draw) and
  `verdict case 206 es3levels: FAIL rc -1, first bad check 2 got 0x04602020
  want 0x04602090` (3D levels 1 and 2 read level 0).
- REFUSED, the 15 lines (#33 unless named): 19 (`REFUSED link: ... reads
  part of a varying ... ('Tint')`), 44 and 91 (`REFUSED draw: mali: a
  texture stored other than RGBA8, sRGB8_A8, R32F or ETC2 ... is not built
  yet`, #32: sampling an RGBA16F texture), 70 (a constant product), 77, 86,
  138, 181, 182, 188, 199 (arithmetic), 80 (an indexed local), 101 (a vertex
  look-up at a constant lane), 149 (a second vertex look-up), 218 (fp16
  arithmetic on gl_FragCoord; a case added since final3).
- Mesa 26.2.1 references of 63, 141, 55 and 91 on the board (record README
  section 2): 141's Attributes and Buffers, 55's blend descriptors and FBD,
  91's two RGBA16F blend descriptors, DCD0/DCD1 and render target are ours
  byte for byte, the addresses and the render targets' `Surface stride`
  aside; 63's programs are byte-equal to the in-tree references, its
  records differ in where the column offset goes (Attribute `Offset` in
  ours, the Buffer's address and size in Mesa's).
- The 20 cases predicted to pass, with 80, 101 and 149, also ran twice on their own
  (`cases-1.txt`, `cases-2.txt`, identical): `23 case(s) run: FAIL 2, PASS
  15, REFUSED 6`.

### Mali: NanoPC-T6 glmark2-es2 (stage 2 criterion)

Main `a57d8104`, 2026-11-11, the board's own Armbian trixie (kernel
6.18.55-current-rockchip64, panthor 1.5.0): `tools/cts/mali-glmark2-session.sh`
(`tools/cts/mali-glmark2-run.sh` between two `tools/cts/mali-board-state.sh`
snapshots) over the `tools/cts/mali-glmark2-build.sh` tree (`dev=no`, glmark2
`22c527cb` with `mali-glmark2-pbuffer.patch`, the `tools/mali/panthor-gles`
pair; every process's `yakogl-mali: loaded` line names the staged
libGLESv2.so.2, `build=a57d8104`). The same 33 outcomes as the first record
(main `da65355e`, 2026-11-10). Board state, before against after: `/boot`
unchanged (418 files), loaded modules unchanged (94), U-Boot env not compared
(the board has no `fw_printenv`), boot_id unchanged, the board reachable over
ssh after the session. One glmark2-es2 process a scene, 800x600
on an EGL pbuffer, 180 s bound, the 33 default scenes in order. Tallied by
`tools/cts/glmark2-mali-tally.py` (its verdicts agree with the runner's).

| Scenes | ok | refused at compile | crash | GL error lines | out of GPU memory | panthor kernel lines | boot_id |
|---:|---:|---:|---:|---:|---:|---:|---|
| 33 of 33 ran | **1** | 31 | 1 | 80 | 0 | 0 | unchanged |

- The outcome is the host forecast's scene for scene
  (`docs/notes/mali-glmark2-forecast.md`, section 1): the one clean scene is
  `desktop:windows=4:effect=shadow`, 0.96 ms a frame (1042 FPS), 270,819
  draws, `refused=0`, picture in the `mali` column of
  `tools/cts/glmark2-pictures.txt` (its frame looked at, four Tux windows
  with shadows over the Linaro background; the probes c, tl, tr, br and the
  mean agree with the AM62P column, probe bl is a moving window's edge). In
  this run the scene's own value differs from the column at probe tr only
  (`959595` against `c0c0c0`, a moving window's shadow at the last frame); a
  second process of the same build with `GLMARK2_FB=1` gave the column's
  probes c, tl, tr, br and mean, and its frame, decoded by
  `tools/cts/glmark2-fb.py` (600 of 600 rows, crc32 agrees), is the same
  desktop picture.
- The 31 refused scenes stop at the online compiler's first refusal, every
  one a plan #309 cause (#33): 18 scenes `VS: a constant product in a shape
  Mesa's algebraic rules move or fold` (build x2, texture x3, shading x4, bump x3,
  pulsar, buffer x3, shadow, refract), 8 `VS: arithmetic other than +, -, *
  and a dot or matrix product` (conditionals x3, function x2, loop x3), 2
  `FS: a constant operation on or to a value that is not a finite normal
  float` (effect2d 3x3, desktop blur), 1 `FS: a shader with more than 32
  local variables` (effect2d 5x5), 1 `VS: a fused multiply-add other than a
  matrix product's` (ideas), 1 `FS: a gl_FragColor lane other than the
  varying's own lane or a constant` (jellyfish). 27 of them print `Set up
  failed` and draw nothing; desktop blur and the three buffer scenes run on
  without their program (16 GL error lines each: `0x501` in
  `glGetAttribLocation`/`glGetUniformLocation`, `0x502` at the draw, what
  ES 2.0 asks for after a failed link) and their picture values (blur drawn
  without its blur pass; the buffer scenes all black) are no reference.
- terrain: `glmark2-es2-mali` aborts (SIGABRT, rc 134) in libstdc++'s
  `std::vector<int>::operator[]` assertion from `Mesh::render_vbo` after 8 of
  its 9 programs are refused (16 GL error lines before it): glmark2 indexes
  with an unlinked program's -1 attribute location. A consequence of the
  refusals, in glmark2 (the forecast's SIGSEGV; Debian trixie's g++ 14
  enables the assertion), not a YakoGL crash; no panthor line, no exit line.
  The runner's probe after it (`build:use-vbo=false`) passed and the run
  went on; the board answered with the same boot_id throughout.
- `D-DEPTHONLY` (shadow's depth-only framebuffer) and `D-NPOTMIP` (refract)
  were not reached: both scenes stop at the compile refusal first.
- The first session of the same build (17:00 UTC) gave the same 33 outcomes;
  it found two defects of the tools, fixed before this record and pinned by
  their self-tests: the runner judged the clean scene `failed-scene` because
  the back end's stdout log splits glmark2's result line from its header,
  and the tally refused the runner's directory because the `-b` scene text
  keeps upstream's option order (`pulsar:quads=5:texture=false:light=false`)
  while the info string sorts it. One scene of that session lost its kernel
  lines to a single failed `dmesg` read (now retried once); this record has
  them for every scene.

Per scene (ms a frame is glmark2's FrameTime; a refused scene has none;
picture against the `mali` column):

| # | Scene | ms/frame | FPS | Verdict | Picture | panthor lines |
| ---: | --- | ---: | ---: | --- | --- | ---: |
| 1 | `[build] use-vbo=false` | - | - | refused: VS constant product | absent | 0 |
| 2 | `[build] use-vbo=true` | - | - | refused: VS constant product | absent | 0 |
| 3 | `[texture] texture-filter=nearest` | - | - | refused: VS constant product | absent | 0 |
| 4 | `[texture] texture-filter=linear` | - | - | refused: VS constant product | absent | 0 |
| 5 | `[texture] texture-filter=mipmap` | - | - | refused: VS constant product | absent | 0 |
| 6 | `[shading] shading=gouraud` | - | - | refused: VS constant product | absent | 0 |
| 7 | `[shading] shading=blinn-phong-inf` | - | - | refused: VS constant product | absent | 0 |
| 8 | `[shading] shading=phong` | - | - | refused: VS constant product | absent | 0 |
| 9 | `[shading] shading=cel` | - | - | refused: VS constant product | absent | 0 |
| 10 | `[bump] bump-render=high-poly` | - | - | refused: VS constant product | absent | 0 |
| 11 | `[bump] bump-render=normals` | - | - | refused: VS constant product | absent | 0 |
| 12 | `[bump] bump-render=height` | - | - | refused: VS constant product | absent | 0 |
| 13 | `[effect2d] kernel=0,1,0;1,-4,1;0,1,0;` | - | - | refused: FS constant on a non-finite float | absent | 0 |
| 14 | `[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | - | - | refused: FS more than 32 locals | absent | 0 |
| 15 | `[pulsar] light=false:quads=5:texture=false` | - | - | refused: VS constant product | absent | 0 |
| 16 | `[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4` | 1.15 | 869 | refused: FS constant on a non-finite float (runs on, 16 GL errors) | no reference | 0 |
| 17 | `[desktop] effect=shadow:windows=4` | 0.96 | 1042 | **ok** | match | 0 |
| 18 | `[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map` | 0.71 | 1416 | refused: VS constant product (runs on, 16 GL errors) | no reference (black) | 0 |
| 19 | `[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata` | 0.69 | 1453 | refused: VS constant product (runs on, 16 GL errors) | no reference (black) | 0 |
| 20 | `[buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map` | 0.36 | 2801 | refused: VS constant product (runs on, 16 GL errors) | no reference (black) | 0 |
| 21 | `[ideas] speed=duration` | - | - | refused: VS fused multiply-add | absent | 0 |
| 22 | `[jellyfish] <default>` | - | - | refused: FS gl_FragColor lane | absent | 0 |
| 23 | `[terrain] <default>` | - | - | crash: glmark2 aborts after its refusals (16 GL errors) | absent | 0 |
| 24 | `[shadow] <default>` | - | - | refused: VS constant product | absent | 0 |
| 25 | `[refract] <default>` | - | - | refused: VS constant product | absent | 0 |
| 26 | `[conditionals] fragment-steps=0:vertex-steps=0` | - | - | refused: VS arithmetic | absent | 0 |
| 27 | `[conditionals] fragment-steps=5:vertex-steps=0` | - | - | refused: VS arithmetic | absent | 0 |
| 28 | `[conditionals] fragment-steps=0:vertex-steps=5` | - | - | refused: VS arithmetic | absent | 0 |
| 29 | `[function] fragment-complexity=low:fragment-steps=5` | - | - | refused: VS arithmetic | absent | 0 |
| 30 | `[function] fragment-complexity=medium:fragment-steps=5` | - | - | refused: VS arithmetic | absent | 0 |
| 31 | `[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5` | - | - | refused: VS arithmetic | absent | 0 |
| 32 | `[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5` | - | - | refused: VS arithmetic | absent | 0 |
| 33 | `[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5` | - | - | refused: VS arithmetic | absent | 0 |

## Mali: NanoPC-T6 on Zephyr (by kexec)

Zephyr images started out of the board's Armbian by kexec
(`driver/tools/kexec-zephyr/nanopc-t6.sh`) and judged against the predicted
record (`driver/tools/kexec-zephyr/t6-verdict.sh`). A development result: the
sign-off form (U-Boot starting the image) has not run. The steps Z1 to Z4,
their predicted records and the result of Z1's first half (`t6_hello`,
2026-10-19) are in `driver/docs/nanopc-t6-mali.md`; a step gets its row
here when it has run.

| Step | Image | Result | Build | Run | Record |
|---|---|---|---|---|---|
| Z1, second half: the GPU power-on | `t6_gpu` | **PASS**, `VERDICT t6_gpu PASS`: `t6_gpu: PASS: GPU_ID 0xa8670005, L2 ready, 16 ms` | `v0.1.0-915-g1bac19568` (main `1bac19568`) | 2026-11-11 21:36 JST | `records/nanopc-t6/20261111-213650-t6` |
| Z2: AS0 and a cache flush | `t6_mmu` | **PASS**, `VERDICT t6_mmu PASS`: `t6_mmu: PASS: AS0 on TRANSTAB 0x0000000010023000, three cache flush round trips, 13 ms` | `v0.1.0-913-g1f19113a3` (main `1f19113a3`) | 2026-11-11 21:01 JST | `records/nanopc-t6/20261111-210127-t6` |
| Z3: the firmware boots | `t6_fw` | **PASS**, `VERDICT t6_fw PASS`: `t6_fw: Z3: rc 0 step 0, 4 ms`, `t6_fw: PASS` | `v0.1.0-986-g988639752` (main `988639752`) | 2026-11-12 11:41 JST | `records/nanopc-t6/20261112-114132-t6` |

- Z2: every line of the predicted record was seen: Z1's power-on (`GPU_ID`
  0xa8670005, L2 ready), the cache flush round trip (2 polls of 1 µs), AS0's
  tables (4 of 16 pages, the walk of VA 0x04001234 reaches the backing),
  the enable (`LOCK`, `UPDATE`, `UNLOCK` without a wait), AS0 read back as
  written with no fault recorded (`MMU_INT_RAWSTAT` 0x00010000 is AS0's
  "command completed" bit, the fault bits 15:0 are clear), the flush after
  the map, and the PASS line 13 ms after Z2's sequence began (its prints
  included).
- The board came back to Armbian through the image's PSCI reset 30 s after
  the jump (`GLB_RST_ST` 0 before and after; Linux's watchdog was open at
  the jump, its `WDT_EN` proof is in the record).
- Z2's run does not show the power-on's PLL step: the GPU clock was on
  CPLL at hand-over, which was running. Z1's run shows it. It started from
  the hand-over that stopped the first run of the day, the GPU clock on
  NPLL (`CLKSEL_CON(158)` 0x00001f60) and NPLL powered down (`PLL_CON1`
  0x00002083), and the image powered the PLL and saw its lock: `t6_gpu:
  GPU PLL on: PLL_CON6 0x00004000 at hand-over, was on 0, waited 10 us,
  PLL_CON6 0x0000a400`. Then PD_GPU, the soft reset and L2 as predicted,
  the PASS line 16 ms after the power-on began (its prints included), and
  the board back in Armbian 30 s after the jump.
- That hand-over was made under Armbian by a write to the GPU devfreq's
  `min_freq` with the GPU suspended, not by a GPU load
  (`driver/docs/nanopc-t6-mali.md`, "The runs of 2026-11-11").
- Z2 by its design does not show the GPU's own walk of the tables: that is
  Z3's first fetch.
- The same day two runs of the image of main `3aa24a68d` stopped (Z1's
  power-on on a powered-down NPLL; the image's own reading of
  `MMU_INT_RAWSTAT`); both causes were corrected before this run
  (`driver/docs/nanopc-t6-mali.md`, "The runs of 2026-11-11").
- Z3: every line of the predicted record was seen (31 of 31). The image
  carried the board's firmware file (282624 bytes, sha256
  `a27847ea…a2405b`); its 7 loaded sections came up in AS0 on 9 of the 16
  table pages with 7 flushes, 1 protected section skipped, every cache
  clean rc 0. `MCU_STATUS` read 0 before and 1 after the start,
  `JOB_INT_RAWSTAT` 0x80000000 (the global interface bit). The firmware
  wrote its control block into our shared section: GLB version 1.5.0
  (0x01050000, the image's), features 0, instr 0x71, 8 groups of stride
  0xa0, the GLB, CSG 0 and CS 0 offsets as predicted. `GLB_REQ` and
  `GLB_ACK` both read 0x0000050e after the configuration (init req 0x40e,
  acked 0x10e) and the `PING`.
- Z3, the values only the board could give: the MCU's interface
  interrupt was seen 43 µs after the start (budget 1 s), the
  configuration was acknowledged in 32 µs and the `PING` in 5 µs (budget
  100 ms each), and CS 0's `features` word is 0x0007085f (96 work
  registers and 8 scoreboards, the counts panthor reports on this board).
  The firmware did not move the control block's offsets or strides.
- Z3: the whole sequence took 4 ms. The board came back to Armbian
  through the image's PSCI reset 28 s after the jump (`GLB_RST_ST` 0
  before and after; Linux's watchdog was open at the jump, its `WDT_EN`
  proof is in the record). Z3 is the GPU's own first walk of AS0's
  tables, which Z2 by its design did not show.
- Next: Z4 (`t6_submit`, one frame submitted from our kernel side).

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
