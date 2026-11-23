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
GitHub Issues. The states of stages 3 and 5 were read on 2026-11-16, at main
`e6e68ff7`; those of stages 1, 2 and 4 were read on 2026-11-05, at main
`08163414`, and have not been re-read since.

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
`e9901869a` (2026-10-15/16, issue #870), and every newer run reads 0 Fail.
The newest whole tree of each board (`tools/cts/RESULTS.md`, "ES 2.0"):

| Board | Pass | Fail | NotSupported | Warnings |
| --- | ---: | ---: | ---: | ---: |
| AM62P (1x run of `36c4305a`, 2026-11-05, plus the 16 `multisample.*` cases of the 4x image from `e9901869a`) | 17,142 | 0 | 18 | 5 |
| i.MX8MP (`0fbe61a3`, 2026-10-31; 17,371 cases begun, the list plus 206 outside it) | 17,318 | 0 | 34 | 19 |
| Raspberry Pi 4 (`46f93767`, 2026-10-24) | 17,126 | 0 | 34 | 5 |

The AM62P's 1x run alone reads 17,126 pass on the 17,165-case list. The 34
NotSupported are the agreed set; the AM62P runs its 16 `multisample.*`
cases on the 4x image, which was not run again after `e9901869a`.

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
  #17).
- One main commit for the three scores again, with terrain and refract
  checked (#1196).

Slow scenes, which the criterion does not judge: on the AM62P terrain,
shadow and refract take 94, 353 and 376 ms a frame, mostly GL-call time on
the CPU (#1199, #1006, #1198); on the Pi 4 terrain takes 162 ms (#1209); on
the i.MX8MP terrain, shadow and refract run at 4, 2 and 2 FPS (shadow and
refract: #15), and the other scenes wait for the panel's VSYNC every frame
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

**State.** Not public yet (read 2026-11-23, main `83c4a779`). On 2026-10-21
the owner decided how (issue #1606): a new public repository whose history
starts with one fresh commit of the cleaned tree, and the order of the work.
Stage 1, which this stage follows, is met. `CHANGELOG.md` has its `0.1.0`
section (dated 2026-10-27, commit `625f82ce`); no version is tagged.
`CONTRIBUTING.md`, `SECURITY.md`, `DESIGN.md`, this roadmap and the release
building guide are in the tree. A rehearsal of the snapshot commit ran on
2026-11-23 on the real commit `da11eb6b` (`docs/release/snapshot.md`,
`45d5b03e`): the tool ran twice and printed the same snapshot hash
(deterministic), the output is one root commit with no remote, nothing was
pushed, and the accidental `before` gitlink that had blocked the tool is
out of the tree (`da11eb6b`). The state of each checklist line is in
`docs/release/checklist.md`. Still open:

- The licence audit is clean with 0 findings (checklist line 7), and
  `undecided` is now a `--fail-on` class of check 25 of
  `tools/cts/cts-selftest.sh`; the rehearsal's audit on the snapshot of
  `da11eb6b` exits 0 with it: 15250 files, 5867 with an SPDX line, 7830
  allowlisted, 0 in every fail-on class, findings 0
  (`docs/release/snapshot.md`, 2026-11-23).
- The redaction scan (line 8): Part 4 of `docs/release/redaction-audit.md`
  was re-run on 2026-11-23 at `937161af5beb` (`eb83cbd5`), 15086 files.
  The public run exits 3 with "0 H/M finding(s), 3 class(es) not run, 115
  informational, 145 look-alike(s) dropped: NOT COMPLETE (private classes
  not run)" — exit 3 is not a pass; the scan's self-test prints
  "self-test passed" and `tests/redaction_scan.rs` passes 4. The private
  run, with a partial value set held in one process's memory only (never
  in a file, a log or a commit), exits 0 with 0 H/M findings: clean.
  Nothing needed rewording: the `lab-host` findings of the earlier private
  runs are gone from the tree (the X2 slow-scene notes no longer name the
  lab's machines, `49f3a112`). The rehearsal's scan on the snapshot commit
  of `da11eb6b` reads the same — public exit 3 with 0 H/M findings,
  private exit 0 with a partial set; a broader private set holding the
  public product and tool names read exit 1 with 165 H/M, all of them
  those public names, and whether any of them must leave the tree is the
  owner's call. Still open for line 8: the owner's run with
  `YAKOGL_REDACTION_PRIVATE_FILE` (the full value set) on the real
  snapshot commit, exit 0, and the history decisions D7/D8 (#1606 step 8).
- The website (line 9): `bash tools/website-check.sh` exits 0 at this tip:
  "website-check: ok (5 pages, 168 links and sources, 19 figures)".
  `website/data.js` is checked against `3a45f7a8` (2026-11-23, `cfe028aa`):
  its dEQP tables and the Mali and Pi 5 lines, and the NanoPC-T6 status
  line of `status.html`, follow `tools/cts/RESULTS.md`, which the check
  verifies against. The glmark2 figures wait for #1196.
- The README glmark2 table from one commit (#1196): it still mixes the
  2026-10-20 run with the AM62P's `86d0b482` (2026-10-28).
- The `CONTRIBUTING.md` walk (line 3): met on 2026-11-12, fresh clone of
  `570cd8c6`, `--quick` 181 ok, 0 failed, 0 gap(s).
- A green full `tools/check.sh` on the snapshot (line 12; its re-run is
  #344's): line 12 was met at `c66bea9f` (2026-11-11, full gate 222 ok, 0
  failed, 0 gap(s), recorded in `4946e879`), which holds for that commit
  only. The nightly `rust_eh_personality` link failure is fixed
  (`fce3e6aa`, `2e949ed5`). Since `760aad3d` (2026-11-14, `cde1db8d`) the
  line is partial: the full run failed 9 rows on `tests/redaction_scan.rs`
  (fixture values of `tools/mp2_uboot_record_check.py`, fixed in
  `31608f8e`) and was cut by a full disk, so the gate is re-run on the
  snapshot.

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
- Raspberry Pi 4: the whole tree (114 rows, the 91 images re-split where dEQP
  aborted) at `b1b5b845` (2026-11-23): 42,810 cases, 35,922 pass / 2,683 fail /
  2,197 NotSupported / 29 other / 1,979 without a result, 0 GPU recoveries
  (#1261). The 1,979 are cases that never began in nine images dEQP aborts on a
  refused draw (S03a, S07, T14a, T17a, TF1a, TF1b, TF2a, TF2b, TF2c); they
  still need one-case images, and the images plan #184's V3D work changes need
  a rerun.
- i.MX8MP: not run on the board yet (#24); the recipe
  (`docs/es3/board-recipe-imx8mp.md`) and the preflight
  (`tools/cts/imx8mp-gles3-preflight.sh`) are in the tree.

The per-cause inventory is `docs/es3-cts-failures.md`.

## 5. More boards

**Criterion.** A new board joins when it meets stages 1 and 2 as the first
three do: a GPU back end selected by one Cargo feature, a standalone boot
from its own storage (`docs/standalone-boot.md`), one whole dEQP-GLES2 tree
with 0 Fail, and the three benchmarks running, each recorded the same way.

**State.** No board has joined (read 2026-11-16, main `e6e68ff7`). A port
to the FriendlyElec NanoPC-T6 (Rockchip RK3588, Arm Mali-G610) runs outside
the five stages (issue #7, `driver/docs/nanopc-t6-mali.md`). Its back end
(`gpu-mali`) runs on the board's own Linux through the panthor kernel
driver. Per `tools/cts/RESULTS.md`, "Mali":

- dEQP-GLES2: the whole 17,165-case list at main `8ff01055` (2026-11-24, one
  boot, 76 rows) reads 10,950 Pass, 5,706 Fail, 506 NotSupported and 3
  QualityWarning, with no crash, hang or kernel line; 5,829 of the non-Pass
  cases are refusals of the online compiler (issue #11251,
  `docs/es2-cts-failures.md` section M). Stage 5 needs 0 Fail.
- `gl_test` sweep (`tools/cts/log/2026-11-t6-sweep-1a22bd38/`, main
  `1a22bd38e`, 2026-11-12): 165 of 256 cases PASS twice, 6 refused, 73 with
  no case, 9 skipped, 3 FAIL, 0 FAULT.
- glmark2-es2 (`08ae7a8e`, main `da65355e`, 2026-11-10; `4bf0e7ab`, main
  `a57d8104`, 2026-11-11, the same 33 outcomes): no score, 1 of 33 scenes
  clean (desktop shadow, picture match), 31 refused at `glCompileShader`
  (#33).
- Under Zephyr (by kexec from Armbian, a development result; U-Boot
  starting the image has not run): Z1 to Z4 PASS (2026-11-11/12) and
  `gl_test`'s regression set 50 of 50 at main `9ad07f55f`, 2026-11-16 02:54
  JST (`c8b6f065`).

Raspberry Pi 5 (BCM2712 D0, V3D 7.1.10; issue #7): the bring-up
under Zephyr, booted by the Pi firmware's tryboot through `tools/board.sh
pi5` (`tools/cts/RESULTS.md`, "V3D 7.1: Raspberry Pi 5 on Zephyr"). On
2026-11-19 the PM watchdog reset the board and tryboot brought Linux back;
D1 (`rpi5_dtb`) read the start state (loaded at 0x200000, EL2, x0 = the
DTB); D2 (`rpi5_ident`) read the V3D identity with no setup, equal to
Linux's debugfs (`tools/v3d/pi5/`); and the first clear (M1 to M4,
`rpi5_clear`, build `8eb2b3cf`) printed `v3d_clear: PASS`, 0 of 8,192
pixels wrong. The first triangle (M5), `gl_test`, dEQP-GLES2, the display
and the benchmarks have not run on the board.

STM32MP257F-DK (STM32MP257F, 2x Cortex-A35, Vivante GCNANOULTRA31_VIP2,
model 0x8000 revision 0x6205; issues #7 and #1281): the bring-up under
Zephyr, our image started by the board's own U-Boot off the microSD as a
one-shot through `tools/board-stm32mp2.sh`; steps 1 to 4 are recorded in
commit `226cd9a2` and `docs/notes/stm32mp257f-dk-bringup.md` ("Board
verdicts"). On 2026-11-21 the return trial printed its banner and the board
came back to ST's Linux by itself: `verdict=done rc=0` after a PSCI
SYSTEM_RESET (47 s, record `20261121-1054-mp2_hello-405d515b`), and
`verdict=watchdog_reset rc=0` after a hang caught by OP-TEE's watchdog (89
s, `20261121-1055-mp2_hello_hang-832dda7e`). U-Boot hands over at EL2 with
the MMU and both caches off, x0 = 1, x3 = the load address, CNTFRQ 40 MHz,
PSCI 1.1. `mp2_gpu` (`20261121-1107-mp2_gpu-9e520020`) then powered the GPU
over SCMI through OP-TEE plus the RCC gate and read model 0x8000, revision
0x6205, product 0x80003, eco 0, customer 0x15; `HI_INTR_ENBL` read back
0xffffffff and the reset reached idle. On 2026-11-21 `gc_fence` passed
(`20261121-1855-gc_fence-55ae8385`: `gc_fence: fence 1 irqs 1 elapsed 4169 us
irq-latency 21 us`, SPI 183 routed, board back after 58 s) and `gc_clear`
passed (`20261121-1859-gc_clear-e1a37b77`: `gc_clear: 256x128 wrong pixels 0
(clear 1 0, clear 2 0, guard 0)`, back after 45 s). On 2026-11-23
`gc_triangle` passed (`20261123-0451-gc_triangle-250f61d4`) and `gl_test`
case 1 passed (`20261123-0454-gl_test_case1-ed1e6361`; that run printed no
`gl_test: all:` tally). Steps 5 to 8 are recorded in the same note's "Board
verdicts" (commits `83c00813` and `cc0fccf2`). The full `gl_test` set, dEQP,
the display, the direct boot and ES 3.0 and 3.1 have not run on the board.

The other candidate boards (#321, #327, #332, #336, #341) are issues only;
the tree holds no board record for them.
