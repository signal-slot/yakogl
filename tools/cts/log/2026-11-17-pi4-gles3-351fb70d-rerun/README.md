<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rerun map at 351fb70d: which b1b5b845 images main has changed (host only)

commit: 351fb70d992eaf9f8a13541061b5ffc597a7ecce

Plan #380, step "the Pi 4's remaining GLES3 images" (issue #1261), host step. For
each of the 100 images of `tools/cts/pi4-gles3-tree.txt` (42,810 mustpass cases,
`check-gles3-tree.py` unchanged), this record compares the image's board result at
`b1b5b845` (the six `tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows*/` sessions)
with a host run of the same cases at main `351fb70d`, and gives the image a verdict:
**rerun** (main has fixed on the host a refusal, an API verdict or an abort the
board hit), **no change expected**, or **plan #376** (S03-S08, T14, TF1, TF2, left
to that plan's board item). HOST-ONLY: no board figure here is new. The board rerun
was not run (see "Board").

## Result

**34 images to rerun** (8,497 mustpass cases): A1 FB3 M1 R1 SQ S10 S32a S32d T03
F3C1 F3C2 F3C3 F3C4 F3F1 F3F2 F3F3 F3F4 F3S1 F3S2 T05a T05b T05c T05d T06 T07 T08
T09 T10 T11 T12 T13 T17 UB2 UB3. In them, 2,061 cases the board did not pass at
b1b5b845 (Fail, ResourceError or never began) now build and draw on the host with
no refusal and no API verdict, and 176 ETC2/EAC cases the board reported
NotSupported now run (455c3bb5). 141 board non-Pass cases in these images are
still refused, or still get the same API verdict, on the host ("still" column).

**57 images: no change expected.** 44 passed in full at b1b5b845, or had only
NotSupported results that the host still reports. 13 had Fails the host does not
change: FB1 FB2 FB4 FR1 FR2 LT S16 UA1 (render verdicts, or API verdicts the host
gives the same), and S32b and S32c1-4, whose refusals moved but did not go (below).

**9 images: plan #376** (S03-S08, T14, TF1, TF2). Their host columns come from the
chunked run only (not one process per case), so they are information for that
plan, not a verdict.

The map was made the same way at `b1e4c0f5` (the closed pull request #7872) and at
`684ef106` (the closed #7704). Outside plan #376's images, all three give the same
case-for-case result: none of the commits since b1e4c0f5 (949f0bef's clear with
draw buffer 0 `GL_NONE`, the RGB10_A2 read-back and copy pins a5de7f30, 07306187 and
351fb70d, dede8650 and 33e7529e's draw-target knob in the board images, default
off) moves a case of the 91 other images. Only 6d2e7d0d (the V3D's 4096 tile
memory, plan #376's fix-v3d) moves cases, all in S03-S08: the precision cases that
were refused as "a target larger than the tile memory is sized for (2048x2048)"
and then `GL_INVALID_OPERATION` now draw (host QualityWarning or Pass). Their
whole-run chunks c_107 and c_109 no longer stop and need no second process. 54
cases are still refused, in S05, S06 and S08: the `mat4`, `mat4x2` and `mat4x3`
cases of `precision.{matrixcompmult,outerproduct,transpose}`, with
`Failed to set up framebuffer object: glGetError() returned GL_INVALID_ENUM`
(`glsShaderExecUtil.cpp`). That is for plan #376 to place.

### Two causes the host still refuses (issue drafts in `drafts.md`)

1. **The job builder writes no QUNIFORM_TEXTURE_DEPTH (46) or
   QUNIFORM_TEXTURE_ARRAY_SIZE (47) word: 48 cases.** Since 8ae0a44b the compiler
   accepts `textureSize` of a 3D or 2D-array sampler. Since 79b98f34 a 3D
   `textureGrad` takes its level from the textureSize words. Both programs read
   uniform 46 or 47, and the uniform switch of `driver/src/v3d/v3d_job.c`
   (`default: cl->bad_field = true`) has no case for either, so each such draw is
   refused: `yakogl/v3d: draw refused: the job builder refused the draw (a uniform,
   program or primitive it does not write)`, `GL_INVALID_OPERATION`. S32b: the 16
   textureSize cases of `[iu]?sampler3d*` and `[iu]?sampler2darray*` (not shadow).
   S32a: the 32 `texturegrad`, `texturegradoffset`, `textureprojgrad` and
   `textureprojgradoffset` cases of `[iu]?sampler3d*`. 32db46e9
   (`tests/es3_texsize3d_v3d.rs`) pins both halves of this on main. The fix is in
   `driver/src/v3d/` (plan #321) and in `src/v3d/ffi.rs` / `backend.rs` (plan #376),
   so it is an issue draft here, not a change.
2. **RET16_OF_32 for a unit that only textureSize reads: 6 cases** (S32c1-4, and
   S32b's `sampler2darrayshadow_{fragment,vertex}`). The cause is the one 361fdb4e
   names. `src/v3d/backend.rs` `texture_units` takes each unit's return size from
   the P1 words. A unit that only textureSize reads has no P1 word, so it is
   checked as a 16-bit look-up, and its R32F depth texture is refused:
   `yakogl/v3d: draw refused: a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI,
   RGBA32I) read by a look-up compiled for the 16-bit TMU output`, all six, each in
   its own process. Since b7c1484e the refusal is `GL_INVALID_OPERATION`, so on the
   host each case ends in Fail and dEQP's run goes on. On the board at b1b5b845,
   S32c aborted at `texturesize.sampler2dshadow_fragment` and the other three never
   began. The fix belongs in `backend.rs` (plan #376), so it is an issue draft. The
   four single-case images S32c1-4 stay split, so the rest of S32 runs.

Other board non-Pass cases in rerun images that get the same verdict on the host
(not new): FB3 78 `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`, S10 15 derivate
`checkFramebufferStatus`, SQ 6 and A1 2 queries, M1 2 `Got invalid error`, T09 6
2048x2048 textures over the tiled-copy budget and the texture arena (the board line
was the same at b1b5b845).

The host run reports 2 `OutOfScratch` refusals, on cases it passes as well:
`shaders.metamorphic.synthetic.variant_3` and `uniform_api.random.83`. These are
not the UB2 and UB3 scratch-arena cases.

### M1

M1's integer-target clear abort (`pbo.renderbuffer.rg16i_clears`, 62 cases never
began) is the `0ff58329` record that `docs/es3/pi4-gles3-fails.tsv` still holds. At
b1b5b845, M1 ran to the end: 524 cases began and 524 have a result, and
`rg16i_clears` passed (`tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows16-30/M1.console.txt`).
So no fix from plan #364 was needed for it. Its clear on an incomplete framebuffer
(82dcceba) is on main anyway and moves no M1 case on the host. M1 is on the rerun
list for its 5 `negative_api` cases (bc200f21, bece11ec).

## Board

Not run. This worker had no `target:rpi4` lease (no `BOARD_UART`, no
`target:rpi4-console`), so none of the following were made: stage `deqp-tree-pi4`,
`board-stage.sh pi4 deqp`, the ES 2.0 regression rows (stage `board-common-pi4`),
and the per-image `cycles.txt`, `table.txt`, consoles and tallies. The rerun is the
34 images above, all built from one origin/main commit with
`DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` and run with `tools/board.sh pi4`,
as the b1b5b845 session READMEs describe. Expect S32a's 32 cases to stay Fail until
cause 1 is fixed. The board session's README replaces this one, or sits beside it as
`<date>-pi4-gles3-<sha8>-rerun/`.

## Method

Host, at 351fb70d, VK-GL-CTS `93bca01861b0` with `tools/cts/cts-yakogl-target.patch`:

    tools/cts/host-deqp-run.sh --gpu=v3d --module=gles3 --cts=<VK-GL-CTS> --jobs=4

The whole mustpass ran: 42,810 of 42,810 cases began (68 chunk processes exited 0
and 373 exited 1). Only 2 chunks needed a second process, both in `texture.units`
(T14, plan #376's):

    host-deqp-run: chunk c_344: exits 1 x12; 100 of 100 began; stopped: after ...texture.units.4_units.only_cube.7; ...; never began: 0
    host-deqp-run: chunk c_345: exits 1 x2; 100 of 100 began; stopped: after ...texture.units.all_units.only_cube.0; ...; never began: 0

The V3D library logs a draw refusal once per cause and process
(`src/v3d/backend.rs` `count_refusal`), so in a 100-case chunk only the first case
of each cause shows its line. The 2,763 board non-Pass cases outside plan #376 were
run again, one process per case, with the same binary, library and dEQP flags
(`percase.sh`), and their results and lines take precedence. `rerun-map.py` and
`percase.sh` are in `method.md`, with the commands that make the case list, the map
(`failmap.tsv`, `hostmap.tsv`) and the replacement block for
`docs/es3/pi4-gles3-fails.tsv`. The tsv files are not committed. Each is rebuilt
from the b1b5b845 consoles in the tree plus a host run, and the block comes out
byte for byte equal to the one the b1e4c0f5 record made (7,110 rows).

`rerun-map.py` assigns each case a class:
- a refusal is a `yakogl` line that refuses;
- an API verdict is a dEQP message the host reproduces (compile, link, GL error,
  framebuffer status, query);
- a render verdict is a pixel message. The host reads black, so it is HOST-ONLY and
  means nothing.

A board case counts as fixed on the host when its board class was a refusal, an API
verdict or "never began", and the host gives it neither a refusal nor an API
verdict.

## Per image

"fixed" counts the board non-Pass cases fixed on the host, plus the NotSupported
cases the host no longer reports. "still" counts the board non-Pass cases the host
still refuses or gives an API verdict; for plan #376's rows it comes from the
chunked run. "commits" lists the commits since b1b5b845 that removed the board's
refusal or verdict class in that image (the class-to-commit list is in `drafts.md`).

| image | mustpass | b1b5b845 P / F / N.S. / other / never began | fixed on the host | still refused or API on the host | verdict | commits |
|---|---:|---|---:|---:|---|---|
| A1 | 472 | 461 / 11 / 0 / 0 / 0 | 9 | 2 | rerun | 31127ec8 455c3bb5 6581f22b cb52fae8 |
| B1 | 695 | 695 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| C1 | 602 | 602 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| D1 | 443 | 367 / 0 / 76 / 0 / 0 | - | 0 | no change expected | - |
| FB1 | 580 | 559 / 14 / 7 / 0 / 0 | - | 14 | no change expected | - |
| FB2 | 610 | 445 / 19 / 146 / 0 / 0 | - | 16 | no change expected | - |
| FB3 | 329 | 157 / 136 / 36 / 0 / 0 | 48 | 78 | rerun | 8a5da05e 8ae0a44b |
| FB4 | 509 | 108 / 335 / 66 / 0 / 0 | - | 90 | no change expected | - |
| FO1 | 677 | 677 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| FO2 | 475 | 475 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| FO3 | 227 | 227 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| FO4 | 450 | 450 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| FO5 | 475 | 475 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| FO6 | 866 | 866 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| FR1 | 420 | 360 / 12 / 48 / 0 / 0 | - | 0 | no change expected | - |
| FR2 | 520 | 422 / 31 / 67 / 0 / 0 | - | 26 | no change expected | - |
| LT | 56 | 55 / 1 / 0 / 0 / 0 | - | 0 | no change expected | - |
| M1 | 507 | 438 / 11 / 58 / 0 / 0 | 5 | 2 | rerun | bc200f21 bece11ec |
| PR | 504 | 504 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| R1 | 303 | 257 / 16 / 26 / 4 / 0 | 14 | 0 | rerun | 8ae0a44b |
| SQ | 946 | 911 / 26 / 9 / 0 / 0 | 20 | 6 | rerun | 31127ec8 455c3bb5 6581f22b 8ae0a44b cb52fae8 |
| S01 | 861 | 861 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S02 | 392 | 392 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S03 | 192 | 0 / 0 / 0 / 1 / 191 | 192 | 0 | plan #376 | 6d2e7d0d (host) |
| S04 | 201 | 0 / 0 / 0 / 1 / 200 | 201 | 0 | plan #376 | 6d2e7d0d (host) |
| S05 | 202 | 0 / 0 / 0 / 1 / 201 | 184 | 18 | plan #376 | 6d2e7d0d (host) |
| S06 | 204 | 0 / 0 / 0 / 1 / 203 | 186 | 18 | plan #376 | 6d2e7d0d (host) |
| S07 | 206 | 0 / 0 / 0 / 0 / 206 | 206 | 0 | plan #376 | 6d2e7d0d (host) |
| S08 | 142 | 0 / 0 / 0 / 1 / 141 | 124 | 18 | plan #376 | 6d2e7d0d (host) |
| S09 | 1156 | 1156 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S10 | 690 | 605 / 85 / 0 / 0 / 0 | 15 | 15 | rerun | 8a5da05e |
| S11 | 756 | 756 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S12 | 1216 | 1216 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S13 | 957 | 957 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S14 | 594 | 594 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S15 | 972 | 972 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S16 | 720 | 719 / 1 / 0 / 0 / 0 | - | 1 | no change expected | - |
| S17 | 716 | 716 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S18 | 864 | 864 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S19 | 716 | 716 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S20 | 666 | 666 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S21 | 680 | 680 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S22 | 768 | 768 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S23 | 866 | 866 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S24 | 1126 | 1126 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S25 | 689 | 689 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S26 | 700 | 700 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S27 | 987 | 987 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S5f | 66 | 66 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S5v | 70 | 70 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S30 | 915 | 915 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S31 | 648 | 648 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| S32a | 522 | 262 / 260 / 0 / 0 / 0 | 228 | 32 | rerun | 06bd7bc5 455c3bb5 79b98f34 84d8c388 8ae0a44b |
| S32b | 34 | 0 / 34 / 0 / 0 / 0 | - | 18 | no change expected | - |
| S32c1 | 1 | 0 / 0 / 0 / 1 / 0 | - | 1 | no change expected | - |
| S32c2 | 1 | 0 / 0 / 0 / 0 / 1 | - | 1 | no change expected | - |
| S32c3 | 1 | 0 / 0 / 0 / 0 / 1 | - | 1 | no change expected | - |
| S32c4 | 1 | 0 / 0 / 0 / 0 / 1 | - | 1 | no change expected | - |
| S32d | 118 | 116 / 2 / 0 / 0 / 0 | 2 | 0 | rerun | 06840143 83644bb2 eceaabe6 |
| T01 | 266 | 0 / 0 / 266 / 0 / 0 | - | 0 | no change expected | - |
| T02 | 129 | 117 / 0 / 12 / 0 / 0 | - | 0 | no change expected | - |
| T03 | 134 | 0 / 122 / 12 / 0 / 0 | 122 | 0 | rerun | 8ae0a44b |
| T04 | 124 | 112 / 0 / 12 / 0 / 0 | - | 0 | no change expected | - |
| F3C1 | 14 | 0 / 14 / 0 / 0 / 0 | 14 | 0 | rerun | 8ae0a44b |
| F3C2 | 14 | 0 / 14 / 0 / 0 / 0 | 14 | 0 | rerun | 8ae0a44b |
| F3C3 | 14 | 0 / 14 / 0 / 0 / 0 | 14 | 0 | rerun | 8ae0a44b |
| F3C4 | 14 | 0 / 14 / 0 / 0 / 0 | 14 | 0 | rerun | 8ae0a44b |
| F3F1 | 15 | 0 / 15 / 0 / 0 / 0 | 15 | 0 | rerun | 8ae0a44b |
| F3F2 | 15 | 0 / 15 / 0 / 0 / 0 | 15 | 0 | rerun | 8ae0a44b |
| F3F3 | 15 | 0 / 15 / 0 / 0 / 0 | 15 | 0 | rerun | 8ae0a44b |
| F3F4 | 15 | 0 / 14 / 1 / 0 / 0 | 14 | 0 | rerun | 8ae0a44b |
| F3F5 | 11 | 0 / 0 / 11 / 0 / 0 | - | 0 | no change expected | - |
| F3S1 | 13 | 0 / 13 / 0 / 0 / 0 | 13 | 0 | rerun | 8ae0a44b |
| F3S2 | 13 | 0 / 13 / 0 / 0 / 0 | 13 | 0 | rerun | 8ae0a44b |
| T05a | 196 | 142 / 34 / 20 / 0 / 0 | 48 | 0 | rerun | 455c3bb5 8ae0a44b |
| T05b | 12 | 0 / 4 / 8 / 0 / 0 | 12 | 0 | rerun | 03f4bbd9 333a19f7 455c3bb5 |
| T05c | 4 | 0 / 0 / 0 / 1 / 3 | 4 | 0 | rerun | 03f4bbd9 333a19f7 455c3bb5 |
| T05d | 8 | 0 / 0 / 8 / 0 / 0 | 8 | 0 | rerun | 455c3bb5 |
| T06 | 326 | 106 / 208 / 12 / 0 / 0 | 206 | 0 | rerun | 8ae0a44b |
| T07 | 224 | 140 / 84 / 0 / 0 / 0 | 64 | 0 | rerun | 8ae0a44b |
| T08 | 288 | 0 / 288 / 0 / 0 / 0 | 288 | 0 | rerun | 06bd7bc5 455c3bb5 84d8c388 8ae0a44b |
| T09 | 222 | 72 / 150 / 0 / 0 / 0 | 144 | 6 | rerun | 06bd7bc5 455c3bb5 84d8c388 |
| T10 | 334 | 212 / 122 / 0 / 0 / 0 | 118 | 0 | rerun | 8ae0a44b |
| T11 | 320 | 184 / 136 / 0 / 0 / 0 | 135 | 0 | rerun | 8ae0a44b |
| T12 | 197 | 119 / 78 / 0 / 0 / 0 | 76 | 0 | rerun | 8ae0a44b |
| T13 | 312 | 180 / 132 / 0 / 0 / 0 | 129 | 0 | rerun | 8ae0a44b |
| T14 | 416 | 136 / 88 / 0 / 1 / 191 | 267 | 11 | plan #376 | - |
| T15 | 432 | 0 / 0 / 432 / 0 / 0 | - | 0 | no change expected | - |
| T16 | 504 | 0 / 0 / 504 / 0 / 0 | - | 0 | no change expected | - |
| T17 | 504 | 36 / 0 / 216 / 1 / 251 | 396 | 0 | rerun | 03f4bbd9 333a19f7 455c3bb5 |
| TF1 | 444 | 4 / 0 / 0 / 1 / 439 | 440 | 0 | plan #376 | - |
| TF2 | 438 | 11 / 0 / 0 / 1 / 426 | 427 | 0 | plan #376 | - |
| UB1 | 968 | 968 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| UB2 | 755 | 754 / 1 / 0 / 0 / 0 | 1 | 0 | rerun |  (scratch: not isolated) |
| UB3 | 632 | 628 / 4 / 0 / 0 / 0 | 4 | 0 | rerun |  (scratch: not isolated) |
| UA1 | 759 | 758 / 1 / 0 / 0 / 0 | - | 1 | no change expected | - |
| UA2 | 1014 | 1014 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| UA3 | 408 | 408 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| VA1 | 203 | 203 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| VA2 | 802 | 802 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |

Rows with "(scratch: not isolated)": the host builds both UB2 and UB3 cases at
351fb70d. `SCRATCH_BYTES` is unchanged since b1b5b845, so the commit that moved
them was not isolated.

## Files

- `README.md`: this map.
- `method.md`: `rerun-map.py`, `percase.sh`, and the commands that rebuild
  `failmap.tsv`, `hostmap.tsv` and the replacement block for
  `docs/es3/pi4-gles3-fails.tsv`.
- `drafts.md`: the #1261 comment, two issue drafts, and the RESULTS.md, STATUS.md,
  ROADMAP.md and `docs/es3-cts-failures.md` lines for the owner.
