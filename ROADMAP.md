<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Roadmap

YakoGL's work runs in five stages, in this order. Each stage has a
criterion that can be checked from the repository, and a stage is done when
its criterion is met and the record is committed. The state written under
each stage is read from what is committed: the `README.md` "Status" tables,
`docs/es2-cts-failures.md`, `docs/es3-cts-failures.md`, the board records
under `docs/board-sessions/`, `docs/release/checklist.md` and `CHANGELOG.md`.
Every number carries the date of its record. Known defects are tracked as
GitHub Issues.

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

## 1. OpenGL ES 2.0 conformance

**Criterion.** On each of the three boards, one run of the whole dEQP-GLES2
mustpass list (17,165 cases, `tools/cts/gles2-mustpass.txt`), built from one
main commit and run on the GPU, reads 0 Fail, 0 crashes and 0 cases without a
result. NotSupported is only the agreed set (34 cases, the same on every
board: `clip_control`, `compressedtexsubimage2d`, multisample at the 1x
config, `multisampled_render_to_texture.readpixels`; section C of
`docs/es2-cts-failures.md`). The `README.md` dEQP-GLES2 table shows those
numbers for every board, with the commit and the record they come from.

**State.** Every board has recorded one such run from a single main commit
(2026-10-12; `docs/board-sessions/am62p-es2-final-r2.md`,
`imx8mp-es2-final-r2.md`, `pi4-es2-final-r2.md`):

| Board | Pass | Fail | NotSupported | Warnings |
| --- | ---: | ---: | ---: | ---: |
| AM62P (1x tree; the 4x multisample image reads 16 of 16) | 17,125 | 0 | 34 | 6 |
| i.MX8MP | 17,318 of 17,371 begun | 0 | 34 | 19 |
| Raspberry Pi 4 | 17,127 | 0 | 34 | 4 |

The `README.md` table does not carry those records yet: it shows the AM62P
tree of 2026-10-10 (54 failures, all fixed since, as the table says) and a
later i.MX8MP tree (2026-10-14) that regressed to 46 failures, every one a
`GL_BGRA_EXT` renderbuffer case in `fbo`. The stage closes when that
regression is fixed and confirmed on the i.MX8MP and the README table reads
0 Fail on all three boards from the records above or newer ones.

## 2. Benchmarks on the three boards

**Criterion.** On each board, booted from its own storage with no host:
glmark2-es2 at 800x600 runs all 33 scenes with no failed scene, no GL error
and no scene that runs out of GPU memory, and every scene's picture has been
checked; the Slint sample (`driver/samples/zephyr/slint/`) and the AGL
cluster demo (`driver/samples/zephyr/cluster/`) run with the same picture on
every board, and a frame rate is recorded for each. The `README.md` glmark2
table holds one score per board from one main commit under those conditions.

**State.** glmark2-es2 runs all 33 scenes on every board (AM62P 465,
Raspberry Pi 4 183, i.MX8MP 43; the `README.md` table with its dates). Two
scenes drew wrong, terrain on every board and refract on the AM62P and the
Pi 4; both are fixed on the host, and no picture of either has been checked
on a board yet. On the AM62P terrain and shadow run out of GPU memory and
refract reports incomplete framebuffers; the i.MX8MP's score is probably
capped by a wait for the panel's VSYNC every frame. Slint runs on all three
boards from one sample with the same picture, and the AGL cluster demo is
built, run and tallied on every board as part of the common benchmark set
(`CHANGELOG.md`). Performance work has not started.

## 3. Public release

**Criterion.** The repository is public and every line of
`docs/release/checklist.md` is met: a current `CHANGELOG.md`, `README.md`
Status tables equal to the latest records, `CONTRIBUTING.md` and
`SECURITY.md` that a new reader can follow, a licence and SPDX audit with no
open finding, a redaction audit of the tree and its history with no finding,
the website consistent with the status sources, this roadmap, and a green
`tools/check.sh` on the tree to publish. A first version is tagged with its
own `CHANGELOG.md` section.

**State.** `CHANGELOG.md` has only an `[Unreleased]` section; no version is
tagged. `CONTRIBUTING.md`, `SECURITY.md`, `DESIGN.md` and the release
building guide are in the tree. The checklist, read at its 2026-10-12 base,
marks the licence audit, the redaction audit, the website's numbers and the
README walkthrough findings as open or pending, and leaves the security
contact and the licence of vendored code to the owner. Its lines on the
CHANGELOG, the README tables and the website depend on stage 1.

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
the dEQP-GLES3 shader corpus (`docs/HANDOFF.md`, "ES 3.0"). On the board,
the AM62P's whole tree read 36,552 pass / 2,880 fail / 2,195 NotSupported /
998 without a result (2026-10-14, `docs/HANDOFF.md`); the per-cause
inventory is `docs/es3-cts-failures.md`. The Raspberry Pi 4 ran a first
baseline of 7 of its 91 images (2,798 pass / 744 fail, 2026-10-14,
`docs/board-sessions/pi4-gles3-baseline.md`); the i.MX8MP has an image table
that has not run on the board yet.

## 5. More boards

**Criterion.** A new board joins when it meets stages 1 and 2 as the first
three do: a GPU back end selected by one Cargo feature, a standalone boot
from its own storage (`docs/standalone-boot.md`), one whole dEQP-GLES2 tree
with 0 Fail, and the three benchmarks running, each recorded the same way.

**State.** Not started. No board is chosen; the choice comes after stage 3.
