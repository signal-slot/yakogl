<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Report drafts: Pi 4 dEQP-GLES3 rerun from a1b8a613

Nothing here carries a board number: the board was not run (no `target:rpi4` lease on the worker that made this commit). The README of this directory has the rerun map (38 images, `TREE_ROWS=0,6-7,16-17,19-20,30,52,54-58,61,63-70,72-85,89`). Fill the blocks below after a board session.

## #1261 comment (draft)

> Plan #380, item "the Pi 4's remaining GLES3 images", host side at `a1b8a613`: `tools/cts/log/2026-11-pi4-gles3-a1b8a613-rerun/README.md` maps all 100 images of `pi4-gles3-tree.txt`. 38 are marked rerun (their failing groups sit behind commits of `b1b5b845..a1b8a613`: clears of several attachments `7fa02269`/`3c3e5470`, the incomplete-framebuffer clear `82dcceba`, msaa `f3ed1fea`/`2830b218`/`4bcb55c3`, limits `cb52fae8`/`6581f22b`, ETC2/EAC and shadow `455c3bb5`, 3D and 2D-array look-ups `8ae0a44b`/`31a4cf8d`, `b7c1484e` refusals as `GL_INVALID_OPERATION`); the rest are unchanged or belong to plan #376. No board record: not run (needs target:rpi4 and target:rpi4-console). No new host run either; the host column is the `26d1c4a2` record. Per-image before/after: the README's last column is "not run" until a board session fills it.

## Issue draft: S32a, 3D-sampler gradient look-ups refused

Title: V3D: draw of `textureGrad`/`textureProjGrad` (+Offset) on a 3D sampler answers GL_INVALID_OPERATION

Body: Pi 4 image S32a (`shaders.texture_functions`, all groups but texturesize). 260 refusals at `b1b5b845`; at `26d1c4a2` the host (HOST-ONLY, `docs/es3/host-gles3-notpass.tsv`) still has 32: `texturegrad`, `texturegradoffset`, `textureprojgrad`, `textureprojgradoffset` for `sampler3d_fixed`, `sampler3d_float`, `isampler3d`, `usampler3d`, each `_fragment` and `_vertex`; dEQP reports `draw: glGetError() returned GL_INVALID_OPERATION at glsShaderRenderCase.cpp:742`. Same row as `docs/es3/shader-host-repro.md` section 1b (RC2, unowned). The gradient lowering is `grad_lod` in `src/shaderc/online/v3d/prepare.rs` (design `docs/notes/v3d-qpu-grad.md`), files held by plan #365. The cases Fail the case, not the image; S32a is not re-split. Needs: the refusing text (rerun `MODULE=gles3 tools/cts/host-deqp-run.sh dEQP-GLES3.functional.shaders.texture_functions.texturegrad.sampler3d_float_fragment` in the v3d configuration), a fix, a `tests/es3_*_v3d.rs` pinning it.

## Issue draft: S32c1-4, shadow textureSize cases Fail with GL_INVALID_OPERATION

Title: V3D: `texturesize.sampler2dshadow_*` / `samplercubeshadow_*` draws refused (RET16_OF_32 for a unit no look-up reads)

Body: At `b1b5b845` the first of the four stopped S32c with a refused draw and `GL_OUT_OF_MEMORY` (dEQP ResourceError, the image ended, later cases never began); the tree split them into S32c1-4 (`361fdb4e`). Since `b7c1484e` the refusal answers `GL_INVALID_OPERATION`, so the case is a Fail and the run goes on. The host (per the tree comment) refuses all four, citing a 16-bit-return-of-32 setting for a unit no look-up reads. Needs the refusal text from a host run, the fix, and a board rerun of S32c1-4. Cause not re-measured here.

## docs/es3/pi4-gles3-fails.tsv replacement block

Not generated: it needs the board consoles. After the session, from the record directory: `python3 tools/cts/gles3-notpass.py --pi4 ...` and `tools/cts/pi4-gles3-report.py` (see their `--help`), then replace the rows of the rerun images only (plan #320 holds the file today).

## Draft lines

`tools/cts/RESULTS.md` (row of the image, current results only):
> Pi 4 dEQP-GLES3, images <rerun labels> at <sha8>: not run (needs target:rpi4); host map `tools/cts/log/2026-11-pi4-gles3-a1b8a613-rerun/README.md`.

`tools/cts/STATUS.md` (Japanese):
> Pi 4 dEQP-GLES3: b1b5b845 以降の修正で結果が変わりうる 38 イメージ (TREE_ROWS=0,6-7,16-17,19-20,30,52,54-58,61,63-70,72-85,89) の再実行表を a1b8a613 で作成。実機は未実施 (target:rpi4 が必要)。S32a の 32 件 (3D サンプラの textureGrad 系) は未修正で、イシュー案あり。

`ROADMAP.md` stage 4 line:
> OpenGL ES 3.0 on the Pi 4: rerun map for 38 images from a1b8a613 written, board rerun pending (target:rpi4); S32a 3D-gradient look-ups open.
