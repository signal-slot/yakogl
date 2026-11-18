<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rerun map at ac2fe51f: which b1b5b845 images main has changed (host only)

commit: ac2fe51f52a1060533a63a8d557fe13639a87436

Plan #380, step "the Pi 4's remaining GLES3 images" (issue #1261), host step. This
is the map of `../2026-11-17-pi4-gles3-351fb70d-rerun/`, made again from a host run
at main `ac2fe51f` (2,056 commits after `b1b5b845`). It replaces that record's two
paragraphs that were read from commits rather than run (dc740b98 and e6d22172). It
is named `-map`, like `../2026-11-pi4-gles3-a1b8a613-map/`, because no board ran:
the `-rerun` name belongs to the board session. HOST-ONLY: no board figure here is
new. For each of the 100 images of `tools/cts/pi4-gles3-tree.txt` (42,810 mustpass
cases; `check-gles3-tree.py` matches each once), the verdict is **rerun**,
**no change expected** or **plan #376** (S03-S08, T14, TF1, TF2, left to that
plan's board item).

## Result

**43 images to rerun (11,679 mustpass cases):** A1 FB1 FB2 FB3 FB4 M1 R1 SQ S10 S16
S32a S32c1 S32c2 S32c3 S32c4 S32d T03 F3C1 F3C2 F3C3 F3C4 F3F1 F3F2 F3F3 F3F4 F3S1
F3S2 T05a T05b T05c T05d T06 T07 T08 T09 T10 T11 T12 T13 T17 UB2 UB3 UA1. Tree rows
(0-based) `0,4-7,17,19-20,30,36,52,54-58,61,63-70,72-85,89,93-95`. In these images
2,103 cases that the board did not pass at b1b5b845 (Fail, ResourceError or never
began) now build and draw on the host with no refusal and no API verdict. 190
NotSupported cases now run (176 ETC2/EAC, 14 in R1). 225 board non-Pass cases are
still refused, or get the same API verdict, on the host (the "still" column below).

**48 images: no change expected.** 44 passed in full at b1b5b845, or had only
NotSupported results that the host still reports. 4 had Fails that the host does
not change: FR1 and LT (render verdicts), FR2 (26 `GL_INVALID_ENUM` after
framebuffer setup, as on the board) and S32b (the job builder, below).

**9 images: plan #376.** Their columns are information for that plan, not a verdict.

### What moved since the 351fb70d map

These were measured at ac2fe51f (whole mustpass chunked, then the board non-Pass
cases one process each), not read from commits:

- **S32c1-S32c4: rerun** (080240bc). `textureSize` of `sampler2DShadow` and
  `samplerCubeShadow`, fragment and vertex, is no longer refused by RET16_OF_32.
  All four draw, each in its own process, with no `yakogl` refusal line. On the
  board at b1b5b845, S32c aborted at `texturesize.sampler2dshadow_fragment` and the
  other three never began. Issue draft 2 of the 351fb70d record is done.
- **S32b: no change expected, the cause has moved.** Its two
  `sampler2darrayshadow_{fragment,vertex}` cases are no longer refused by
  RET16_OF_32 either. Like the other 16 3D and 2D-array `textureSize` cases, they
  are now refused by the job builder: `yakogl/v3d: draw refused: the job builder
  refused the draw (a uniform, program or primitive it does not write)`. That is
  issue draft 1, which now covers **50 cases** (S32b 18, S32a 32).
- **FB1: rerun** (c558647b). `fbo.completeness.renderable.{renderbuffer,texture}.color0.r32f`:
  R32F is incomplete without `GL_EXT_color_buffer_float`, as dEQP expects. The 12
  `Framebuffer checked is incomplete, expected complete` verdicts stay.
- **FB4: rerun** (582b201c, c558647b). Six `fbo.invalidate.{sub,whole}.unbind_blit_{color,depth,stencil}`:
  these blits carry depth and stencil, and they no longer get `GL_INVALID_OPERATION`.
  Six `fbo.render.*.tex2d_rgb16f*` cases: RGB16F is now a colour target under
  `GL_EXT_color_buffer_half_float`, so the framebuffer is no longer
  `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`. 78 API verdicts stay.
- **FB2: rerun** (582b201c), as the dc740b98 paragraph of the 351fb70d record said.
  16 `fbo.blit.depth_stencil` cases blit with no error now.
- **FB3:** 54 fixed, against 48 at 351fb70d (c558647b's RGB16F). 72 incomplete-attachment
  verdicts stay.
- **S16 and UA1: rerun** (f2fb64c9). The Pi 4's dEQP images now lend the compiler
  8 MiB of scratch (`driver/samples/zephyr/deqp/boards/rpi_4b.conf`). The host
  library keeps the 4 MiB default, so the host run still fails
  `metamorphic.synthetic.variant_3` and `uniform_api.random.83` with
  `Program(Select(OutOfScratch))`. At 8 MiB both link:
  `tests/es3_v3d_out_of_scratch.rs` now has a row for each
  (`*_links_in_the_pi4_images_8_mib`), which lends the size it reads from the board
  file. f2fb64c9 is also the likeliest board-side fix for UB2's and UB3's scratch
  refusals (`this fragment shader does not fit the scratch arena`). The host builds
  those cases at 4 MiB too, so that was not isolated.
- `GL_EXT_color_buffer_float` (48cb7272) is still not listed on the V3D
  (`color_buffer_float_capable()` is false), so no case moves for it. No board
  non-Pass case outside the ones above moved on the host. So 0b8d04df (stencil
  blits through a depth-stencil texture), 0658692a (RGB10_A2 glCopyTex*) and the
  PowerVR, Mali and GC7000 commits since e6d22172 fix none of them.

### A regression found on main: 4255adf1, four lifetime cases (fixed in this change)

The host run also checked the other direction: cases the board passed at b1b5b845
that the host now refuses or gives an API verdict. Beside the pixel verdicts (the
host reads black) and plan #376's T14 tiled-copy refusals, four cases came up:
`lifetime.attach.deleted_{name,output}.{texture,renderbuffer}_framebuffer`
(image LT). Each fails with `glBindFramebuffer(GL_FRAMEBUFFER, fbo): glGetError()
returned GL_INVALID_FRAMEBUFFER_OPERATION at glsLifetimeTests.cpp:314`, after
`gl_fb_target: framebuffer 2 incomplete (status 0x8cd7)`. They pass at 6efed23e
and fail at 4255adf1 (host libraries of both, the same deqp-gles3).

The cause is in 4255adf1's parked clear (`draw::park_pending_clear`). dEQP clears
fbo 2 fully while a texture is attached, so the clear pends and is not parked.
dEQP binds fbo 2 again and detaches the texture. Fbo 2 now has no attachment,
which `framebuffer::attachments_private_renderbuffers` took as "renderbuffers
only", so the next bind parked the clear. `glDeleteFramebuffers` then flushes
parked clears (`flush_parked_clear`), and the clear went into the empty, incomplete
fbo 2 and raised `GL_INVALID_FRAMEBUFFER_OPERATION`. dEQP read the error at its
next check. Before 4255adf1 the pending clear went quietly with the deleted
framebuffer. The ES 2.0 mustpass has the same cases, and the regression reaches
the stage-1 result as well. With the same host deqp-gles2, 4 of the 8
`dEQP-GLES2.functional.lifetime.attach.*` cases fail on 4255adf1's ES 2.0 V3D
library (`deleted_name` and `deleted_output`, `texture_framebuffer` and
`renderbuffer_framebuffer`), and all 8 pass with the fix
(`MODULE=gles2 tools/cts/host-deqp-run.sh --gpu=v3d lifetime`: 33 of 33 began).
The cause is in the GL core, not the V3D back end, so the other GPUs' builds are
expected to show the same. Only the V3D build was run.

Fix: a framebuffer is parkable only when it is complete
(`attachments_private_renderbuffers` also checks `check_status`). Pinned by
`tests/gl_pending_clear_rebind.rs`
`a_clear_of_a_framebuffer_emptied_since_is_not_kept` (the PowerVR and V3D builds of
the core; it fails at ac2fe51f on `glDeleteFramebuffers(A)`). With the fix, all 58
`dEQP-GLES3.functional.lifetime.*` cases pass on the host. The other rows of that
file, `v3d_terrain_jobs`, `pvr_terrain_kicks` and the `gl::` lib tests are
unchanged. The map above is at ac2fe51f, without this fix. The fix only moves
these four board-Pass cases, so no verdict changes, but the board rerun has to be
built from a main that has the fix. Otherwise LT (and the ES 2.0 regression rows)
lose four cases.

### Still refused on the host

1. **Issue draft 1 (`drafts.md`): the job builder writes no
   QUNIFORM_TEXTURE_DEPTH (46) or QUNIFORM_TEXTURE_ARRAY_SIZE (47) word, 50 cases.**
   S32b's 18 `textureSize` cases of 3D and 2D-array samplers (2D-array shadow
   included), and S32a's 32 `texturegrad*` / `textureprojgrad*` cases of
   `[iu]?sampler3d*`. `tests/es3_texsize3d_v3d.rs` still pins it. The fix is in
   `driver/src/v3d/` (plan #321) and `src/v3d/` (plan #376), so this is a draft,
   not a change. Every one of these draws fails the case, not the run, so S32a and
   S32b run to the end and nothing needs a re-split.
2. T09: 6 2048x2048 textures over the tiled-copy budget and the texture arena, as
   on the board at b1b5b845.
3. API verdicts the host gives the same as the board: FB3 72, FB4 78, FB1 12
   (framebuffer completeness and `GL_INVALID_OPERATION`), S10 15
   (derivate `checkFramebufferStatus`), SQ 6, A1 2, M1 2, FR2 26.

### M1

M1's integer-target clear abort (`pbo.renderbuffer.rg16i_clears`, 62 cases never
began) is the `0ff58329` record, which `docs/es3/pi4-gles3-fails.tsv` still holds.
At b1b5b845 M1 ran to the end and `rg16i_clears` passed
(`../2026-11-pi4-gles3-b1b5b845-rows16-30/M1.console.txt`). Plan #364's
incomplete-framebuffer clear (82dcceba) is on main. M1 is a rerun image for its
5 `negative_api` cases (bc200f21, bece11ec).

## Board

Not run. This worker had no `target:rpi4` / `target:rpi4-console` lease (no
`BOARD_UART` in the environment), so none of the following was done: stage
`deqp-tree-pi4` and `board-stage.sh pi4 deqp`, the 43 images, the ES 2.0 regression
rows (stage `board-common-pi4`), and the per-image `cycles.txt`, `table.txt`,
consoles and tallies. The session that holds the board does it from one main commit
that has the lifetime fix above:

    DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <img-dir> \
        A1 FB1 FB2 FB3 FB4 M1 R1 SQ S10 S16 S32a S32c1 S32c2 S32c3 S32c4 S32d T03 \
        F3C1 F3C2 F3C3 F3C4 F3F1 F3F2 F3F3 F3F4 F3S1 F3S2 T05a T05b T05c T05d T06 T07 \
        T08 T09 T10 T11 T12 T13 T17 UB2 UB3 UA1

Run them with `tools/board.sh pi4` the way the b1b5b845 session READMEs describe,
and write `<date>-pi4-gles3-<sha8>-rerun/README.md`, shaped like
`../2026-11-pi4-gles3-b1b5b845-rows16-30/README.md`. Expect S32a 32 and S32b 18 to
stay Fail until issue draft 1 is fixed. The S32c split can be merged back once the
board confirms the four cases. The tree is unchanged here.

## Method

Host, at ac2fe51f, with VK-GL-CTS `93bca01861b0` (the deps.toml `DEQP_WORK` recipe:
clone, checkout, `external/fetch_sources.py`) and a private copy with
`tools/cts/cts-yakogl-target.patch`, as `cts-build-zephyr.sh` step 1 makes:

    tools/cts/host-deqp-run.sh --gpu=v3d --module=gles3 --cts=<copy> --jobs=4
    host-deqp-run: 42810 of 42810 cases began; process exits (a resumed chunk has several):
         69 exit=0
        372 exit=1

Only chunks c_344 and c_345 (`texture.units`, T14, plan #376) needed a second
process, as at 351fb70d. Then come steps 2 and 3 of
`../2026-11-17-pi4-gles3-351fb70d-rerun/method.md`, its `rerun-map.py` and
`percase.sh` unchanged: the same 2,763 board non-Pass cases outside plan #376, one
process each (2,763 of 2,763 logged).

    rerun-map: 100 images, 42810 mustpass; no change expected 50, plan #376 9, rerun 41

S16 and UA1 are then set to rerun by hand: the host lends 4 MiB, the image 8 MiB
(above). That gives 43 / 48 / 9. In the table, "fixed" is `fixed + ns_changed` of
`hostmap.tsv`, and "still" is `h_refused + h_api`. The `pi4-gles3-fails.tsv` block
of `method.md` comes out the same, 7,110 rows. Its input is the b1b5b845 consoles
alone.

The other direction (board Pass, host refusal or API verdict) is a pass over the
same consoles and host run with `rerun-map.py`'s `parse_stream`, `parse_qpa` and
`klass`. Its API class also counts pixel messages that `RENDER` does not list
("Got faulty pixels", "Buffer verification failed", ...), so only the
non-pixel ones were followed up: the four lifetime cases above, and T14's three
tiled-copy refusals (plan #376's image, chunked state).

## Per image

"commits" lists the commits since b1b5b845 that removed the board's refusal or
verdict class in that image. For the 351fb70d rows they are that record's column.
The ones added here (FB1, FB2, FB4, S32c1-4) are read from what the commits change
against the host's new result, not bisected. Only 4255adf1 (above) was bisected.
"(new)" marks a verdict that changed since the 351fb70d map.

| image | mustpass | b1b5b845 P / F / N.S. / other / never began | fixed on the host | still refused or API on the host | verdict | commits |
|---|---:|---|---:|---:|---|---|
| A1 | 472 | 461 / 11 / 0 / 0 / 0 | 9 | 2 | rerun | 31127ec8 455c3bb5 6581f22b cb52fae8 |
| B1 | 695 | 695 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| C1 | 602 | 602 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| D1 | 443 | 367 / 0 / 76 / 0 / 0 | - | 0 | no change expected | - |
| FB1 | 580 | 559 / 14 / 7 / 0 / 0 | 2 | 12 | rerun (new) | c558647b |
| FB2 | 610 | 445 / 19 / 146 / 0 / 0 | 16 | 0 | rerun (new) | 582b201c |
| FB3 | 329 | 157 / 136 / 36 / 0 / 0 | 54 | 72 | rerun | 8a5da05e 8ae0a44b |
| FB4 | 509 | 108 / 335 / 66 / 0 / 0 | 12 | 78 | rerun (new) | 582b201c c558647b |
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
| R1 | 303 | 257 / 16 / 26 / 4 / 0 | 28 | 0 | rerun | 8ae0a44b |
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
| S16 | 720 | 719 / 1 / 0 / 0 / 0 | 1 | 0 | rerun (new) | f2fb64c9 (board scratch 8 MiB) |
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
| S32c1 | 1 | 0 / 0 / 0 / 1 / 0 | 1 | 0 | rerun (new) | 080240bc |
| S32c2 | 1 | 0 / 0 / 0 / 0 / 1 | 1 | 0 | rerun (new) | 080240bc |
| S32c3 | 1 | 0 / 0 / 0 / 0 / 1 | 1 | 0 | rerun (new) | 080240bc |
| S32c4 | 1 | 0 / 0 / 0 / 0 / 1 | 1 | 0 | rerun (new) | 080240bc |
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
| UB2 | 755 | 754 / 1 / 0 / 0 / 0 | 1 | 0 | rerun | f2fb64c9 (board scratch 8 MiB; host builds it at 4 MiB too) |
| UB3 | 632 | 628 / 4 / 0 / 0 / 0 | 4 | 0 | rerun | f2fb64c9 (board scratch 8 MiB; host builds it at 4 MiB too) |
| UA1 | 759 | 758 / 1 / 0 / 0 / 0 | 1 | 0 | rerun (new) | f2fb64c9 (board scratch 8 MiB) |
| UA2 | 1014 | 1014 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| UA3 | 408 | 408 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| VA1 | 203 | 203 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |
| VA2 | 802 | 802 / 0 / 0 / 0 / 0 | - | 0 | no change expected | - |

## Files

- `README.md`: this map.
- `drafts.md`: the #1261 comment, issue draft 1 (updated to 50 cases), the note on
  `docs/es3/pi4-gles3-fails.tsv`, and the RESULTS.md, STATUS.md, ROADMAP.md and
  `docs/es3-cts-failures.md` lines for the owner.

The scripts and the commands that rebuild `failmap.tsv` and `hostmap.tsv` are in
`../2026-11-17-pi4-gles3-351fb70d-rerun/method.md`. Point `--host` at the ac2fe51f run.
