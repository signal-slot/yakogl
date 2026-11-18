<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rerun map: b1b5b845 to a1b8a613

Renamed from 2026-11-pi4-gles3-a1b8a613-rerun: a map with no board run, so that the board rerun directory is the only match of tools/cts/log/2026-11-pi4-gles3-*-rerun.

commit a1b8a613962bbac4c56c7ad876a26e5413d51465

Item "the Pi 4's remaining GLES3 images" of plan #380 (issue #1261). **Board: not run.** This worker held no `target:rpi4` / `target:rpi4-console` lease, so no image was built or booted and every board column below says "not run". Nothing here is a board result.

The map says, for each of the 100 images of `tools/cts/pi4-gles3-tree.txt` (the task text said 94), whether it needs a rerun from a1b8a613, and the commits in `b1b5b845..a1b8a613` (1885 commits) that touch the code behind its failing cases. The commit columns are read from commit subjects and from the board failure groups of the b1b5b845 sessions: they are candidates, not proofs. The host column is the in-tree v3d host record (`docs/es3/host-gles3-notpass.tsv`, HOST-ONLY, at `26d1c4a2`). **No new host run was made** (no deqp-gles3 host build on this worker; `MODULE=gles3 tools/cts/host-deqp-run.sh` not run), so the earlier claims "all 396 T17 cases begin on the host" and "T09: six 2048x2048 cases over budget" were not re-measured, and the S32a count of 32 is the `26d1c4a2` record. For M1 the tree itself shows `82dcceba` (plan #364) in the range.

`TREE_ROWS=0,6-7,16-17,19-20,30,52,54-58,61,63-70,72-85,89`  (0-based rows of the tree file, A1 = 0; 38 images)

S32a: its remaining refusals are 32 `GL_INVALID_OPERATION` draw errors on the 3D-sampler gradient forms (below). They fail the case, not the run, so the rest of the image runs; nothing was re-split, `tools/cts/pi4-gles3-tree.txt` is unchanged and `check-gles3-tree.py` still matches all 42,810 cases once.

## Map

| row | image | cases | b1b5b845 board (Pass/Fail/NS/other) | host v3d at 26d1c4a2 (counting rows) | decision | commits in b1b5b845..a1b8a613 (in / after 26d1c4a2) | board now |
|---|---|---|---|---|---|---|---|
| 0 | A1 | 472 | 461/11/0/0 (rows00-15) | 0 | **rerun**: implementation_limits 11 Fail: uniform block size / combined uniform components, 15 varying vectors, compressed format count | `cb52fae8` after `6581f22b` after `d17f4362` after `455c3bb5` after | not run |
| 1 | B1 | 695 | 695/0/0/0 (rows00-15) | 0 | no change expected: all Pass at the last record | - | not run |
| 2 | C1 | 602 | 602/0/0/0 (rows00-15) | 0 | no change expected: all Pass at the last record | - | not run |
| 3 | D1 | 443 | 367/0/76/0 (rows00-15) | 76 NotSupported refusal | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 4 | FB1 | 580 | 559/14/7/0 (rows00-15) | 7 NotSupported gl-error | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 5 | FB2 | 610 | 445/19/146/0 (rows00-15) | 146 NotSupported refusal, 16 Fail gl-error | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 6 | FB3 | 329 | 157/136/36/0 (rows00-15) | 36 NotSupported refusal | **rerun**: fbo.color repeated_clear, tex2darray/tex3d, depth_test_clamp: clears of several or mixed attachments, 128-bit tile rows, incomplete-framebuffer clear | `7fa02269` in `3c3e5470` in `684ef106` after `4bcb55c3` in `82dcceba` after `949f0bef` after | not run |
| 7 | FB4 | 509 | 108/335/66/0 (rows00-15) | 66 NotSupported refusal, 40 Fail gl-error | **rerun**: fbo.render resize/shared/recreate, msaa 2/4 samples: multi-attachment clear, per-RT 4x buffers, msaa renderbuffer rules | `7fa02269` in `3c3e5470` in `684ef106` after `4bcb55c3` in `f3ed1fea` after `2830b218` in `c1288b0c` in | not run |
| 8 | FO1 | 677 | 677/0/0/0 (rows00-15) | 0 | no change expected: all Pass at the last record | - | not run |
| 9 | FO2 | 475 | 475/0/0/0 (rows00-15) | 0 | no change expected: all Pass at the last record | - | not run |
| 10 | FO3 | 227 | 227/0/0/0 (rows00-15) | 0 | no change expected: all Pass at the last record | - | not run |
| 11 | FO4 | 450 | 450/0/0/0 (rows00-15) | 0 | no change expected: all Pass at the last record | - | not run |
| 12 | FO5 | 475 | 475/0/0/0 (rows00-15) | 0 | no change expected: all Pass at the last record | - | not run |
| 13 | FO6 | 866 | 866/0/0/0 (rows00-15) | 0 | no change expected: all Pass at the last record | - | not run |
| 14 | FR1 | 420 | 360/12/48/0 (rows00-15) | 48 NotSupported refusal | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 15 | FR2 | 520 | 422/31/67/0 (rows00-15) | 67 NotSupported refusal, 26 Fail gl-error | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 16 | LT | 56 | 55/1/0/0 (rows16-30) | 0 | **rerun**: lifetime.attach.deleted_output: a deleted buffer name bound again is a new buffer | `da0eede3` in | not run |
| 17 | M1 | 507 | 438/11/58/0 (rows16-30) | 58 NotSupported refusal, 2 Fail gl-error | **rerun**: pbo / multisample / negative_api: incomplete-framebuffer clear (#364), GL_NONE draw buffer 0 clear, msaa queries | `82dcceba` after `949f0bef` after `7fa02269` in `f3ed1fea` after `2830b218` in `4bcb55c3` in | not run |
| 18 | PR | 504 | 504/0/0/0 (rows16-30) | 0 | no change expected: all Pass at the last record | - | not run |
| 19 | R1 | 303 | 257/16/26/4 (rows16-30) | 26 NotSupported refusal | **rerun**: rasterization.fbo rbo_multisample_max, samplers.multi_tex_3d: msaa renderbuffer, 3D look-ups | `f3ed1fea` after `2830b218` in `4bcb55c3` in `8ae0a44b` in | not run |
| 20 | SQ | 946 | 911/26/9/0 (rows16-30) | 9 NotSupported refusal | **rerun**: state_query.integers: max uniform/varying/draw buffers, compressed formats, max samples | `cb52fae8` after `6581f22b` after `455c3bb5` after `2830b218` in | not run |
| 21 | S01 | 861 | 861/0/0/0 (rows16-30) | 0 | no change expected: all Pass at the last record | - | not run |
| 22 | S02 | 392 | 392/0/0/0 (rows16-30) | 0 | no change expected: all Pass at the last record | - | not run |
| 23 | S03 | 192 | 0/0/0/1 (rows16-30) | 178 Fail gl-error | no change expected here: its refusals and files are plan #376 | - | not run |
| 24 | S04 | 201 | 0/0/0/1 (rows16-30) | 191 Fail gl-error | no change expected here: its refusals and files are plan #376 | - | not run |
| 25 | S05 | 202 | 0/0/0/1 (rows16-30) | 191 Fail gl-error, 1 ResourceError gl-error | no change expected here: its refusals and files are plan #376 | - | not run |
| 26 | S06 | 204 | 0/0/0/1 (rows16-30) | 197 Fail gl-error, 1 ResourceError gl-error | no change expected here: its refusals and files are plan #376 | - | not run |
| 27 | S07 | 206 | 0/0/0/0 (rows16-30) | 194 Fail gl-error | no change expected here: its refusals and files are plan #376 | - | not run |
| 28 | S08 | 142 | 0/0/0/1 (rows16-30) | 136 Fail gl-error | no change expected here: its refusals and files are plan #376 | - | not run |
| 29 | S09 | 1156 | 1156/0/0/0 (rows16-30) | 0 | no change expected: all Pass at the last record | - | not run |
| 30 | S10 | 690 | 605/85/0/0 (rows16-30) | 0 | **rerun**: shaders.fragdepth gl_FragDepth TLB Z write; derivate on msaa/float targets | `8a5da05e` in `de737051` in `4bcb55c3` in `684ef106` after | not run |
| 31 | S11 | 756 | 756/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 32 | S12 | 1216 | 1216/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 33 | S13 | 957 | 957/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 34 | S14 | 594 | 594/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 35 | S15 | 972 | 972/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 36 | S16 | 720 | 719/1/0/0 (rows31-45) | 1 Fail compile | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 37 | S17 | 716 | 716/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 38 | S18 | 864 | 864/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 39 | S19 | 716 | 716/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 40 | S20 | 666 | 666/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 41 | S21 | 680 | 680/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 42 | S22 | 768 | 768/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 43 | S23 | 866 | 866/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 44 | S24 | 1126 | 1126/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 45 | S25 | 689 | 689/0/0/0 (rows31-45) | 0 | no change expected: all Pass at the last record | - | not run |
| 46 | S26 | 700 | 700/0/0/0 (rows46-60) | 0 | no change expected: all Pass at the last record | - | not run |
| 47 | S27 | 987 | 987/0/0/0 (rows46-60) | 0 | no change expected: all Pass at the last record | - | not run |
| 48 | S5f | 66 | 66/0/0/0 (rows46-60) | 0 | no change expected: all Pass at the last record | - | not run |
| 49 | S5v | 70 | 70/0/0/0 (rows46-60) | 0 | no change expected: all Pass at the last record | - | not run |
| 50 | S30 | 915 | 915/0/0/0 (rows46-60) | 0 | no change expected: all Pass at the last record | - | not run |
| 51 | S31 | 648 | 648/0/0/0 (rows46-60) | 0 | no change expected: all Pass at the last record | - | not run |
| 52 | S32a | 522 | 262/260/0/0 (rows46-60) | 32 Fail gl-error | **rerun**: texture_functions: 260 refusals at b1b5b845; host at 26d1c4a2 has 32 left, the 3D gradient forms | `79b98f34` in `8ae0a44b` in `455c3bb5` after `b7c1484e` in | not run |
| 53 | S32b | 34 | 0/34/0/0 (rows46-60) | 0 | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 54 | S32c1 | 1 | older records only | 0 | **rerun**: texturesize.sampler2dshadow_fragment: shadow look-ups end to end; a refusal is now GL_INVALID_OPERATION | `06bd7bc5` in `455c3bb5` after `b7c1484e` in | not run |
| 55 | S32c2 | 1 | older records only | 0 | **rerun**: as S32c1 (sampler2dshadow_vertex) | `06bd7bc5` in `455c3bb5` after `b7c1484e` in | not run |
| 56 | S32c3 | 1 | older records only | 0 | **rerun**: as S32c1 (samplercubeshadow_fragment) | `06bd7bc5` in `455c3bb5` after `b7c1484e` in | not run |
| 57 | S32c4 | 1 | older records only | 0 | **rerun**: as S32c1 (samplercubeshadow_vertex) | `06bd7bc5` in `455c3bb5` after `b7c1484e` in | not run |
| 58 | S32d | 118 | 116/2/0/0 (rows46-60) | 0 | **rerun**: shaders.uniform_block.valid 2 Fail: uniform block size 16384 | `cb52fae8` after | not run |
| 59 | T01 | 266 | 0/0/266/0 (rows46-60) | 266 NotSupported refusal | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 60 | T02 | 129 | 117/0/12/0 (rows46-60) | 12 NotSupported refusal | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 61 | T03 | 134 | 0/122/12/0 (rows46-60) | 12 NotSupported refusal | **rerun**: texture.filtering.2d_array 122 Fail: 2D-array look-ups | `8ae0a44b` in `31a4cf8d` in | not run |
| 62 | T04 | 124 | 112/0/12/0 (rows46-60) | 12 NotSupported refusal | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 63 | F3C1 | 14 | 0/14/0/0 (rows46-60) | 0 | **rerun**: texture.filtering.3d: 3D look-ups | `8ae0a44b` in `31a4cf8d` in | not run |
| 64 | F3C2 | 14 | 0/14/0/0 (rows46-60) | 0 | **rerun**: as F3C1 | `8ae0a44b` in `31a4cf8d` in | not run |
| 65 | F3C3 | 14 | 0/14/0/0 (rows46-60) | 0 | **rerun**: as F3C1 | `8ae0a44b` in `31a4cf8d` in | not run |
| 66 | F3C4 | 14 | 0/14/0/0 (rows46-60) | 0 | **rerun**: as F3C1 | `8ae0a44b` in `31a4cf8d` in | not run |
| 67 | F3F1 | 15 | 0/15/0/0 (rows61-75) | 0 | **rerun**: as F3C1 | `8ae0a44b` in `31a4cf8d` in | not run |
| 68 | F3F2 | 15 | 0/15/0/0 (rows61-75) | 0 | **rerun**: as F3C1 | `8ae0a44b` in `31a4cf8d` in | not run |
| 69 | F3F3 | 15 | 0/15/0/0 (rows61-75) | 0 | **rerun**: as F3C1 | `8ae0a44b` in `31a4cf8d` in | not run |
| 70 | F3F4 | 15 | 0/14/1/0 (rows61-75) | 1 NotSupported refusal | **rerun**: as F3C1 | `8ae0a44b` in `31a4cf8d` in | not run |
| 71 | F3F5 | 11 | 0/0/11/0 (rows61-75) | 11 NotSupported refusal | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 72 | F3S1 | 13 | 0/13/0/0 (rows61-75) | 0 | **rerun**: as F3C1 | `8ae0a44b` in `31a4cf8d` in | not run |
| 73 | F3S2 | 13 | 0/13/0/0 (rows61-75) | 0 | **rerun**: as F3C1 | `8ae0a44b` in `31a4cf8d` in | not run |
| 74 | T05a | 196 | 142/34/20/0 (rows61-75) | 20 NotSupported refusal | **rerun**: texture.format.unsized/sized: texture classes (RGB10A2, R/RG 32-bit integer) | `455c3bb5` after `03f4bbd9` in `6c5b004f` in `d7d7357a` in | not run |
| 75 | T05b | 12 | 0/4/8/0 (rows61-75) | 8 NotSupported refusal | **rerun**: as T05a | `455c3bb5` after `03f4bbd9` in `6c5b004f` in `d7d7357a` in | not run |
| 76 | T05c | 4 | 0/0/0/1 (rows61-75) | 4 Fail gl-error | **rerun**: as T05a (the image ended incomplete at b1b5b845) | `455c3bb5` after `03f4bbd9` in `6c5b004f` in `d7d7357a` in | not run |
| 77 | T05d | 8 | 0/0/8/0 (rows61-75) | 8 NotSupported refusal | **rerun**: as T05a | `455c3bb5` after `03f4bbd9` in `6c5b004f` in `d7d7357a` in | not run |
| 78 | T06 | 326 | 106/208/12/0 (rows61-75) | 12 NotSupported refusal | **rerun**: texture.format.sized 208 Fail | `455c3bb5` after `03f4bbd9` in `333a19f7` in `e9f4b8d7` after | not run |
| 79 | T07 | 224 | 140/84/0/0 (rows61-75) | 0 | **rerun**: texture.mipmap.3d/2d/cube | `8ae0a44b` in `21a236e5` in `31a4cf8d` in | not run |
| 80 | T08 | 288 | 0/288/0/0 (rows61-75) | 0 | **rerun**: texture.shadow.2d / 2d_array 288 Fail: the V3D shadow compare | `455c3bb5` after `06bd7bc5` in `c534603b` in `753dedbf` in `333a19f7` in | not run |
| 81 | T09 | 222 | 72/150/0/0 (rows61-75) | 0 | **rerun**: texture.shadow.cube 144 Fail; the six texture.size.2d.2048x2048_* cases are said to stay over the tiled-copy budget (not re-measured) | `455c3bb5` after `06bd7bc5` in `c534603b` in `753dedbf` in | not run |
| 82 | T10 | 334 | 212/122/0/0 (rows61-75) | 0 | **rerun**: texture.specification.basic_teximage3d / texsubimage3d | `8ae0a44b` in `bece11ec` in `31a4cf8d` in | not run |
| 83 | T11 | 320 | 184/136/0/0 (rows61-75) | 0 | **rerun**: texture.specification.teximage3d_* | `8ae0a44b` in `bece11ec` in `31a4cf8d` in | not run |
| 84 | T12 | 197 | 119/78/0/0 (rows61-75) | 0 | **rerun**: texture.specification.texstorage3d | `8ae0a44b` in `bece11ec` in `31a4cf8d` in | not run |
| 85 | T13 | 312 | 180/132/0/0 (rows76-90) | 0 | **rerun**: texture.specification.texsubimage3d_* | `8ae0a44b` in `bece11ec` in `31a4cf8d` in | not run |
| 86 | T14 | 416 | 136/88/0/1 (rows76-90) | 11 ResourceError gl-error | no change expected here: its refusals and files are plan #376 | - | not run |
| 87 | T15 | 432 | 0/0/432/0 (rows76-90) | 432 NotSupported refusal | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 88 | T16 | 504 | 0/0/504/0 (rows76-90) | 504 NotSupported refusal | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 89 | T17 | 504 | 36/0/216/1 (rows76-90) | 360 NotSupported refusal, 72 Fail gl-error | **rerun**: texture.wrap.etc2_eac_*: the ten ETC2/EAC formats; STOPPED EARLY at b1b5b845 | `455c3bb5` after `333a19f7` in `765cbf81` after `e9f4b8d7` after `00366619` after `4e539da8` in | not run |
| 90 | TF1 | 444 | 4/0/0/1 (rows76-90) | 99 Fail gl-error | no change expected here: its refusals and files are plan #376 | - | not run |
| 91 | TF2 | 438 | 11/0/0/1 (rows76-90) | 30 Fail gl-error, 12 NotSupported refusal | no change expected here: its refusals and files are plan #376 | - | not run |
| 92 | UB1 | 968 | 968/0/0/0 (rows76-90) | 0 | no change expected: all Pass at the last record | - | not run |
| 93 | UB2 | 755 | 754/1/0/0 (rows76-90) | 0 | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 94 | UB3 | 632 | 628/4/0/0 (rows76-90) | 0 | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 95 | UA1 | 759 | 758/1/0/0 (rows76-90) | 1 Fail compile | no change expected: the non-Pass cases are refusals or failures no commit of the range moves | - | not run |
| 96 | UA2 | 1014 | 1014/0/0/0 (rows76-90) | 0 | no change expected: all Pass at the last record | - | not run |
| 97 | UA3 | 408 | 408/0/0/0 (rows76-90) | 0 | no change expected: all Pass at the last record | - | not run |
| 98 | VA1 | 203 | 203/0/0/0 (rows76-90) | 0 | no change expected: all Pass at the last record | - | not run |
| 99 | VA2 | 802 | 802/0/0/0 (rows76-90) | 0 | no change expected: all Pass at the last record | - | not run |

## Causes found

- **S32a** (`shaders.texture_functions`, every group but texturesize): 260 refusals at b1b5b845, 32 left at `26d1c4a2`, all `texture_functions.{texturegrad,texturegradoffset,textureprojgrad,textureprojgradoffset}.{sampler3d_fixed,sampler3d_float,isampler3d,usampler3d}_{fragment,vertex}`: dEQP's draw returns `GL_INVALID_OPERATION` (`glsShaderRenderCase.cpp:742`), a refusal of the gradient look-up on a 3D sampler (`docs/es3/shader-host-repro.md` section 1b, RC2, unowned). The lowering (`grad_lod`, `src/shaderc/online/v3d/prepare.rs`) is in files plan #365 holds, so no fix and no host test were made here. Issue draft in `report-drafts.md`.
- **S32c1-4**: the four shadow `textureSize` cases stopped the image at b1b5b845 (a refused draw, `GL_OUT_OF_MEMORY`); `b7c1484e` answers such refusals with `GL_INVALID_OPERATION`, so a case Fails instead of ending the run. Issue draft in `report-drafts.md`.

## Method for the board session that fills the last column

From one origin/main commit: `tools/cts/board-stage.sh pi4 deqp`, then `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <img-dir>`, run with `tools/board.sh pi4` as it is (rows = `TREE_ROWS` above), plus the ES 2.0 regression rows (stage `board-common-pi4`) from the same commit. Records go to `records/pi4/<YYYYMMDD-HHMM>-<name>-<sha8>/` (gitignored, redacted); `cycles.txt`, `table.txt` and a console and tally per image are added to this directory. Board addresses and UART paths stay in the environment.

