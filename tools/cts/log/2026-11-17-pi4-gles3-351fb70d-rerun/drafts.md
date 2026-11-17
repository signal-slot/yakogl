<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Drafts for the owner: plan #380, the Pi 4's remaining GLES3 images (host step at 351fb70d)

Nothing below has been posted or applied. Agents do not run `gh`, and these files
belong to other plans: RESULTS.md, STATUS.md and ROADMAP.md (plan #321),
`docs/es3/pi4-gles3-fails.tsv` (plan #320) and `docs/es3-cts-failures.md`
(plan #372).

## Class to commit (b1b5b845..351fb70d)

| board class at b1b5b845 | commits that removed it |
|---|---|
| `NotYet("a texture look-up other than 2D or cube")` (texelFetch, offsets, 2D-array and 3D look-ups) | 8ae0a44b |
| `NotYet("textureSize of a 3D or array sampler")` | 8ae0a44b (the compile only; the draw is now refused, issue draft 1) |
| `NotYet("a texture look-up with gradients")`, vertex-stage 32-bit look-ups | 79b98f34 (except 3D samplers: issue draft 1) |
| `NotYet("a shadow look-up")`, the draw-time sampler compare | 06bd7bc5, 455c3bb5, 84d8c388 |
| `NotYet("gl_FragDepth")` | 8a5da05e |
| `a texel format the V3D does not sample` (ETC2/EAC, RGB10A2), the `not recorded` draws after it, the 254 cases of T17 and T05c that never began, the ETC2 NotSupported | 333a19f7, 455c3bb5 (plan #354), 03f4bbd9 |
| a uniform name of 63 characters (`long_instance_name`) | 06840143, 83644bb2, eceaabe6 |
| implementation_limits / state_query values (varyings, combined uniform components, block size, compressed formats) | 6581f22b, cb52fae8, 31127ec8, 455c3bb5 |
| negative_api `Got invalid error` (uniformui, texsubimage) | bc200f21, bece11ec |
| `this fragment shader does not fit the scratch arena` (UB2 1, UB3 4) | not isolated: the host builds them at 351fb70d, and `SCRATCH_BYTES` is unchanged since b1b5b845 |
| (plan #376) `a target larger than the tile memory is sized for (2048x2048)`, S03-S08 | 6d2e7d0d, host only |

## #1261 comment

> Plan #380, host step of "the Pi 4's remaining GLES3 images", main `351fb70d`.
> Record: `tools/cts/log/2026-11-17-pi4-gles3-351fb70d-rerun/`. `README.md` has
> the map and the per-image table; `method.md` has the scripts and the commands
> that rebuild `failmap.tsv`, `hostmap.tsv` and the `pi4-gles3-fails.tsv` block.
> Host run: the whole mustpass with `GPU=v3d MODULE=gles3`, 42,810 of 42,810
> began. The 2,763 board non-Pass cases outside #376 were run again, one process
> per case.
>
> Rerun (34 images, 8,497 cases): A1 FB3 M1 R1 SQ S10 S32a S32d T03 F3C1-4 F3F1-4
> F3S1-2 T05a-d T06-T13 T17 UB2 UB3. Before (b1b5b845) and on the host now:
>
> | image | b1b5b845 non-Pass | fixed on the host | still |
> |---|---|---:|---:|
> | T03, F3C1-4, F3F1-4, F3S1-2, T06, T10-T13 | 939 F (3D / 2D-array look-ups) | 927 | 0 |
> | T08, T09 | 438 F (shadow, 2D-array) | 432 | 6 (2048x2048 over the tiled-copy budget, as before) |
> | S32a | 260 F | 228 | 32 (3D textureGrad: uniform 46 not written) |
> | T17, T05a-d | 38 F, 2 other, 254 never began, 176 NotSupported | 292 + 176 NotSupported | 0 |
> | T07, R1, FB3, S10 | 321 F, 4 QualityWarning | 141 | 93 (FB incomplete, derivate FBO) |
> | A1, SQ, M1 | 48 F | 34 | 10 |
> | S32d, UB2, UB3 | 7 F | 7 | 0 |
> | total | | 2,061 + 176 NotSupported | 141 |
>
> No change expected: 57 images. FB1 FB2 FB4 FR1 FR2 LT S16 UA1 fail on the host as
> they did on the board. S32b and S32c1-4 are still refused (the two issues below).
> The map is the one made at b1e4c0f5 (closed #7872) and 684ef106 (closed #7704),
> case for case, except in #376's images. There, 6d2e7d0d's 4096 tile memory makes
> the S03-S08 precision cases draw on the host: 1,093 of their 1,147 board non-Pass
> cases. The 54 still refused, in S05, S06 and S08, are the mat4/mat4x2/mat4x3
> cases of matrixcompmult, outerproduct and transpose (`Failed to set up
> framebuffer object: ... GL_INVALID_ENUM`). Those images stay with #376. M1's
> `rg16i_clears` abort is the 0ff58329 record: at b1b5b845 M1 ran to the end. The
> board rerun has not been done yet: the worker had no target:rpi4 lease.

## Issue draft 1: V3D: the job builder writes no texture depth or layer-count uniform

> **V3D: draws reading QUNIFORM_TEXTURE_DEPTH / _ARRAY_SIZE are refused by the job builder (48 dEQP-GLES3 cases)**
>
> Since 8ae0a44b the V3D compiler builds `textureSize` of a 3D or 2D-array sampler
> (Mesa's `ntq_emit_txs`: lane 2 is QUNIFORM_TEXTURE_DEPTH, 46, or
> QUNIFORM_TEXTURE_ARRAY_SIZE, 47). Since 79b98f34 a `textureGrad` on a 3D sampler
> computes its level from the same words (`prepare.rs` `grad_lod`). The uniform
> writer of `driver/src/v3d/v3d_job.c` knows 40/41 (TMU P0/P1) and 44/45
> (width/height) and refuses any other word (`default: cl->bad_field = true`). So
> each such draw fails with `yakogl/v3d: draw refused: the job builder refused the
> draw (a uniform, program or primitive it does not write)` and
> `GL_INVALID_OPERATION`. `tests/es3_texsize3d_v3d.rs` (32db46e9) pins both halves
> on main.
>
> Cases (host at 351fb70d, one process each):
> `shaders.texture_functions.texturesize.{,i,u}sampler{3d,2darray}*` (16, image S32b)
> and `shaders.texture_functions.{texturegrad,texturegradoffset,textureprojgrad,textureprojgradoffset}.{,i,u}sampler3d*`
> (32, image S32a).
>
> Fix: `struct v3d_job_tmu` (and `V3dJobTmu` in `src/v3d/ffi.rs`) gets the base
> level's depth and the layer count. `v3d_job.c` writes 46 (depth, not minified)
> and 47 (layers), as Mesa's `v3d_uniforms.c` does. `src/v3d/backend.rs`
> `texture_units` also gives a TMU record to a unit that only words 46 / 47 name.
> Files held by plan #321 (`driver/src/v3d/`) and #376 (`backend.rs`). Then flip
> the pins of `tests/es3_texsize3d_v3d.rs` and rerun S32a and S32b on the Pi 4.

## Issue draft 2: V3D: a depth texture only textureSize reads is refused as a 16-bit look-up

> **V3D: textureSize of a shadow sampler is refused (RET16_OF_32), 6 dEQP-GLES3 cases**
>
> `shaders.texture_functions.texturesize.{sampler2dshadow,samplercubeshadow,sampler2darrayshadow}_{fragment,vertex}`
> (images S32c1-4 and S32b) are refused on the host at 351fb70d, all six, each in
> its own process: `yakogl/v3d: draw refused: a texture of 32-bit channels (R32F,
> RGBA32F, RGBA32UI, RGBA32I) read by a look-up compiled for the 16-bit TMU output`.
> The cause is the one 361fdb4e wrote: `src/v3d/backend.rs` `texture_units` takes
> each unit's return size from the P1 words of the streams. A unit that only
> textureSize reads has no P1 word, so it is checked as a 16-bit look-up
> (`src/v3d/texture.rs` `refusal_for`, `RET16_OF_32`) and the R32F depth texture
> is refused. A unit with no P0/P1 word is never sampled: its record can take the
> format's own return size, or the unit can be skipped and given only the size
> words. Held by plan #376 (backend.rs). Since b7c1484e the refusal is
> GL_INVALID_OPERATION, so the case fails instead of stopping dEQP's run. The
> board's abort at b1b5b845 was the earlier GL_OUT_OF_MEMORY. Test: a
> `src/v3d/backend_tests.rs` case beside
> `a_program_reading_a_texture_size_gets_the_bindings_size`, with an R32F binding
> that only words 44/45 name. If an issue for this already exists from 361fdb4e,
> add this as a comment there instead.

## Replacement block for docs/es3/pi4-gles3-fails.tsv

Made by the command in `method.md` ("The replacement block"): every image from its
b1b5b845 console, 7,110 rows. It replaces the 0ff58329, 8990a5cc and 009f62a5 rows
the file still has, M1's 62 NoResult rows of the 0ff58329 abort among them. Its
four header lines replace the file's four comment lines. The rows of the 34 rerun
images change again with the board rerun.

## RESULTS.md (Pi 4 GLES3 paragraph, an added sentence)

> Host map at `351fb70d` (plan #380,
> `tools/cts/log/2026-11-17-pi4-gles3-351fb70d-rerun/`): 34 of the b1b5b845 images
> (8,497 cases) carry 2,061 non-Pass cases that main has fixed on the host (3D and
> 2D-array look-ups, shadow look-ups, textureGrad, gl_FragDepth, ETC2/EAC, long
> names), plus 176 ETC2 cases that are no longer NotSupported; they are the next
> board rerun. 48 cases of S32a/S32b are still refused at the draw (texture depth
> and layer-count uniforms), and 6 shadow textureSize cases are refused by
> RET16_OF_32. HOST-ONLY; the board figures above are unchanged.

## STATUS.md (段階 4 の Pi 4 の行に追加)

> Pi 4 の GLES3: b1b5b845 のボード結果を main 351fb70d のホスト実行と照合した（計画 #380、
> tools/cts/log/2026-11-17-pi4-gles3-351fb70d-rerun/）。ボードで再実行が必要なのは
> 34 画像（必須 8,497 件）。そのうちボードで不合格だった 2,061 件は、ホストでは拒否も
> API エラーも出なくなった（3D・2D 配列のサンプル、シャドウ、textureGrad、
> gl_FragDepth、ETC2/EAC、長い名前）。ETC2 の未対応 176 件も実行されるようになった。
> 残る拒否は 2 つ。3D・2D 配列の textureSize と 3D の textureGrad の描画 48 件は、
> ジョブビルダー（v3d_job.c）が uniform 46/47 を書かないため拒否される。シャドウの
> textureSize 6 件は RET16_OF_32 で拒否される。ボードでの再実行はまだ行っていない。

## ROADMAP.md stage 4 (the Pi 4 bullet, an added sentence)

> The b1b5b845 images checked against main `351fb70d` on the host (plan #380): 34
> images need a board rerun, and 2,061 board non-Pass cases are fixed on the host.
> Two causes are left: the job builder writes no texture depth / layer-count
> uniforms (48 cases), and RET16_OF_32 refuses shadow textureSize (6 cases).

## docs/es3-cts-failures.md (RC2 and RC5, for plan #372's file)

> RC2, after the V3D sentence "the compiler's refusals of texelFetch, offsets,
> 2D-array/3D look-ups, textureGrad, ... are all gone", add: "On the host at
> 351fb70d every draw goes through but two: textureSize of a 3D or 2D-array sampler
> and textureGrad on a 3D sampler compile, but the job builder refuses their draw
> because it writes no QUNIFORM_TEXTURE_DEPTH / _ARRAY_SIZE (48 cases,
> `tools/cts/log/2026-11-17-pi4-gles3-351fb70d-rerun/`). textureSize of a shadow
> sampler is refused by RET16_OF_32 (6 cases)."
>
> RC5, after "the Pi 4 run is pending", add: "In the host map at 351fb70d, every
> `texture.shadow` case (T08) and every `texture.filtering`/`mipmap` shadow case of
> the Pi 4 images builds and draws without a refusal: 432 of 438 in T08/T09. The 6
> left are 2048x2048 textures over the tiled-copy budget."
