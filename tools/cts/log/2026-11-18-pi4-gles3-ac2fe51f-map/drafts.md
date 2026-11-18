<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Drafts for the owner: plan #380, the Pi 4's remaining GLES3 images (host map at ac2fe51f)

Nothing below has been posted or applied. Agents do not run `gh`, and these files
belong to other plans: RESULTS.md, STATUS.md and ROADMAP.md (plan #321),
`docs/es3/pi4-gles3-fails.tsv` (plan #320) and `docs/es3-cts-failures.md`
(plan #372). The class-to-commit table of `../2026-11-17-pi4-gles3-351fb70d-rerun/drafts.md`
still holds. This adds 080240bc (RET16_OF_32 for a unit only textureSize reads:
S32c1-4), c558647b (RGB16F and R32F completeness: FB1, FB3, FB4), 582b201c
(depth/stencil blits: FB2, FB4) and f2fb64c9 (8 MiB compiler scratch in the Pi 4
images: S16, UA1, likely UB2 and UB3).

## #1261 comment

> Plan #380, host step of "the Pi 4's remaining GLES3 images", main `ac2fe51f`.
> Record: `tools/cts/log/2026-11-18-pi4-gles3-ac2fe51f-map/` (map and per-image
> table). The method is `../2026-11-17-pi4-gles3-351fb70d-rerun/method.md`
> unchanged. Host run: the whole mustpass with `GPU=v3d MODULE=gles3`, 42,810 of
> 42,810 began. Then the 2,763 board non-Pass cases outside #376 ran one process
> each. **Board: not run** (no target:rpi4 lease on the worker).
>
> Rerun (43 images, 11,679 cases): A1 FB1 FB2 FB3 FB4 M1 R1 SQ S10 S16 S32a S32c1-4
> S32d T03 F3C1-4 F3F1-4 F3S1-2 T05a-d T06-T13 T17 UB2 UB3 UA1. Before (b1b5b845)
> and on the host now:
>
> | image | b1b5b845 non-Pass | fixed on the host | still |
> |---|---|---:|---:|
> | T03, F3C1-4, F3F1-4, F3S1-2, T06, T10-T13 | 939 F (3D / 2D-array look-ups) | 927 | 0 |
> | T08, T09 | 438 F (shadow, 2D-array) | 432 | 6 (2048x2048 over the tiled-copy budget, as before) |
> | S32a | 260 F | 228 | 32 (3D textureGrad: uniform 46 not written) |
> | S32c1-4 | 1 ResourceError, 3 never began (RET16_OF_32 abort) | 4 | 0 |
> | T17, T05a-d | 38 F, 2 other, 254 never began, 176 NotSupported | 292 + 176 NotSupported | 0 |
> | T07, R1, FB3, S10 | 321 F, 4 QualityWarning | 147 + 14 NotSupported | 87 (FB incomplete, derivate FBO) |
> | FB1, FB2, FB4 | 368 F | 30 | 90 (FB completeness, GL_INVALID_OPERATION) |
> | A1, SQ, M1 | 48 F | 34 | 10 |
> | S32d, UB2, UB3 | 7 F | 7 | 0 |
> | S16, UA1 | 2 F (OutOfScratch at 4 MiB) | 2 (8 MiB, f2fb64c9; pinned on the host) | 0 |
>
> No change expected: 48 images. FR1 FR2 LT fail on the host as they did on the
> board. S32b's 18 refusals are all the job builder's now, not RET16_OF_32
> (080240bc): issue draft 1 below, 50 cases with S32a's 32.
>
> **A regression on main, fixed in this change:** 4255adf1 (parked full clears)
> made `lifetime.attach.deleted_{name,output}.{texture,renderbuffer}_framebuffer`
> fail with GL_INVALID_FRAMEBUFFER_OPERATION, in dEQP-GLES3 (image LT) and in
> dEQP-GLES2 (stage 1, 4 of 8 `lifetime.attach` on the host). A framebuffer
> emptied while its clear pended was parked, and the clear went into it at
> `glDeleteFramebuffers`. Now a framebuffer is parked only when complete
> (`tests/gl_pending_clear_rebind.rs`). The board rerun has to be built from a
> main that has this.

## Issue draft 1 (updated): V3D: the job builder writes no texture depth or layer-count uniform

> **V3D: draws reading QUNIFORM_TEXTURE_DEPTH / _ARRAY_SIZE are refused by the job builder (50 dEQP-GLES3 cases)**
>
> Since 8ae0a44b the V3D compiler builds `textureSize` of a 3D or 2D-array sampler
> (Mesa's `ntq_emit_txs`: lane 2 is QUNIFORM_TEXTURE_DEPTH, 46, or
> QUNIFORM_TEXTURE_ARRAY_SIZE, 47). Since 79b98f34 a `textureGrad` on a 3D sampler
> computes its level from the same words. The uniform writer of
> `driver/src/v3d/v3d_job.c` knows 40/41 (TMU P0/P1) and 44/45 (width/height) and
> refuses any other word (`default: cl->bad_field = true`). So each such draw fails
> with `yakogl/v3d: draw refused: the job builder refused the draw (a uniform,
> program or primitive it does not write)` and `GL_INVALID_OPERATION`.
> `tests/es3_texsize3d_v3d.rs` (32db46e9) pins both halves on main.
>
> Cases (host at ac2fe51f, one process each):
> `shaders.texture_functions.texturesize.{,i,u}sampler{3d,2darray}*` and
> `texturesize.sampler2darrayshadow_{fragment,vertex}` (18, image S32b; the two
> shadow ones were RET16_OF_32 before 080240bc), and
> `shaders.texture_functions.{texturegrad,texturegradoffset,textureprojgrad,textureprojgradoffset}.{,i,u}sampler3d*`
> (32, image S32a).
>
> Fix: `struct v3d_job_tmu` (and `V3dJobTmu` in `src/v3d/ffi.rs`) gets the base
> level's depth and the layer count. `v3d_job.c` writes 46 (depth, not minified)
> and 47 (layers), as Mesa's `v3d_uniforms.c` does. `src/v3d/backend.rs`
> `texture_units` gives a TMU record to a unit that only words 46 / 47 name. The
> files are plan #321's (`driver/src/v3d/`) and #376's (`backend.rs`). Then flip
> the pins of `tests/es3_texsize3d_v3d.rs` and rerun S32a and S32b on the Pi 4.

Issue draft 2 of the 351fb70d record (RET16_OF_32 for shadow textureSize) is done
by 080240bc: close it, or do not file it.

## docs/es3/pi4-gles3-fails.tsv (plan #320)

No replacement block is needed now. The method.md command makes 7,110 rows from
the b1b5b845 consoles, and they are byte for byte the 7,110 rows the file already
has on main (only the four header lines differ in wording). When the board rerun
lands, the rows of the 43 rerun images are replaced from its consoles.

## RESULTS.md (Pi 4 GLES3 paragraph, an added sentence)

> Host map at `ac2fe51f` (plan #380,
> `tools/cts/log/2026-11-18-pi4-gles3-ac2fe51f-map/`): 43 of the b1b5b845 images
> (11,679 cases) carry 2,103 non-Pass cases that main has fixed on the host (3D and
> 2D-array look-ups, shadow look-ups and shadow textureSize, textureGrad,
> gl_FragDepth, ETC2/EAC, long names, depth/stencil blits, RGB16F/R32F
> completeness, the 8 MiB compiler scratch), plus 190 cases that are no longer
> NotSupported. They are the next board rerun. 50 cases of S32a/S32b are still
> refused at the draw (texture depth and layer-count uniforms). HOST-ONLY; the
> board figures above are unchanged.

## STATUS.md (段階 4 の Pi 4 の行に追加)

> Pi 4 の GLES3: b1b5b845 のボード結果を main ac2fe51f のホスト実行と照合した（計画 #380、
> tools/cts/log/2026-11-18-pi4-gles3-ac2fe51f-map/）。ボードで再実行が必要なのは
> 43 画像（必須 11,679 件）。そのうちボードで合格しなかった 2,103 件は、ホストでは拒否も
> API エラーも出なくなった（3D・2D 配列のサンプル、シャドウとシャドウの textureSize、
> textureGrad、gl_FragDepth、ETC2/EAC、長い名前、深度・ステンシルのブリット、
> RGB16F/R32F の完全性、コンパイラのスクラッチ 8 MiB）。未対応だった 190 件も実行される
> ようになった。残る拒否は、3D・2D 配列の textureSize と 3D の textureGrad の描画 50 件で、
> ジョブビルダー（v3d_job.c）が uniform 46/47 を書かないため。また 4255adf1 による
> lifetime.attach 4 件の退行（GLES2 にも同じ 4 件）を見つけ、同じ変更で直した。
> ボードでの再実行はまだ行っていない。

## ROADMAP.md stage 4 (the Pi 4 bullet, an added sentence)

> The b1b5b845 images checked against main `ac2fe51f` on the host (plan #380): 43
> images need a board rerun, and 2,103 board non-Pass cases are fixed on the host.
> One cause is left: the job builder writes no texture depth / layer-count uniforms
> (50 cases).

## docs/es3-cts-failures.md (RC2 and RC5, for plan #372's file)

> RC2, after the V3D sentence "the compiler's refusals of texelFetch, offsets,
> 2D-array/3D look-ups, textureGrad, ... are all gone", add: "On the host at
> ac2fe51f every draw goes through but one class: textureSize of a 3D or 2D-array
> sampler (2D-array shadow included) and textureGrad on a 3D sampler compile, but
> the job builder refuses their draw because it writes no QUNIFORM_TEXTURE_DEPTH /
> _ARRAY_SIZE (50 cases, `tools/cts/log/2026-11-18-pi4-gles3-ac2fe51f-map/`).
> textureSize of a shadow sampler is no longer refused by RET16_OF_32 (080240bc)."
>
> RC5, after "the Pi 4 run is pending", add: "In the host map at ac2fe51f, every
> `texture.shadow` case (T08) and every `texture.filtering`/`mipmap` shadow case of
> the Pi 4 images builds and draws without a refusal: 432 of 438 in T08/T09. The 6
> left are 2048x2048 textures over the tiled-copy budget."
