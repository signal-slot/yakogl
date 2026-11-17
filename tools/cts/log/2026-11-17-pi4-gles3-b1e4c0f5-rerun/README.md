<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rerun map at b1e4c0f5: which b1b5b845 images main has changed (host only)

commit: b1e4c0f5495f (host check; the board rerun is not done, see "Board")

Plan #380, step "the Pi 4's remaining GLES3 images" (issue #1261). For each of the
100 images of `tools/cts/pi4-gles3-tree.txt`, this record compares the image's board
result at `b1b5b845` (the six `tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows*/`
sessions) with a host run of the same mustpass cases at main `b1e4c0f5`, and gives
the image a verdict: **rerun** (main has fixed on the host a refusal, an API verdict
or an abort that the board hit), **no change expected**, or **plan #376** (S03-S08,
T14, TF1, TF2, left to that plan's board item).

## Result

**34 images to rerun** (8,497 mustpass cases): A1 FB3 M1 R1 SQ S10 S32a S32d T03
F3C1 F3C2 F3C3 F3C4 F3F1 F3F2 F3F3 F3F4 F3S1 F3S2 T05a T05b T05c T05d T06 T07 T08
T09 T10 T11 T12 T13 T17 UB2 UB3. In them, 2,061 cases the board did not pass at
b1b5b845 (Fail, ResourceError or never began) now build and draw on the host with
no refusal and no API verdict, and 176 ETC2/EAC cases the board reported
NotSupported are now run (455c3bb5). 141 board non-Pass cases in these images are
still refused or still give the same API verdict on the host (column "still").

**57 images: no change expected.** 44 passed in full or had only NotSupported at
b1b5b845 that the host still reports. 13 had Fails the host does not change:
FB1 FB2 FB4 FR1 FR2 LT S16 UA1 (render verdicts, or API verdicts the host gives
the same), and S32b S32c1-4, whose refusals moved but did not go (below).

The same map at `684ef106` (the closed pull request #7704's record, made the same way)
gave the same `failmap.tsv` byte for byte, and the same `host-s32c.log` but for its
commit line: the commits after it (82dcceba's clear on an incomplete framebuffer,
2a4bc97a's one- and two-component float outputs, ab8b7cbf's program binaries,
b9c663fa's plane key, 008306cb's d24s8 helpers) move no case of the 100 images.

**9 images: plan #376** (S03-S08, T14, TF1, TF2). Their host columns come from the
chunked run only (not one process per case) and are not a verdict.

### Two causes the host still refuses (issue drafts in `drafts.md`)

1. **The job builder writes no QUNIFORM_TEXTURE_DEPTH (46) or
   QUNIFORM_TEXTURE_ARRAY_SIZE (47) word: 48 cases.** 8ae0a44b made the compiler
   accept `textureSize` of a 3D or 2D-array sampler, and 79b98f34 computes a
   `textureGrad` level from the textureSize words (`grad_lod`, lane 2 of a 3D
   sampler is its depth). Both programs now read uniforms 46 / 47, and
   `driver/src/v3d/v3d_job.c` (the uniform switch: `default: cl->bad_field = true`)
   has no case for them, so every such draw is refused:
   `yakogl/v3d: draw refused: the job builder refused the draw (a uniform, program or
   primitive it does not write)`, `GL_INVALID_OPERATION`. S32b: the 16 textureSize
   cases of `[iu]?sampler3d*` and `[iu]?sampler2darray*` (not shadow); S32a: the 32
   `texturegrad`, `texturegradoffset`, `textureprojgrad` and `textureprojgradoffset`
   cases of `[iu]?sampler3d*`. Before, these failed at compile (`NotYet("textureSize
   of a 3D or array sampler")`, `NotYet("a texture look-up with gradients")`); the
   board result stays Fail. The fix lies in `driver/src/v3d/` (plan #321) and in
   `src/v3d/ffi.rs` / `backend.rs` (`V3dJobTmu` has no depth or layer count;
   `texture_units` names a unit by words 40, 41, 44, 45 only; plan #376 holds
   backend.rs), so it is an issue draft, not a change here.
2. **RET16_OF_32 for a unit only textureSize reads: 6 cases** (S32c1-4 and S32b's
   `sampler2darrayshadow_{fragment,vertex}`). The cause is the one 361fdb4e names:
   `texture_units` takes each unit's return size from the P1 words, a textureSize-only
   unit has none, and a depth texture (R32F) is refused as a 16-bit look-up. Since
   b7c1484e the refusal is `GL_INVALID_OPERATION`, so on the host each case ends Fail
   instead of stopping dEQP; S32c1-4 stay split as they are.

Other board non-Pass cases the host still gives the same verdict, in rerun images
(not new): FB3 78 `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`, S10 15 derivate
`checkFramebufferStatus`, SQ 6 and A1 2 queries, M1 2 `Got invalid error`, T09 6
2048x2048 textures over the tiled-copy budget and the texture arena (the board line
was the same at b1b5b845).

### M1

M1's integer-target clear abort (`pbo.renderbuffer.rg16i_clears`, 62 never began)
is the `0ff58329` record that `docs/es3/pi4-gles3-fails.tsv` still holds. At
b1b5b845 M1 ran to the end (524 begun, 524 with a result; `rg16i_clears` passed,
`tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows16-30/M1.console.txt`), so no fix of
plan #364 was needed for it (its clear on an incomplete framebuffer, 82dcceba, is
on main all the same and moves no M1 case on the host); M1 is in the rerun list for its 5 `negative_api`
cases (bc200f21, bece11ec). `fails-b1b5b845.tsv` here is the replacement block that
moves the tsv onto b1b5b845 for all 100 images.

## Board

Not run. This worker has no `target:rpi4` lease and no `BOARD_UART`, so stage
`deqp-tree-pi4`, `board-stage.sh pi4 deqp`, the ES 2.0 regression rows (stage
`board-common-pi4`) and the per-image `cycles.txt`, `table.txt`, consoles and
tallies were not made. The rerun is the 34 images above, built from one origin/main
commit with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` and run with
`tools/board.sh pi4` as the b1b5b845 session READMEs describe; S32a and S32b keep
their 48 refused cases until cause 1 is fixed.

## Method

Host, at b1e4c0f5, VK-GL-CTS `93bca01861b0` with `tools/cts/cts-yakogl-target.patch`:

    tools/cts/host-deqp-run.sh --gpu=v3d --module=gles3 --cts=<VK-GL-CTS> --jobs=4

The whole mustpass, 42,810 of 42,810 cases began; 4 chunks needed a second
process (precision S03-S08 and `texture.units` T14, plan #376's) and no case was
left without a start (`host-run.log`). The V3D library logs a draw refusal once per
cause and process (`src/v3d/backend.rs` `count_refusal`), so in a 100-case chunk
only the first case of a cause shows its line. The 2,763 board non-Pass cases
outside plan #376 were run again one process per case (`percase.sh`, the same
binary, library and dEQP flags), and their results and lines win:

    awk -F'\t' 'NR>5 && $3!="NotSupported" && $1!~/^(S0[3-8]|T14|TF1|TF2)$/ {print $2}' failmap.tsv > list
    bash percase.sh list build/rm/pc 4
    python3 -I rerun-map.py --tree tools/cts/pi4-gles3-tree.txt \
        --mustpass tools/cts/gles3-mustpass.txt \
        --board 'tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows*' \
        --host build/host-deqp/run-b1e4c0f5-v3d-gles3 --percase build/rm/pc \
        --failmap failmap.tsv --hostmap hostmap.tsv

(the case list is taken from a first `rerun-map.py` run without `--percase`).
`rerun-map.py` says what a class is: a refusal is a `yakogl` line that refuses, an
API verdict is a dEQP message the host reproduces (compile, link, GL error,
framebuffer status, query), a render verdict is a pixel message (the host reads
black, HOST-ONLY). A board case is fixed on the host when its class was a refusal,
an API verdict or never began, and the host gives it neither.

## Per image

"fixed" counts the board non-Pass cases fixed on the host plus the NotSupported the
host no longer reports; "still" the board non-Pass cases the host still refuses or
gives an API verdict (for plan #376's rows, from the chunked run). Commits: those in
b1b5b845..b1e4c0f5 that removed the board's refusal or verdict class in that image
(the class to commit list is in `drafts.md`).

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
| S03 | 192 | 0 / 0 / 0 / 1 / 191 | 14 | 178 | plan #376 | - |
| S04 | 201 | 0 / 0 / 0 / 1 / 200 | 10 | 191 | plan #376 | - |
| S05 | 202 | 0 / 0 / 0 / 1 / 201 | 10 | 192 | plan #376 | - |
| S06 | 204 | 0 / 0 / 0 / 1 / 203 | 6 | 198 | plan #376 | - |
| S07 | 206 | 0 / 0 / 0 / 0 / 206 | 12 | 194 | plan #376 | - |
| S08 | 142 | 0 / 0 / 0 / 1 / 141 | 6 | 136 | plan #376 | - |
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

## Files

- `hostmap.tsv`: one row per image (100): b1b5b845 counts, host counts, fixed,
  NotSupported changed, verdict, the board's classes and the host's remaining ones.
- `failmap.tsv`: every mustpass case the Pi 4 did not pass at b1b5b845 (7,110), its
  board status and class, host status and class, and whether it is fixed on the host.
- `fails-b1b5b845.tsv`: the replacement block for `docs/es3/pi4-gles3-fails.tsv`
  (plan #320's file, handed over in `drafts.md`).
- `host-run.log`: the whole-mustpass host run's report (`host-deqp-run.sh`).
- `host-s32c.log`: the 54 S32a / S32b / S32c1-4 cases still refused at the draw, each
  from its own process: the case, the refusal line, the result.
- `rerun-map.py`, `percase.sh`: the map and the one-process-per-case run.
- `drafts.md`: the #1261 comment, two issue drafts, and the RESULTS.md, STATUS.md,
  ROADMAP.md and `docs/es3-cts-failures.md` lines for the owner.
