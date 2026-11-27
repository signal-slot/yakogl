commit: 02a2ed238208ec2e1534fe642f7c39b517c886fa

<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 at 02a2ed23: the re-split labels of the aborted images S03a and T14a (with T14) that had no result

Session `2026-11-pi4-gles3-02a2ed23-aborts-b`, item board-aborts of plan #399 (issue #1261). At `b1b5b845` nine images aborted on a refused draw: S03a, S07, T14a, T17a, TF1a, TF1b, TF2a, TF2b and TF2c. The tree at this commit (`tools/cts/pi4-gles3-tree.txt`, 151 images, `check-gles3-tree.py`: 42810 matched once) splits them into labels that isolate each aborting case. This session runs every one of those labels whose cases had no result in a merged `87881c78` tally:

- **S03a1, S03a2a..S03a7b** (16 labels, 1,141 mustpass cases). The `87881c78` S03a run (`../2026-11-pi4-gles3-87881c78-aborts-a/`) stopped at its 210 s record deadline with 937 cases without a result and no `deqp: summary` line. All sixteen labels run here, including those whose cases passed there, so the S03a family has one complete record.
- **T14, T14a1, T14aa, T14aa1** (242 mustpass cases), built with `driver/samples/zephyr/deqp/texture-2048.conf` as their `# image` lines say. No `87881c78` session ran them: `../2026-11-pi4-gles3-87881c78-tex2/` left them to this item. T14 is not one of the nine aborted images; it is the part of the old T14 before the abort, and its tree line was split with T14a's.

Not run here, because a merged `87881c78` tally already gives every one of their cases a result: the labels of S07, T17a (T17a1, T17aa, T17aaa, T17aaaa), TF1a (TF1a, TF1a1, TF1aa, TF1aaa, TF1aaaa), TF1b (TF1b, TF1b1, TF1ba, TF1baa, TF1baaa), TF2a (TF2a, TF2a1, TF2aa, TF2aaa, TF2aaaa), TF2b (TF2b, TF2b1, TF2ba, TF2baa) and TF2c (TF2c1, TF2ca, TF2caa). `../2026-11-pi4-gles3-87881c78-aborts-a/` ran their parent images S07, T17a, TF1a, TF1b, TF2a, TF2b and TF2c to `DONE, all passed`, `complete=1`, with all 871 mustpass cases passing. The re-split labels select exactly those cases (71 = 1+1+1+68, 184 = 3+1+2+1+177, 189 = 8+1+2+1+177, 177 = 20+1+21+21+114, 189 = 11+1+21+156, 60 = 1+6+53). The record checker reads those results under the parent label names, not the tree's current labels; the totals item maps them.

Per-image files, relative to this directory: `<LABEL>.console.txt` and `<LABEL>.tally.txt`. The counts below are the mustpass view of each tally's TOTAL line. "No result" means a case of the image's filter that never began. The "dEQP ran" column is dEQP's own counters (`deqp: done`), which count every case the filter ran, including the ones outside the mustpass.

| label | conf | mustpass | Pass | Fail | NotSupported | other | no result | dEQP ran | end line | carve-out high-water (KiB) |
|---|---|---|---|---|---|---|---|---|---|---|
| S03a1 | rpi_4b.conf | 1 | 1 | 0 | 0 | 0 | 0 | 1 P 0 F 0 N, complete=1, in 0.2 s | DONE, all passed | 20663 of 48636 |
| S03a2a | rpi_4b.conf | 82 | 82 | 0 | 0 | 0 | 0 | 94 P 0 F 0 N, complete=1, in 39.8 s | DONE, all passed | 21713 of 48636 |
| S03a2b | rpi_4b.conf | 108 | 108 | 0 | 0 | 0 | 0 | 168 P 0 F 0 N, complete=1, in 88.0 s | DONE, all passed | 22918 of 48636 |
| S03a3a | rpi_4b.conf | 83 | 83 | 0 | 0 | 0 | 0 | 83 P 0 F 0 N, complete=1, in 121.3 s | DONE, all passed | 21473 of 48636 |
| S03a3b | rpi_4b.conf | 81 | 81 | 0 | 0 | 0 | 0 | 96 P 0 F 0 N, complete=1, in 48.2 s | DONE, all passed | 21653 of 48636 |
| S03a3c | rpi_4b.conf | 26 | 26 | 0 | 0 | 0 | 0 | 38 P 0 F 0 N, complete=1, in 57.1 s | DONE, all passed | 22919 of 48636 |
| S03a4a | rpi_4b.conf | 85 | 85 | 0 | 0 | 0 | 0 | 87 P 0 F 0 N, complete=1, in 58.3 s | DONE, all passed | 21433 of 48636 |
| S03a4b | rpi_4b.conf | 105 | 87 | 18 | 0 | 0 | 0 | 95 P 18 F 0 N, complete=1, in 35.3 s | DONE | 23999 of 48636 |
| S03a5a | rpi_4b.conf | 68 | 68 | 0 | 0 | 0 | 0 | 76 P 0 F 0 N, complete=1, in 53.2 s | DONE, all passed | 22918 of 48636 |
| S03a5b | rpi_4b.conf | 122 | 104 | 18 | 0 | 0 | 0 | 104 P 18 F 0 N, complete=1, in 81.2 s | DONE | 21900 of 48636 |
| S03a6a | rpi_4b.conf | 45 | 45 | 0 | 0 | 0 | 0 | 54 P 0 F 0 N, complete=1, in 56.3 s | DONE, all passed | 21653 of 48636 |
| S03a6b | rpi_4b.conf | 12 | 12 | 0 | 0 | 0 | 0 | 12 P 0 F 0 N, complete=1, in 40.5 s | DONE, all passed | 22347 of 48636 |
| S03a6c | rpi_4b.conf | 12 | 12 | 0 | 0 | 0 | 0 | 12 P 0 F 0 N, complete=1, in 41.2 s | DONE, all passed | 22347 of 48636 |
| S03a6d | rpi_4b.conf | 121 | 121 | 0 | 0 | 0 | 0 | 135 P 0 F 0 N, complete=1, in 58.4 s | DONE, all passed | 22919 of 48636 |
| S03a7a | rpi_4b.conf | 88 | 88 | 0 | 0 | 0 | 0 | 96 P 0 F 0 N, complete=1, in 46.8 s | DONE, all passed | 22823 of 48636 |
| S03a7b | rpi_4b.conf | 102 | 84 | 18 | 0 | 0 | 0 | 84 P 18 F 0 N, complete=1, in 64.2 s | DONE | 22395 of 48636 |
| T14 | texture-2048.conf | 225 | 217 | 8 | 0 | 0 | 0 | 217 P 8 F 0 N, complete=1, in 27.7 s | DONE | 68111 of 114172 |
| T14a1 | texture-2048.conf | 1 | 0 | 1 | 0 | 0 | 0 | 0 P 1 F 0 N, complete=1, in 0.9 s | DONE | 63284 of 114172 |
| T14aa | texture-2048.conf | 15 | 15 | 0 | 0 | 0 | 0 | 41 P 6 F 0 N, complete=1, in 23.0 s | DONE | 106295 of 114172 |
| T14aa1 | texture-2048.conf | 1 | 1 | 0 | 0 | 0 | 0 | 1 P 0 F 0 N, complete=1, in 1.8 s | DONE, all passed | 91990 of 114172 |
| total | | 1383 | 1320 | 63 | 0 | 0 | 0 | | | |

Refusal cause of any stop: **none.** No label stopped.

## Result

Run on 2026-11-27 from 15:07 to 16:52 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr, tryboot), one cycle per image, with no retry. Every image is built from `02a2ed23`, the integration tip, which is on origin/main. It includes jobstatus-apply, so a refused draw would name its cause after `the job builder refused the draw`.

**Every one of the 1,383 mustpass cases has a result: 1,320 Pass, 63 Fail, 0 NotSupported, 0 other, 0 without a result.** All twenty images ended with `complete=1` and `deqp: summary: N cases begun, N with a result, 0 without`. No console has a refused draw (`draw refused` / `refused the draw`), a `Test run was ABORTED!`, a `STOPPED EARLY` or a ResourceError. Every image reports `allocations refused for lack of GPU memory: 0`, 0 slow jobs, 0 MMU faults, 0 recoveries, no hung job and `FIRST FAILURE: none`. No case needed a further split or a case list, so step 3 of the item (a rerun after a stopping case) did not apply.

- **S03a family:** 1,087 Pass and 54 Fail of 1,141. The longest image, S03a3a, took 121.3 s of the 200 s window. The 54 Fails are the mat4, mat4x2 and mat4x3 cases of `precision.matrixcompmult` (S03a4b), `precision.outerproduct` (S03a5b) and `precision.transpose` (S03a7b), at the six precision/stage pairs each (3 x 6 x 3 = 54). Every one fails to link with `glLinkProgram(2): link error: fragment output at location 3 ends at 4, beyond the 3 draw buffers` and dEQP reports `Fail (Shader compilation failed at glsBuiltinPrecisionTests.cpp:4967)`. This is the V3D back end's limit of 3 draw targets, the cause in `docs/notes/v3d-refusals.md` (items s03a-link-cause, s03a-link-pin, mrt4-design). It is a Fail, not an abort: the run goes on.
- **T14 family:** 233 Pass and 9 Fail of 242, all `Fail (Image comparison failed)`. T14 has 8 Fails: `texture.units.2_units.mixed.5`, `4_units.mixed.5`, `4_units.mixed.7`, `8_units.mixed.5`, `8_units.mixed.7`, `8_units.only_2d.7`, `8_units.only_2d_array.7` and `8_units.only_3d.7`. At `b1b5b845`, `docs/es3/pi4-gles3-fails.tsv` has 88 T14 Fails. T14a1's one case, `texture.units.8_units.only_cube.2`, where T14a aborted for lack of GPU memory at `b1b5b845`, now runs and fails image comparison. T14aa's 15 mustpass cases pass, and so does T14aa1's `texture.units.all_units.only_cube.0` (the case that might have run out of the carve-out). T14aa's six Fails are all outside the mustpass (`all_units.mixed.*`, `all_units.only_*`). Under texture-2048.conf's 114172 KiB carve-out, T14aa's high-water is 106295 KiB (93 %) and T14aa1's is 91990 KiB.

## Method

The workspace checkout at `02a2ed23` (`git describe`: `02a2ed23`, clean). This machine's dependency cache had no dEQP stage 1. So VK-GL-CTS was fetched at 93bca01 the way `tools/cts/deps.toml`'s `DEQP_WORK` entry does (clone, `external/fetch_sources.py`, rc 0). Then the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 was built (`cts-build-zephyr.sh`, `MODULE=gles3`, `BOARD=rpi_4b`), and it ended `cts-build-zephyr: libraries in <WORK>/build` after 453 ninja steps. Last came the images, built with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <dir> <the 20 labels>` (section 3). That printed twenty `rc=0 build=02a2ed23` lines. The sixteen S03a images are `RAM: 166476-166480 KB` (63.51 %) of the 256 MB window. The four T14 images, with `-DEXTRA_CONF_FILE=texture-2048.conf` (the dry run lists it), are `RAM: 232044 KB` (88.52 %). `cycles.txt` has each image's sha256.

The board runner could not capture the console: the Pi 4's UART bridge takes one client at a time, and the leased serial console (an MCP server that connected forwards) held it. As in the earlier hand-run sessions, `tools/board.sh pi4`'s steps were done by hand in its order, one image at a time:

1. Check that the board has no `/boot/firmware/yakogl-*` image, no `tryboot.txt` and no `config-linux.txt`. At the session's start, an image `yakogl-TF1b1.bin` and a `tryboot.txt` naming it were left on the board from an earlier run (written about 70 min before this session, not by it). They were removed before the first cycle. `config.txt` was not touched.
2. Copy the image to `/boot/firmware/yakogl-<LABEL>.bin` and write `tryboot.txt` with `pi4_config`'s lines (`[all]`, `kernel=yakogl-<LABEL>.bin`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`).
3. Read both back before the reboot: the image's sha256 is equal and the text is byte for byte the same.
4. Reboot with `reboot "0 tryboot"`.
5. Wait 235 s, which is past the image's 210 s record and before the ~250 s PM watchdog reset.
6. Read the console with `serial_read`. A script converted the tool's saved results, never copied by hand: it cut from the firmware's first line after Linux's `reboot: Restarting system` to the image's end line, and dropped CRs and NULs. The cut holds no other control byte.
7. Wait for Linux (a new boot id).
8. Remove the staged image and `tryboot.txt`, and check that neither is there.

The board is left as found at the end: no image and no `tryboot.txt`. Linux was back 299-309 s after each reboot.

Tallies: `tools/cts/pi4-session-tally.sh <this directory>` with the tree and mustpass list of this commit. `table.txt` is `deqp-uart-tally.py --mustpass ../../gles3-mustpass.txt --table *.console.txt`, run from inside this directory. Its counters column names the directory rather than the label. The consoles were then passed through `tools/cts/console-redact.py`.

## Files

- `<LABEL>.console.txt`: that boot's console, from the firmware's first line to the image's end line, after a three-line header.
- `<LABEL>.tally.txt`: `deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt --label <LABEL> <LABEL>.console.txt`, through `pi4-session-tally.sh`.
- `table.txt`: `--table` over the twenty consoles, with every mustpass case that is not a Pass named (63 `Fail`).
- `cycles.txt`: per image in run order: the sha256, the Linux boot id before and after, the reboot time, the seconds to Linux, the clean-up read-back, and the `cases in`, `carve-out`, `slow jobs` and end lines.

## Report texts (for plans #387, #389 and issue #1261)

Drafted from the merged state: `docs/es3/pi4-gles3-fails.tsv` (header "images run (151 of 151)"), `tools/cts/pi4-gles3-tree.txt` (151 images), the records `tools/cts/log/2026-11-pi4-gles3-*/` and `docs/notes/v3d-refusals.md`. The totals are the tsv's: 3,291 not-Pass rows (1,399 Fail, 1,877 NotSupported, 4 QualityWarning, 11 ResourceError); Pass is the rest, 42,810 - 3,291 = 39,519. 39,519 + 1,399 + 1,877 + 15 + 0 = 42,810. Nothing here edits `RESULTS.md`, `STATUS.md` or `ROADMAP.md`; plans #387 and #389 apply the texts.

### (a) tools/cts/RESULTS.md, 'Pi 4, whole tree'

Replace the span from `**Pi 4, whole tree**` up to, not including, `**Host, ES 3.0 back-end`, with:

```
**Pi 4, whole tree** on `b1b5b845`, `87881c78` and `02a2ed23` (`02a2ed238208ec2e1534fe642f7c39b517c886fa`;
all on origin/main; tryboot, console through the UART bridge; `tools/cts/pi4-gles3-tree.txt`,
151 images: the 91 images, re-split so that every aborting case sits in its own image;
recipe `docs/es3/board-recipe-pi4.md`), 2026-11-25 to 2026-11-27: 151 of 151 images run,
42,810 mustpass cases (each counted once, under the label the tree gives it, the newest
record of a case wins): 39,519 Pass / 1,399 Fail / 1,877 NotSupported / 15 other
(4 QualityWarning, 11 ResourceError) / 0 without a result, which adds up to 42,810.
Every case has a result. Commits: 76 images at `87881c78` (fbo, fragment_ops, fragment_out,
draw and colour-clear, texture T01-T13, T14b-T17b, and the aborted images S07, T17a, TF1a,
TF1b, TF2a, TF2b, TF2c), 20 at `02a2ed23` (S03a1 to S03a7b and T14, T14a1, T14aa, T14aa1,
the last four built with `texture-2048.conf`), 55 not rerun from `b1b5b845`. The nine images
that aborted at `b1b5b845` (S03a, S07, T14a, T17a, TF1a, TF1b, TF2a, TF2b, TF2c) no longer
abort: S07, T17a, TF1a, TF1b, TF2a, TF2b and TF2c pass all their 871 mustpass cases, the
S03a family has 1,087 Pass and 54 Fail of 1,141, the T14 family 233 Pass and 9 Fail of 242.
0 GPU recoveries, 0 hung jobs, 0 MMU faults. ES 2.0 regression rows (D, FB1, T1, T3, TF) at
`87881c78`: 1,184 of 1,184 mustpass Pass (1,245 of 1,245 begun), unchanged. Records:
`tools/cts/log/2026-11-pi4-gles3-b1b5b845-{rows00-15,rows16-30,rows31-45,rows46-60,rows61-75,rows76-90,s32c,aborts,aborts2,aborts3}/`,
`tools/cts/log/2026-11-pi4-gles3-87881c78-{fbo,tex1,tex2,aborts-a,es2}/` and
`tools/cts/log/2026-11-pi4-gles3-02a2ed23-aborts-b/`. Every case that is not Pass:
`docs/es3/pi4-gles3-fails.tsv` (3,291 rows). Remaining: no abort on the V3D 4.2 board. The
refusals that are left are Fails, not aborts: the 3 draw targets of the V3D (54 S03a
`precision.{matrixcompmult,outerproduct,transpose}` mat4, mat4x2 and mat4x3 link fails and 26
`fragment_out.random` cases, `docs/notes/v3d-refusals.md` G); `-E2BIG` for the texture units of
T14a on `rpi_4b.conf` (with `texture-2048.conf` it runs); the six T09 `size.2d.2048x2048_*`
cases refused under `rpi_4b.conf`'s tiled-copy budget; ASTC (T01, T15, T16, T17: not advertised).
The largest groups of Fail still open: fbo 370 (FB3 50, FB4 317, FB2 3), `fragment_out` 31, A1 2;
see the issue texts below.
```

### (b) tools/cts/STATUS.md, the Japanese Pi 4 line

Replace the span from `Pi 4 は全ツリー` up to, not including, `i.MX8MP はまだボー`, with:

```
Pi 4 は全ツリー（151 画像。中断した 9 画像を、中断するケースごとに分けて再分割したもの）を b1b5b845、87881c78、02a2ed23（いずれも origin/main、2026-11-25〜27）で実行し、必須 42,810 件中 合格 39,519 / 失敗 1,399 / 未対応 1,877 / その他 15 / 結果なし 0（全ケースに結果がある。GPU 復旧 0、ハング 0、MMU フォールト 0）。b1b5b845 で中断した S03a S07 T14a T17a TF1a TF1b TF2a TF2b TF2c はもう中断しない（S07 T17a TF1a TF1b TF2a TF2b TF2c は 871 件すべて合格、S03a は 1,141 件中 合格 1,087 / 失敗 54、T14 系は 242 件中 合格 233 / 失敗 9）。残る拒否は失敗であって中断ではない（V3D の描画ターゲットが 3 つのため S03a の mat4 系 54 件と fragment_out.random 26 件、rpi_4b.conf での T09 の 2048x2048 の 6 件、ASTC は未対応）。ES 2.0 の回帰行（D FB1 T1 T3 TF）は 87881c78 で必須 1,184 件すべて合格。失敗の一覧は docs/es3/pi4-gles3-fails.tsv、記録は tools/cts/log/2026-11-pi4-gles3-*/。
```

### (c) ROADMAP.md stage 4, the Pi 4 bullet

Replace the bullet that begins `- Raspberry Pi 4: the whole tree (131 rows` and ends at `need a rerun.` with:

```
- Raspberry Pi 4: the whole tree (151 images, the nine aborted images re-split so that
  each aborting case is isolated) at `b1b5b845`, `87881c78` and `02a2ed23`
  (2026-11-25 to 27): 42,810 cases, 39,519 pass / 1,399 fail / 1,877 NotSupported /
  15 other / 0 without a result, 0 GPU recoveries (#1261). Every case has a result: the
  nine images that aborted on a refused draw no longer abort. The fails left are
  not aborts: the V3D's 3 draw targets (80 cases in FR2 and S03a), the fbo groups
  of `docs/es3/pi4-gles3-fails.tsv`, ASTC (not advertised) and the 2048x2048
  textures under `rpi_4b.conf`'s budget.
```

### (d) comment for issue #1261

```
Pi 4 (V3D 4.2) dEQP-GLES3: every one of the 42,810 mustpass cases now has a result.

Commits (all on origin/main): b1b5b845 (the whole tree, 2026-11-23/25), 87881c78 (fbo, fragment_ops, fragment_out, draw, colour clear, texture T01-T17b and the seven aborted images that the tip frees, the ES 2.0 regression rows), 02a2ed23 (the S03a family and T14, T14a1, T14aa, T14aa1 with texture-2048.conf).
Records: tools/cts/log/2026-11-pi4-gles3-b1b5b845-*/, tools/cts/log/2026-11-pi4-gles3-87881c78-{fbo,tex1,tex2,aborts-a,es2}/, tools/cts/log/2026-11-pi4-gles3-02a2ed23-aborts-b/. Fails: docs/es3/pi4-gles3-fails.tsv; tree: tools/cts/pi4-gles3-tree.txt (151 images, 151 of 151 run).

Totals out of 42,810: 39,519 Pass / 1,399 Fail / 1,877 NotSupported / 15 other (4 QualityWarning, 11 ResourceError) / 0 without a result. At b1b5b845 it was 35,922 / 2,683 / 2,197 / 29 / 1,979 without a result. 0 GPU recoveries, 0 hung jobs, 0 MMU faults.

Refusals removed: the nine aborts (S03a, S07, T14a, T17a, TF1a, TF1b, TF2a, TF2b, TF2c) are gone. A5 (4096 targets, 6d2e7d0d), ETC2 sampling (455c3bb5), the job-builder input cap and -ENOTSUP for every refusal of legal state (6581f22b, b7c1484e) took them out: S07, T17a, TF1a, TF1b, TF2a, TF2b, TF2c pass all 871 cases; S03a has 1,087 Pass and 54 Fail of 1,141.
Refusals that remain (Fails, not aborts): the 3 draw targets (54 S03a mat4 link fails, 26 fragment_out.random); T14a's lost draw (-E2BIG) on rpi_4b.conf, fitted by texture-2048.conf (T14 family: 233 Pass, 9 Fail of 242); the six T09 size.2d.2048x2048 cases under rpi_4b.conf's tiled-copy budget; ASTC not advertised (T01, T15, T16, T17).
New defect issues are listed in the report: R/RG integer texture sampling (48), R8/RG8 blend read-back (2), msaa with max_samples 0 (42), three draw targets (26 + 54), 5 fragment_out.random seeds, the unexplained FB4 fails (275).
ES 2.0 regression rows (D, FB1, T1, T3, TF) at 87881c78: 1,184 of 1,184 Pass. The ES 3.0 gl_test cases that b1b5b845 left out have no record directory in the tree at this commit; they are not counted here.
```

### (e) new defect issues

1. **Title:** `gl: R and RG integer textures sampled through a colour fbo give wrong pixels (fbo.color.tex2d, tex2darray, tex3d, texcube; Pi 4, 48 cases)`
   Text: image FB3 at 87881c78, `fbo.color.{tex2d,tex2darray,tex3d,texcube}`: 12 cases each, 48 Fail ("Image comparison failed") of FB3's 50 Fail. They are the R and RG integer formats (R8I to RG32UI class). The fix is in `src/gl/draw.rs` (the GL core's sampling of R/RG integer textures); it is outside plan #399, which only names it. Records: `tools/cts/log/2026-11-pi4-gles3-87881c78-fbo/FB3.*`. Test: a host test of the sampled texel of an R8I and an RG32UI texture, then FB3 on the Pi 4.
2. **Title:** `gl: R8 and RG8 colour blend read-back differs (fbo.color.blend.r8_src_over, rg8_src_over; Pi 4, 2 cases)`
   Text: FB3 at 87881c78: `fbo.color.blend.r8_src_over` and `fbo.color.blend.rg8_src_over` Fail, the other 48 FB3 Fails being the issue above. The read-back of an R8 or RG8 colour is the GL core's. Record: `87881c78-fbo/FB3.*`.
3. **Title:** `v3d: GL_MAX_SAMPLES is 0, so fbo.msaa and its renderbuffer cases fail (Pi 4, 42 cases)`
   Text: FB4 at 87881c78: `fbo.msaa.{2,4,8}_samples.*` give 42 Fail; the back end's `max_samples` is 0 (`docs/notes/es3-msaa-renderbuffers.md`, `msaa-fbo` feature). Back end: the 4x framebuffer objects of `7f72cb46` are the start; the Pi 4 image must be built with `msaa-fbo` and rerun. Record: `87881c78-fbo/FB4.*`.
4. **Title:** `v3d: only 3 draw targets; ES 3.0 asks for 4 (fragment_out.random 26 cases, S03a precision mat4 54 cases; Pi 4)`
   Text: `DRAW_TARGETS` is 3 for `gpu-v3d` (`src/backend.rs`), so `GL_MAX_DRAW_BUFFERS` and `GL_MAX_COLOR_ATTACHMENTS` are 3, below ES 3.0's minimum of 4 (`docs/notes/v3d-refusals.md` G). FR2 at 87881c78 has 26 `fragment_out.random.*` link fails; S03a (02a2ed23: S03a4b, S03a5b, S03a7b) has 54 `precision.{matrixcompmult,outerproduct,transpose}` mat4, mat4x2 and mat4x3 link fails ("fragment output at location 3 ends at 4, beyond the 3 draw buffers"); A1's 2 Fails (`implementation_limits.max_color_attachments`, `max_draw_buffers`) are the same limit. A fourth target reaches fbo, fragment_out, draw_buffers and the 4096 memory budget (mrt4 design).
5. **Title:** `v3d: 5 fragment_out.random seeds fail for a cause other than the 3-target limit (FR2-b; Pi 4)`
   Text: FR2 at 87881c78: 31 Fail, all `fragment_out.random.N`; 26 are the 3-target limit (issue above), the other 5 are not explained by it. Name the seeds from the host run first (`fr2b-cause`), then the fix. Record: `87881c78-fbo/FR2.*`.
6. **Title:** `v3d: 275 fbo.render and fbo.invalidate Fails of Pi 4 image FB4 are unexplained`
   Text: FB4 at 87881c78: 317 Fail, 42 of them msaa (issue above). The other 275 are `fbo.render.{resize 94, shared_colorbuffer 84, recreate_color 69, shared_colorbuffer_clear 34, recreate_depth_stencil 11, shared_depth_stencil 10, stencil_clear 3}` and `fbo.invalidate.{whole, sub, format}` (4 each, including those that a wide store may move, `docs/notes/v3d-fbo-render.md`). Cause not found; first step is to keep the first message of each not-pass case (the board console), then a host reproduction. Records: `87881c78-fbo/FB4.*`.
7. **Title:** `v3d: the T09 size.2d.2048x2048 cases are refused under rpi_4b.conf's tiled-copy budget (Pi 4, 6 cases)`
   Text: T09 at 87881c78: six cases (`rgb888`, `rgba4444`, `rgba8888`, plain and `_mipmap`), a refused draw (Fail). The tree builds T09 with `texture-2048.conf`; that was not run by this plan. Run it and record. Record: `87881c78-tex2/`.
