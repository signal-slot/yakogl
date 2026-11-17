<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Drafts for the owner: plan #380, the Pi 4's remaining GLES3 images (host step)

Nothing below is posted or applied: agents do not run `gh`, and RESULTS.md,
STATUS.md, ROADMAP.md (plan #321), `docs/es3/pi4-gles3-fails.tsv` (plan #320)
and `docs/es3-cts-failures.md` (plan #372) are other plans' files.

## Class to commit (b1b5b845..684ef106)

| board class at b1b5b845 | commits that removed it |
|---|---|
| `NotYet("a texture look-up other than 2D or cube")` (texelFetch, offsets, 2D-array and 3D look-ups) | 8ae0a44b |
| `NotYet("textureSize of a 3D or array sampler")` | 8ae0a44b (the compile; the draw is refused now, issue draft 1) |
| `NotYet("a texture look-up with gradients")`, vertex-stage 32-bit look-ups | 79b98f34 (3D samplers: issue draft 1) |
| `NotYet("a shadow look-up")`, the draw-time sampler compare | 06bd7bc5, 455c3bb5, 84d8c388 |
| `NotYet("gl_FragDepth")` | 8a5da05e (de737051) |
| `a texel format the V3D does not sample` (ETC2/EAC, RGB10A2), the `not recorded` draws after it, T17's and T05c's 254 cases that never began, the ETC2 NotSupported | 333a19f7, 455c3bb5 (plan #354), 03f4bbd9 |
| a uniform name of 63 characters (`long_instance_name`) | 06840143, 83644bb2, eceaabe6 |
| implementation_limits / state_query values (varyings, combined uniform components, block size, compressed formats) | 6581f22b, cb52fae8, 31127ec8, 455c3bb5 |
| negative_api `Got invalid error` (uniformui, texsubimage) | bc200f21, bece11ec |
| `this fragment shader does not fit the scratch arena` (UB2 1, UB3 4) | not isolated: the host builds them at 684ef106; `SCRATCH_BYTES` is unchanged since b1b5b845 |

## #1261 comment

> Plan #380, host step of "the Pi 4's remaining GLES3 images", main `684ef106`.
> Record: `tools/cts/log/2026-11-17-pi4-gles3-684ef106-rerun/` (README, `hostmap.tsv`
> one row per image, `failmap.tsv` every case the Pi 4 did not pass at b1b5b845 with
> its board and host class, `host-run.log`, `host-s32c.log`). Host run: whole
> mustpass on `GPU=v3d MODULE=gles3`, 42,810 of 42,810 began; the 2,763 board
> non-Pass cases outside #376 again one process per case.
>
> Rerun (34 images, 8,497 cases): A1 FB3 M1 R1 SQ S10 S32a S32d T03 F3C1-4 F3F1-4
> F3S1-2 T05a-d T06-T13 T17 UB2 UB3. Before (b1b5b845) / host now:
>
> | image | b1b5b845 non-Pass | fixed on the host | still |
> |---|---|---:|---:|
> | T03, F3C1-4, F3F1-4, F3S1-2, T06, T10-T13 | 939 F (3D / 2D-array look-ups) | 927 | 0 |
> | T08, T09 | 438 F (shadow, 2D-array) | 432 | 6 (2048x2048 over the tiled-copy budget, as before) |
> | S32a | 260 F | 228 | 32 (3D textureGrad: uniform 46 not written) |
> | T17, T05a-d | 38 F, 2 other, 254 never began, 176 n/s | 292 + 176 n/s | 0 |
> | T07, R1, FB3, S10 | 321 F, 4 QualityWarning | 141 | 93 (FB incomplete, derivate FBO) |
> | A1, SQ, M1 | 48 F | 34 | 10 |
> | S32d, UB2, UB3 | 7 F | 7 | 0 |
> | total | | 2,061 + 176 n/s | 141 |
>
> No change expected: 57 images (FB1 FB2 FB4 FR1 FR2 LT S16 UA1 fail as before on
> the host; S32b and S32c1-4 are still refused, two new issues below). #376's
> images (S03-S08, T14, TF1, TF2) are left to it. M1's `rg16i_clears` abort is the
> 0ff58329 record: at b1b5b845 M1 completed. The board rerun is not done yet (no
> board lease on the worker).

## Issue draft 1: V3D: the job builder writes no texture depth or layer-count uniform

> **V3D: draws reading QUNIFORM_TEXTURE_DEPTH / _ARRAY_SIZE are refused by the job builder (48 dEQP-GLES3 cases)**
>
> Since 8ae0a44b the V3D compiler builds `textureSize` of a 3D or 2D-array sampler
> (Mesa's `ntq_emit_txs`: lane 2 is QUNIFORM_TEXTURE_DEPTH, 46, or
> QUNIFORM_TEXTURE_ARRAY_SIZE, 47), and since 79b98f34 a `textureGrad` on a 3D
> sampler computes its level from the same words (`prepare.rs` `grad_lod`). The
> uniform writer of `driver/src/v3d/v3d_job.c` knows 40/41 (TMU P0/P1) and 44/45
> (width/height) and refuses any other word (`default: cl->bad_field = true`), so
> each such draw fails: `yakogl/v3d: draw refused: the job builder refused the draw (a
> uniform, program or primitive it does not write)`, `GL_INVALID_OPERATION`.
>
> Cases (host at 684ef106, one process each, `host-s32c.log`):
> `shaders.texture_functions.texturesize.{,i,u}sampler{3d,2darray}*` (16, image S32b)
> and `shaders.texture_functions.{texturegrad,texturegradoffset,textureprojgrad,textureprojgradoffset}.{,i,u}sampler3d*`
> (32, image S32a).
>
> Fix: `struct v3d_job_tmu` (and `V3dJobTmu` in `src/v3d/ffi.rs`) gets the base
> level's depth and the layer count; `v3d_job.c` writes 46 (depth, not minified) and
> 47 (layers) as Mesa's `v3d_uniforms.c` does; `src/v3d/backend.rs` `texture_units`
> also gives a TMU record to a unit only words 46 / 47 name. Files held by plan #321
> (`driver/src/v3d/`) and #376 (`backend.rs`). Test: a host test that draws
> `textureSize(sampler3D, 0)` and `textureGrad` on a 3D and a 2D-array texture and
> pins the words the job wrote. Then rerun S32a and S32b on the Pi 4.

## Issue draft 2: V3D: a depth texture only textureSize reads is refused as a 16-bit look-up

> **V3D: textureSize of a shadow sampler is refused (RET16_OF_32), 6 dEQP-GLES3 cases**
>
> `shaders.texture_functions.texturesize.{sampler2dshadow,samplercubeshadow,sampler2darrayshadow}_{fragment,vertex}`
> (images S32c1-4 and S32b): `yakogl/v3d: draw refused: a texture of 32-bit channels
> (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up compiled for the 16-bit TMU
> output`, on the host at 684ef106 for all six (`host-s32c.log`). Cause, as 361fdb4e
> wrote: `src/v3d/backend.rs` `texture_units` takes each unit's return size from the P1
> words of the streams; a unit that only textureSize reads has no P1 word, so it is
> checked as a 16-bit look-up and the R32F depth texture is refused. A unit with no
> P0/P1 word is never sampled: its record can be made for the format's own return
> size (or skipped, with the size words given). Held by plan #376 (backend.rs). Since
> b7c1484e the refusal is GL_INVALID_OPERATION, so the case fails instead of stopping
> dEQP's run (the board's abort at b1b5b845 was the earlier GL_OUT_OF_MEMORY).
> (If an issue for this already exists from 361fdb4e, add this as a comment instead.)

## Replacement block for docs/es3/pi4-gles3-fails.tsv

`fails-b1b5b845.tsv` in this directory: every image from its b1b5b845 console
(7,110 rows), replacing the 0ff58329, 8990a5cc and 009f62a5 rows the file still
has (M1's 62 NoResult rows of the 0ff58329 abort among them). Its header lines go
in place of the file's first four. The 34 rerun images' rows change again with the
board rerun.

## RESULTS.md (Pi 4 GLES3 paragraph, an added sentence)

> Host map at `684ef106` (plan #380,
> `tools/cts/log/2026-11-17-pi4-gles3-684ef106-rerun/`): of the b1b5b845 images, 34
> (8,497 cases) carry 2,061 non-Pass cases main has fixed on the host (3D and
> 2D-array look-ups, shadow look-ups, textureGrad, gl_FragDepth, ETC2/EAC, long
> names) and 176 ETC2 cases no longer NotSupported; they are the next board
> rerun. 48 cases of S32a/S32b are still refused at the draw (texture depth and
> layer-count uniforms) and 6 shadow textureSize cases by RET16_OF_32. HOST-ONLY;
> the board figures above are unchanged.

## STATUS.md (段階 4 の Pi 4 の行に追加)

> Pi 4 の GLES3: b1b5b845 の結果をホスト（main 684ef106）と照合した（計画 #380、
> tools/cts/log/2026-11-17-pi4-gles3-684ef106-rerun/）。再実行が要るのは 34 画像
> （必須 8,497 件）で、ボードで不合格だった 2,061 件がホストでは拒否も API の誤りも
> 出なくなった（3D・2D 配列のサンプル、シャドウ、textureGrad、gl_FragDepth、
> ETC2/EAC、長い名前）。ETC2 の未対応 176 件も実行されるようになった。残る拒否は
> 2 つ: 3D・2D 配列の textureSize と 3D の textureGrad の描画がジョブビルダーに
> 拒否される 48 件（uniform 46/47 を書かない、v3d_job.c）と、シャドウの
> textureSize 6 件（RET16_OF_32）。ボードでの再実行はまだ。

## ROADMAP.md stage 4 (the Pi 4 bullet, an added sentence)

> The b1b5b845 images against main `684ef106` on the host (plan #380): 34 images
> to rerun on the board, 2,061 board non-Pass cases fixed on the host; two causes
> left (texture depth / layer-count uniforms in the job builder, 48 cases; RET16_OF_32
> for shadow textureSize, 6).

## docs/es3-cts-failures.md (RC2 and RC5, for plan #372's file)

> RC2, the V3D sentence "the compiler's refusals of texelFetch, offsets, 2D-array/3D
> look-ups, textureGrad, ... are all gone": add "on the host at 684ef106 all but two
> draws: textureSize of a 3D or 2D-array sampler and textureGrad on a 3D sampler
> compile, and their draw is refused by the job builder, which writes no
> QUNIFORM_TEXTURE_DEPTH / _ARRAY_SIZE (48 cases, `tools/cts/log/2026-11-17-pi4-gles3-684ef106-rerun/`);
> textureSize of a shadow sampler is refused by RET16_OF_32 (6 cases)."
>
> RC5, after "the Pi 4 run is pending": "The host map at 684ef106 has every
> `texture.shadow` (T08) and `texture.filtering`/`mipmap` shadow case of the Pi 4
> images building and drawing without a refusal (432 of 438 in T08/T09; the 6 left
> are 2048x2048 textures over the tiled-copy budget)."
