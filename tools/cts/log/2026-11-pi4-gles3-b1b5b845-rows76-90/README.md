<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 images T13-VA2 (rows 76-90) at b1b5b845

commit b1b5b8456ec69247c64d1bd07e42461266dc5136

Run 2026-11-06 22:35-23:55 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr
af17c0c8, tryboot), item pi4-s6 of plan #320 (issue #1261), session 6 of
the one-commit Pi 4 dEQP-GLES3 record. The fifteen images are T13 through
VA2, the last fifteen labels (rows 76-90, counted from 0) of
`tools/cts/pi4-gles3-tree.txt`, run in the table's order. Everything was
built from a detached worktree of `b1b5b845` outside the repository: the
gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (VK-GL-CTS
93bca01, Zephyr SDK 1.0.1; 453 ninja steps in 248 s on 4 cores), then the
images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` (section 3):
15 `rc=0 build=b1b5b845` lines, each image 161,844-161,848 KB of the
256 MiB window (61.74 %), each `.config` passed by `check-deqp-images.py
--config --gles3`, each passed by `tools/board.sh pi4 run --dry-run` (the
AArch64 header, the window, the watchdog marker).

## Result

8,591 mustpass cases in the fifteen images; 7,284 with a result: **5,902
pass, 226 fail, 1,152 NotSupported, 4 ResourceError, 1,307 never began**
(dEQP aborted four images, T14, T17, TF1 and TF2). 0 GPU recoveries, 0
MMU faults, no job hung or timed out, no reset before the end of any run,
no image near its window (the longest run 52.0 s of about 200 s). Per
image (`<LABEL>.tally.txt`; "begun" is the image's own counter over its
whole filter, the other columns are the mustpass view; "run" is the
record's `<n> cases in <ms> ms` line):

| row | image | mustpass | begun | pass | fail | n.s. | other | none | run | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| 76 | T13 | 312 | 312 | 180 | 132 | 0 | 0 | 0 | 20.5 s | DONE |
| 77 | T14 | 416 | 225 | 136 | 88 | 0 | 1 | 191 | 19.2 s | STOPPED EARLY by dEQP at #224 |
| 78 | T15 | 432 | 432 | 0 | 0 | 432 | 0 | 0 | 14.0 s | DONE, none failed |
| 79 | T16 | 504 | 504 | 0 | 0 | 504 | 0 | 0 | 16.1 s | DONE, none failed |
| 80 | T17 | 504 | 253 | 36 | 0 | 216 | 1 | 251 | 10.4 s | STOPPED EARLY by dEQP at #252 |
| 81 | TF1 | 444 | 5 | 4 | 0 | 0 | 1 | 439 | 0.4 s | STOPPED EARLY by dEQP at #4 |
| 82 | TF2 | 438 | 12 | 11 | 0 | 0 | 1 | 426 | 0.8 s | STOPPED EARLY by dEQP at #11 |
| 83 | UB1 | 968 | 968 | 968 | 0 | 0 | 0 | 0 | 31.4 s | DONE, all passed |
| 84 | UB2 | 755 | 757 | 754 | 1 | 0 | 0 | 0 | 39.2 s | DONE |
| 85 | UB3 | 632 | 632 | 628 | 4 | 0 | 0 | 0 | 52.0 s | DONE |
| 86 | UA1 | 759 | 760 | 758 | 1 | 0 | 0 | 0 | 31.0 s | DONE |
| 87 | UA2 | 1014 | 1014 | 1014 | 0 | 0 | 0 | 0 | 42.7 s | DONE, all passed |
| 88 | UA3 | 408 | 408 | 408 | 0 | 0 | 0 | 0 | 14.8 s | DONE, all passed |
| 89 | VA1 | 203 | 203 | 203 | 0 | 0 | 0 | 0 | 20.7 s | DONE, all passed |
| 90 | VA2 | 802 | 802 | 802 | 0 | 0 | 0 | 0 | 50.7 s | DONE, all passed |
| | total | 8591 | 7287 | 5902 | 226 | 1152 | 4 | 1307 | | |

"begun" above "mustpass" (UB2, UA1) is the filter matching cases outside
the mustpass list (one of UA1's two fails, `uniform_api.random.23`, is one
of them); they are left out of the other columns. "other" is the
ResourceError of each aborted image; "none" is the cases that never
began. `table.txt` is the `--table` view over the fifteen consoles (the
same TOTAL row, then every non-Pass case by name). Linux came back
296-307 s after each reboot (`cycles.txt`).

No image overran or hung, so nothing was re-split (recipe section 6):
`tools/cts/pi4-gles3-tree.txt` is unchanged and `check-gles3-tree.py`
still ends with `91 images, 42810 matched once, 0 not matched, 0 matched
more than once, 0 empty images`.

### The four aborted images

Each ended on its first ResourceError: the V3D back end refused a draw,
the GL core reported `GL_OUT_OF_MEMORY`, dEQP recorded a ResourceError
and ended the run (`Test run was ABORTED!`, `complete=0`). The cases
after it never began; they are not a result.

- T14 at case #224 `texture.units.8_units.only_cube.1`
  (`es3fTextureUnitTests.cpp:1184`): `yakogl/v3d: no GPU memory for a
  tiled copy of 1576960 bytes (13713408 of 16777216 bytes of copies live,
  a job open)`, the retry in a new job failed the same way, then `draw
  refused: no GPU memory for the texture's tiled copy ..., and it is
  larger than the texture arena (8 MiB tiled)`. 191 cases never began:
  the rest of `texture.units.*` and all of `texture.vertex.*`.
- T17 at case #252 `texture.wrap.etc2_eac_rgba8.clamp_clamp_linear_npot`
  (`es3fTextureWrapTests.cpp:371`): `a texel format the V3D does not sample
  (ETC2, or the RGB10A2 class the GL core keeps in RGBA8 here)`. The GL core
  accepts the ETC2 upload (the EAC formats are NotSupported at
  `gluTexture.cpp:142`, `etc1` passes 36/36), the draw is refused. 251
  cases never began: the rest of `etc2_eac_rgba8.*`, then
  `etc2_eac_srgb8_alpha8`, `etc2_rgb8`, `etc2_rgb8_punchthrough_alpha1`,
  `etc2_srgb8`, `etc2_srgb8_punchthrough_alpha1` and `rgba8`.
- TF1 at case #4 `transform_feedback.array.interleaved.lines.highp_ivec4`
  and TF2 at case #11
  `transform_feedback.basic_types.interleaved.lines.highp_mat4`
  (`es3fTransformFeedbackTests.cpp:1290`): `the job builder refused the
  draw (a uniform, program or primitive it does not write)`. The smaller
  interleaved captures before them pass (`highp_ivec3`, `highp_mat3x4`).
  439 + 426 cases never began: nearly all of `transform_feedback.*`.

### The other non-Pass cases, by cause

- `texture.specification` (T13), 132 Fail: 129 "Compile failed"
  (`Program(NotYet("a texture look-up other than 2D or cube"))`, every
  `texsubimage3d_pbo.*` (110), `texsubimage3d_unpack_params.*` (14) and the
  five `texsubimage3d_depth.*_2d_array`) and 3 "Image comparison failed"
  (`texsubimage2d_pbo.rgba32f_2d`, `texsubimage2d_pbo.rgba32f_cube`,
  `texsubimage2d_unpack_params.rgba32f_complex`).
- `texture.units` (T14), 88 Fail: 86 the same 3D / 2D-array look-up
  refusal (`*_units.only_3d`, `*_units.only_2d_array`, `*_units.mixed`)
  and 2 "Image comparison failed" (`4_units.mixed.7`, `8_units.only_2d.7`).
  `texture.swizzle.*` passes in full.
- `texture.wrap` (T15, T16, T17), 1,152 NotSupported: 1,008 "ASTC not
  supported" (every `astc_*`), 144 "Compressed texture format not
  supported" (`eac_r11`, `eac_rg11`, `eac_signed_r11`, `eac_signed_rg11`).
- `ubo` (UB2 1, UB3 4) and `uniform_api` (UA1 1 in the mustpass), 6 Fail:
  the online shader compiler ran out of its scratch arena
  (`raise shaderc::online::store::SCRATCH_BYTES`, or
  `Program(Sched(OutOfScratch))` / `Program(Select(OutOfScratch))`):
  `ubo.random.all_per_block_buffers.28`,
  `ubo.single_struct_array.{per_block_buffer,single_buffer}.{shared,std140}_instance_array_both`,
  `uniform_api.random.83` (and `uniform_api.random.23`, outside the
  mustpass).

Groups that pass in full: `ubo.single_basic_type` 968,
`uniform_api.value.assigned` 1,014, `uniform_api.value.initial` 408,
`uniform_api.info_query`, `vertex_arrays` 1,005 (VA1, VA2),
`texture.swizzle`, `texture.wrap.etc1`.

## Method

The board runner could not capture the console from this task machine,
for the reason the 2026-10-31 record (`2026-10-31-pi4-gles3-a`) gives: the
Pi 4's UART bridge takes one client at a time and the leased serial console
(the MCP server connected forwards) held it (`tools/am62p_netboot_uart.py
probe` answered `busy`). The cycles ran `tools/board.sh`'s pi4 steps by
hand, in its order, one image at a time:

1. Before the first image `/boot/firmware` held a `yakogl-S02.bin` and a
   `tryboot.txt` naming it, left by an earlier session that had not cleaned
   up (Linux was up on the normal configuration). Both were removed;
   `config.txt` was not touched.
2. Prechecks over ssh: `sudo -n`, boot_id, no GL client running,
   `/boot/firmware` free space at least twice the image, no leftover
   `yakogl-*.bin`, `tryboot.txt` or `config-linux.txt`.
3. Staging as `pi4_stage`: the image to `/boot/firmware/yakogl-<LABEL>.bin`
   through `mktemp -d` and `install -m 0755`, `tryboot.txt` with
   `pi4_config`'s lines, both read back (sha256 and text) before the reboot.
4. `sudo reboot "0 tryboot"`; the console read through the leased serial
   console from the offset before the reboot (`serial_expect` on the end
   line `v3d_deqp: build <sha8> (DONE|INCOMPLETE|STOPPED)`, then
   `serial_read` of the whole range, written to disk by a script without a
   copy by hand). The bound per image was the image's own 240 s lifetime
   and PM watchdog, and 900 s for Linux to come back.
5. Linux back (a new boot_id), then `pi4_undo`: the staged image and
   `tryboot.txt` removed and read back. Every cycle read back clean.

One GL client at a time: each image ran alone, the next one staged only
after Linux was back and the previous one's files were gone.

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the
tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first
  line of its boot to the `v3d_deqp: build b1b5b845 ...` end line, CRs and
  NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: run from inside this directory,
  `python3 ../../deqp-uart-tally.py --mustpass ../../gles3-mustpass.txt
  --label <LABEL> <LABEL>.console.txt`.
- `table.txt`: `--table` over the fifteen consoles (its per-image counter
  lines carry the directory name, in the table's order T13 ... VA2).
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the boot_id before,
  the reboot time, the seconds until Linux was back, the clean-up
  read-back, the `cases in` line, the jobs line and the end line.
