<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Roadmap

YakoGL's work runs in five stages, in this order. Each stage has a
criterion that can be checked from the repository, and a stage is done when
its criterion is met and the record is committed. The state written under
each stage is read from what is committed: the `README.md` "Status" tables,
`docs/es2-cts-failures.md`, `docs/es3-cts-failures.md`, the board results in
`tools/cts/RESULTS.md`, `docs/release/checklist.md` and `CHANGELOG.md`.
Every number carries the date of its record. Known defects are tracked as
GitHub Issues. The states below were read on 2026-10-26, at main
`5ac5ac23c`.

The order is a priority, not a schedule. Board time and review go to the
lowest-numbered stage that is not done. Stage 4 runs on the host in parallel
with stages 1 to 3 and takes a board only when the earlier stages do not need
it. Stage 5 starts after stage 3.

The three boards of stages 1 and 2:

| Board | SoC | GPU |
| --- | --- | --- |
| Toradex Verdin AM62P | TI AM62P | Imagination PowerVR BXS-4-64 |
| Toradex Verdin iMX8M Plus | NXP i.MX 8M Plus | Vivante GC7000UL |
| Raspberry Pi 4 | Broadcom BCM2711 | Broadcom V3D 4.2 |

## 1. OpenGL ES 2.0: zero dEQP-GLES2 failures

**Criterion.** On each of the three boards, one run of the whole dEQP-GLES2
mustpass list (17,165 cases, `tools/cts/gles2-mustpass.txt`), built from one
main commit and run on the GPU, reads 0 Fail, 0 crashes and 0 cases without a
result. NotSupported is only the agreed set (34 cases, the same on every
board: `clip_control`, `compressedtexsubimage2d`, multisample at the 1x
config, `multisampled_render_to_texture.readpixels`; section C of
`docs/es2-cts-failures.md`). The `README.md` dEQP-GLES2 table shows those
numbers for every board, with the commit and the record they come from.

**State.** Met. The three boards ran the whole tree from one main commit,
`e9901869a` (2026-10-15/16, issue #870; `tools/cts/RESULTS.md`):

| Board | Pass | Fail | NotSupported | Warnings |
| --- | ---: | ---: | ---: | ---: |
| AM62P (1x tree with the 16 `multisample.*` cases of the 4x image) | 17,140 | 0 | 18 | 7 |
| i.MX8MP | 17,318 of 17,371 begun | 0 | 34 | 19 |
| Raspberry Pi 4 | 17,126 | 0 | 34 | 5 |

Later runs from newer main commits read the same counts: the AM62P 1x images
at `25cce714` (2026-10-25), the i.MX8MP at `d0c5dc48` (2026-10-23) and the
Raspberry Pi 4 at `46f93767` (2026-10-24). The AM62P's 4x image was not run
again.

## 2. Benchmarks on the three boards

**Criterion.** On each board, booted from its own storage with no host:
glmark2-es2 at 800x600 runs all 33 scenes with no failed scene, no GL error
and no scene that runs out of GPU memory, every scene's picture has been
checked, and a frame rate is recorded for each scene. The `README.md` glmark2
table holds one score per board from one main commit under those conditions.
The demos (the Slint sample, `examples/slint/`) are kept working
on every board, but they are not tests or benchmarks: no criterion of this
roadmap is measured with them.

**State.** glmark2-es2 runs all 33 scenes on every board with no failed
scene and no GL error (`tools/cts/RESULTS.md`, "Common set"). The `README.md`
table holds one score per board from main `445c5653` (2026-10-20): AM62P 461,
Raspberry Pi 4 193, i.MX8MP 42. The Pi 4 has since read 195 at `c4af4917`
(2026-10-22). A frame time and a picture value are recorded for every scene
on every board (`tools/cts/glmark2-pictures.txt`), and no scene is black.

Left for the criterion:

- Pictures by eye: only the AM62P's were looked at on its panel (at
  `bc43ea09`; 29 of 33 scenes match the current run, the other 4 are animated).
  The Pi 4 and i.MX8MP pictures are checked by value only (the i.MX8MP:
  #1207).
- One main commit for the three scores again, with terrain and refract
  checked (#1196).

Slow scenes, which the criterion does not judge: on the AM62P terrain,
shadow and refract take 94, 353 and 376 ms a frame, mostly GL-call time on
the CPU (#1199, #1006, #1198); on the Pi 4 terrain takes 162 ms (#1209); on
the i.MX8MP terrain, shadow and refract run at 4, 2 and 2 FPS (shadow and
refract: #1204), and the other scenes wait for the panel's VSYNC every frame
(`docs/notes/imx8mp-frame-floor.md`). The gap list is
`docs/notes/stage2-bench-gap.md`.

## 3. Public release

**Criterion.** The repository is public and every line of
`docs/release/checklist.md` is met: a current `CHANGELOG.md`, `README.md`
Status tables equal to the latest records, `CONTRIBUTING.md` and
`SECURITY.md` that a new reader can follow, a licence and SPDX audit with no
open finding, a redaction audit of the tree and its history with no finding,
the website consistent with the status sources, this roadmap, and a green
`tools/check.sh` on the tree to publish. A first version is tagged with its
own `CHANGELOG.md` section.

**State.** Not public yet. On 2026-10-21 the owner decided how
(issue #1606): a new public repository whose history starts with one fresh
commit of the cleaned tree, and the order of the work. Stage 1, which this
stage follows, is met. `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`,
`DESIGN.md`, this roadmap and the release building guide are in the tree;
`CHANGELOG.md` has no version section and no version is tagged yet (#1606,
step 8). The state of each checklist line is in `docs/release/checklist.md`.
Still open there: the licence audit's undecided files, the redaction scan of
the snapshot commit, the website's numbers, the README glmark2 table from one
commit (#1196), and a green full `tools/check.sh` on the snapshot.

## 4. OpenGL ES 3.0

**Criterion.** On each of the three boards, one run of the whole dEQP-GLES3
mustpass list (42,810 cases, `tools/cts/gles3-mustpass.txt`) from one main
commit reads 0 Fail and 0 cases without a result, while the stage-1 ES 2.0
result is unchanged. A product image keeps reporting ES 2.0 by default; ES
3.0 is what an image that turns that off gets.

**State.** Host work runs ahead of the boards: transform feedback, uniform
blocks, sized internal formats, occlusion queries, `glBlitFramebuffer`,
sRGB, buffer mapping and pixel buffers are built and tested on the host for
the three GPUs, and the compiler's GLSL ES 3.00 coverage is measured over
the dEQP-GLES3 shader corpus (`docs/es3-cts-failures.md`, "Feature state").
On the boards (`tools/cts/RESULTS.md`, "ES 3.0"):

- AM62P: the whole tree at `ad2b5ed` (2026-10-14) read 36,552 pass /
  2,880 fail / 2,195 NotSupported / 998 without a result. 987 of the 998 were
  one image that stopped on a `ResourceError`; that case passes since
  `ffee428a` (on the board at `be84094b`, 2026-10-25), and the tree is now
  31 images, 5 of them run since. Images 20 and 21 hang the GPU in
  `texture.filtering.3d` (#1285).
- Raspberry Pi 4: 9 of its 91 images at `925dbca9` (2026-10-22): 4,917
  cases, 3,943 pass / 643 fail / 331 NotSupported, 0 without a result (#1261).
- i.MX8MP: not run on the board yet (#1262).

The per-cause inventory is `docs/es3-cts-failures.md`.

## 5. More boards

**Criterion.** A new board joins when it meets stages 1 and 2 as the first
three do: a GPU back end selected by one Cargo feature, a standalone boot
from its own storage (`docs/standalone-boot.md`), one whole dEQP-GLES2 tree
with 0 Fail, and the three benchmarks running, each recorded the same way.

**State.** No board has joined. A port to the FriendlyElec NanoPC-T6
(Rockchip RK3588, Arm Mali-G610) runs outside the five stages (issue #20,
`driver/docs/nanopc-t6-mali.md`). Its back end (`gpu-mali`) runs on the
board's own Linux through the panthor kernel driver: the newest `gl_test`
sweep (2026-10-24) reads 98 of 256 cases PASS, 0 FAIL, 63 refused by name,
87 with no case and 8 skipped (`tools/cts/RESULTS.md`, "Mali"). Booting it under
Zephyr is milestones Z1 to Z4 (#1281).
