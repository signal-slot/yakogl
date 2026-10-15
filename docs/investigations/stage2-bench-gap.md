<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Stage 2 (benchmarks): what is measured and what the criterion still needs

Audit of 2026-10-15 at main `548860df`. Host only; nothing was run on a board.
Every number below is quoted from a committed record; the raw records
(`records/<board>/...`) are gitignored and were not read.

## The criterion (ROADMAP.md, "2. Benchmarks on the three boards")

On each board, booted from its own storage with no host: glmark2-es2 at
800x600 runs all 33 scenes with no failed scene, no GL error and no scene out
of GPU memory, **and every scene's picture has been checked**; the Slint sample
and the AGL cluster demo run **with the same picture on every board**, and
**a frame rate is recorded for each**. The README glmark2 table holds one score
per board from one main commit under those conditions.

## Answer: are glmark2 pictures checked per scene today?

**No.** Nothing in the repository compares a glmark2 frame with anything.

- The glmark2 image (`driver/samples/zephyr/glmark2/src/main.cpp`,
  `platform.c`) holds no read-back, CRC or reference: it prints `FPS:` lines,
  `glmark2-frame:` / `glmark2-mem:` lines, and a GLMARK2 record of counts
  (score, scenes ran / failed / unsupported, GL errors raised, draws that
  sampled an incomplete texture).
- `tools/cts/common-tally.py` reads that record and the console. Its only
  per-scene judgement is `glmark2_scenes()` / `glmark2_cell()`
  (`tools/cts/common-tally.py:216-299`): a scene is amber when a line of
  `GLMARK2_BAD_LINES` (a GL error, an incomplete texture, a memory refusal;
  line 88) falls inside it, green otherwise. That is a **log-line check, not a
  picture check**: a scene that draws black with no error line is green.
- The only picture checks in the common set are the Slint and cluster
  **probes** (a `PROBES` record: five pixels and a crc16 of RGB565 samples,
  `slint probes pass ... crc16`). Nothing like it exists for glmark2.
- `README.md` and ROADMAP.md say so: "no picture of either [terrain, refract]
  has been checked yet". The Pi 4 session note lists "terrain not black" as
  "not shown by the record ... nobody looked at the panel"
  (`docs/board-sessions/pi4-common-int.md`, "The expected lines").
- The i.MX8MP has no console capture: its glmark2 evidence is the SNVS record
  (counts only), so not even the log-line check or an fps line exists
  (`docs/board-sessions/imx8mp-common-int.md`, "glmark2 (GM)").

## Records the table rests on (newest per board)

| Board | Record | Commit | Notes |
|---|---|---|---|
| am62p | `docs/board-sessions/am62p-common-int.md` (2026-10-14; `records/am62p/common-cd738b1a`) | `cd738b1a` (+ docs commit `d5cb878`) | GM, SO, CL, GT ran; **SV returned 0 console bytes**. Per-scene fps present (console). Earlier GM: `tools/cts/RESULTS.md` "AM62P: glmark2 (GM image) on `f2a13877`" (score 465, 33 scenes). Commit 504d4fae (256 MiB carve-out) has not been run on a board. |
| pi4 | `docs/board-sessions/pi4-common-int.md` (2026-10-14; `d64e995b`, the code of `79c73ea2`) | `79c73ea2` | all five rows done, resets 0. The note quotes fps for terrain, refract and shadow only; the other per-scene lines are in the gitignored console. |
| imx8mp | `docs/board-sessions/imx8mp-common-int.md` (`b568940`) and `tools/cts/RESULTS.md` "i.MX8MP: common set on main a63fcc5b" (2026-10-14) | `a63fcc5b` | no console: counts and probes only; score 43. |

## Board x benchmark table

Cell form `ran / fps`. "ran" is the record's verdict; for pi4 and imx8mp the
record counts only (33 ran, 0 failed), so a scene is "ran" but not named
individually. `fps` is the per-scene frame rate where a committed file gives
it, `none` otherwise.

Columns that are the same for every glmark2 row, stated once instead of 99
times:

- *Picture checked*: **no**, on all three boards and all 33 scenes (no code
  path; see the answer above). The only evidence is "no GL-error /
  incomplete-texture / memory-refusal line" (AM62P and Pi 4 consoles) or
  "failed 0" (i.MX8MP record).
- *Same picture on every board*: **not established** for any scene; no frame
  was compared across boards. Pictures known wrong earlier: terrain (black
  texture, all boards) and refract (AM62P, Pi 4); both fixed on the host,
  unverified on a board.
- *Rests on*: AM62P `docs/board-sessions/am62p-common-int.md` (`cd738b1a`);
  Pi 4 `docs/board-sessions/pi4-common-int.md` (`79c73ea2`); i.MX8MP
  `docs/board-sessions/imx8mp-common-int.md` and `tools/cts/RESULTS.md`
  (`a63fcc5b`).

| # | scene (options) | am62p | pi4 | imx8mp |
|---|---|---|---|---|
| 1 | build use-vbo=false | ran / 102 | ran / none | ran / none |
| 2 | build use-vbo=true | ran / 879 | ran / none | ran / none |
| 3 | texture nearest | ran / 939 | ran / none | ran / none |
| 4 | texture linear | ran / 935 | ran / none | ran / none |
| 5 | texture mipmap | ran / 920 | ran / none | ran / none |
| 6 | shading gouraud | ran / 705 | ran / none | ran / none |
| 7 | shading blinn-phong-inf | ran / 606 | ran / none | ran / none |
| 8 | shading phong | ran / 485 | ran / none | ran / none |
| 9 | shading cel | ran / 414 | ran / none | ran / none |
| 10 | bump high-poly | ran / 329 | ran / none | ran / none |
| 11 | bump normals | ran / 845 | ran / none | ran / none |
| 12 | bump height | ran / 710 | ran / none | ran / none |
| 13 | effect2d kernel 0,1,0 | ran / 173 | ran / none | ran / none |
| 14 | effect2d kernel 1,1,1,1,1 | ran / 112 | ran / none | ran / none |
| 15 | pulsar | ran / 889 | ran / none | ran / none |
| 16 | desktop blur | ran / 47 | ran / none | ran / none |
| 17 | desktop shadow | ran / 145 | ran / none | ran / none |
| 18 | buffer map | ran / 62 | ran / none | ran / none |
| 19 | buffer subdata | ran / 62 | ran / none | ran / none |
| 20 | buffer interleave map | ran / 69 | ran / none | ran / none |
| 21 | ideas | ran / 180 | ran / none | ran / none |
| 22 | jellyfish | ran / 158 | ran / none | ran / none |
| 23 | terrain | ran, **out of GPU memory**, FB incomplete every frame / 1 | ran, no refusal / 3 | ran / none |
| 24 | shadow | ran, **out of GPU memory** / 3 | ran / 5 | ran / none |
| 25 | refract | ran, **1694 incomplete-framebuffer errors** / 85 | ran, no refusal / 2 | ran / none |
| 26 | conditionals frag 0 vtx 0 | ran / 951 | ran / none | ran / none |
| 27 | conditionals frag 5 vtx 0 | ran / 546 | ran / none | ran / none |
| 28 | conditionals frag 0 vtx 5 | ran / 952 | ran / none | ran / none |
| 29 | function low | ran / 788 | ran / none | ran / none |
| 30 | function medium | ran / 559 | ran / none | ran / none |
| 31 | loop fragment-loop=false | ran / 741 | ran / none | ran / none |
| 32 | loop fragment-uniform=false | ran / 741 | ran / none | ran / none |
| 33 | loop fragment-uniform=true | ran / 258 | ran / none | ran / none |
| | **score** | 465 (`cd738b1a`, also `f2a13877`) | 183 (`79c73ea2`) | 43 (`a63fcc5b`, `cd738b1a`) |
| | GL errors | 2573 raised | 0 | "15 or more" (counter saturates) |

The AM62P fps are the console lines of the `cd738b1a` run at the 64 MiB
carve-out (before 504d4fae).

### SO, SV, cluster

| Benchmark | am62p | pi4 | imx8mp |
|---|---|---|---|
| **SO** slint view off | ran; probes pass, crc16 c592; picture checked by the probes (5 pixels + crc16, tolerance 1 per channel); equal to imx8mp (`08420862 00210842 c5920883`), Pi 4 one RGB565 step off; fps **none**. `am62p-common-int.md`, `cd738b1a` | ran; probes pass, crc16 312b (was c592; p2 0862 not 0842; window fingerprint `0xe7113010`, was `0xfaa46995`; cause unbisected); fps none. `pi4-common-int.md`, `79c73ea2` | ran; probes pass, crc16 c592; fps none. `RESULTS.md` "i.MX8MP: common set on main a63fcc5b" |
| **SV** slint view on | **no result**: 0 console bytes, `verdict=empty`, not repeated (the AM62P board layer has no scan-out view; the SV image is the SO image, so this row is the copy path again). `am62p-common-int.md`, `cd738b1a` | ran; probes pass, crc16 312b, identical to SO; "the window is a view into the framebuffer (no copy)"; fps none. `pi4-common-int.md` | ran; probes pass, crc16 c592, identical to SO; fps none. `RESULTS.md` (`a63fcc5b`) |
| **cluster** | ran; probes pass, crc16 110c; **1024x600, 75.3 fps, 46.3 Mpx/s**, draw 13.1 ms mean. `am62p-common-int.md`, `cd738b1a` | ran; probes pass, crc16 7b93; **1920x720, 139.3 fps, 192.6 Mpx/s**, draw 5.5 ms; fingerprint `0xfdf7d08f` equal to the 8abeaabb baseline. `pi4-common-int.md`, `79c73ea2` | ran; probes pass, crc16 b107; **fps none** (no console). `RESULTS.md` (`a63fcc5b`) |

"Same picture on every board" for the cluster is not met by construction: the
panels differ (1024x600, 1920x720, 1280x800), the probes sample different
pixels, and the crc16 values (110c, 7b93, b107) are not comparable. The only
cross-board picture comparison is Slint SO: AM62P and i.MX8MP equal, Pi 4 one
step off. A frame rate is recorded for the cluster on two boards of three and
for Slint on none.

## Gap against the criterion

| Criterion part | am62p | pi4 | imx8mp |
|---|---|---|---|
| 33 scenes run, none failed | yes | yes | yes (counts) |
| no GL error, no scene out of GPU memory | **no** at 64 MiB (terrain, shadow, refract; 2573 errors); 504d4fae unmeasured | yes (0 errors) | unknown (counter saturated; no console) |
| every scene's picture checked | **no** | **no** | **no** |
| same picture on every board | **no** | **no** | **no** |
| frame rate, glmark2 | per scene | score and 3 scenes | score only |
| frame rate, Slint | none | none | none |
| frame rate, cluster | yes | yes | **none** |
| SV ran | **no** (0 bytes) | yes | yes |
| README table from one main commit | scores come from three different commits (`f2a13877`, 2026-10-04, `cd738b1a`) | | |

## Work that closes stage 2

1. **Add a per-scene picture value to the glmark2 image.** Read each scene's
   last frame back (a set of probe pixels or a crc over the 800x600 window) in
   `driver/samples/zephyr/glmark2/src/main.cpp` and print it in the
   `glmark2-frame:` line, so the picture exists as data on every board.
2. **Make `tools/cts/common-tally.py` judge pictures.** Beside
   `glmark2_scenes()` / `GLMARK2_BAD_LINES`, compare each scene's value with a
   committed per-scene reference table (a new file next to
   `tools/cts/common-tree.txt`, with a tolerance like the Slint probes) and
   turn a mismatch or an all-black scene into an amber or failed cell; extend
   `tools/cts/cts-selftest.sh` and the fixtures in `tools/cts/testdata/`
   (`glmark2-clean.txt`, `glmark2-terrain-0x502.txt`).
3. **Make the i.MX8MP's glmark2 evidence per scene.** It has no console: carry
   the per-scene picture values and fps in the SNVS record (`record.rs`,
   decoded by `tools/board.sh`) or give the board a console path. Without
   this, items 1-2 cannot cover the third board and no per-scene fps exists.
4. **Record frame rates for Slint, for every Pi 4 scene and for the i.MX8MP
   cluster.** Print the per-scene fps table in `common-tally.py` output for
   the Pi 4, add a frame rate to the Slint sample's output
   (`driver/samples/zephyr/slint/`), and add the cluster's fps to the i.MX8MP
   record.
5. **Measure the AM62P at the 256 MiB carve-out** (504d4fae) with
   `tools/cts/cts-common.sh am62p build|run`: terrain and shadow must stop
   running out of memory, refract must lose its incomplete-framebuffer lines,
   the GL error count (2573) must fall to 0. This is plan #149 item
   `am62p-common-carveout`.
6. **Repeat AM62P SV** until it returns a record (0 bytes at `cd738b1a`), or
   decide that SV is not a row on the AM62P because its board layer has no
   scan-out view (`tools/cts/common-tree.txt`: "the row is the copy path
   again").
7. **Check terrain and refract on a board.** Both are "fixed on the host" and
   no picture has been checked; item 1 supplies the check; run both on all
   three boards and put the result in `README.md`.
8. **Settle the Slint picture across boards.** Decide whether the Pi 4's
   p2 0862 / crc16 312b is the new reference (the Pi 4 session note says the
   reference fingerprint then needs updating) or a regression (candidates
   npot, femtovg-pin; unbisected), then compare the same pixels on all three
   boards.
9. **Define "same picture" for the cluster.** The panels differ in size:
   either render at a common size or compare per-panel probes against a
   resolution-independent reference; implement in `common-tally.py` and the
   cluster sample's PROBES (`driver/samples/zephyr/cluster/`).
10. **One main commit for the README glmark2 table.** After 1-9, run the five
    common rows on all three boards at the same main commit
    (`tools/cts/cts-common.sh <board> build|run`) and update the `README.md`
    glmark2 table from those records.
