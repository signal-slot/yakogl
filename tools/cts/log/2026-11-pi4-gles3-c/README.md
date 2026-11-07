<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 images S17-S32, S5f, S5v, TF, UB, UA, VA at 009f62a5

commit: 009f62a5f8e66357fc22d7d1e708943513c94467

Run 2026-11-07 06:04-08:21 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr
af17c0c8, tryboot), item pi4-deqp-c of plan #308 (issue #2). The 26 images
are the second half of the shader images and the transform-feedback,
uniform-block, uniform-API and vertex-array images of
`tools/cts/pi4-gles3-tree.txt` (S17-S27, S5f, S5v, S30-S32, TF1, TF2,
UB1-UB3, UA1-UA3, VA1, VA2; the table has no S28 or S29). Every image was
built from origin/main `009f62a5` with the gles3 stage 1 of
`docs/es3/board-recipe-pi4.md` section 2 (VK-GL-CTS 93bca01, Zephyr SDK
1.0.1), each 161,844 KB of the 256 MiB window (61.74 %). Every console
prints `v3d_deqp: build 009f62a5` and no other build. All 26 images ran;
none was left unrun.

## Result

17,578 mustpass cases in the 26 images; 16,576 with a result: **16,289
pass, 284 fail, 0 NotSupported, 0 QualityWarning, 3 ResourceError, 1,002
never began** (dEQP aborted three images). 0 GPU recoveries, no job hung,
no image hung, no reset before the end of any run. Per image
(`<LABEL>.tally.txt`; "begun" is the image's own counter over its whole
filter, the other columns are the mustpass view; "run" is the image's
`<n> cases in <ms> ms`, "back" the seconds from the reboot until Linux
answered again):

| image | mustpass | begun | pass | fail | n.s. | QW | other | none | run | back | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|
| S17 | 716 | 720 | 716 | 0 | 0 | 0 | 0 | 0 | 34.3 s | 302 s | DONE, all passed |
| S18 | 864 | 864 | 864 | 0 | 0 | 0 | 0 | 0 | 43.2 s | 301 s | DONE, all passed |
| S19 | 716 | 716 | 716 | 0 | 0 | 0 | 0 | 0 | 34.8 s | 303 s | DONE, all passed |
| S20 | 666 | 666 | 666 | 0 | 0 | 0 | 0 | 0 | 34.2 s | 302 s | DONE, all passed |
| S21 | 680 | 680 | 680 | 0 | 0 | 0 | 0 | 0 | 32.4 s | 319 s | DONE, all passed |
| S22 | 768 | 768 | 768 | 0 | 0 | 0 | 0 | 0 | 38.0 s | 304 s | DONE, all passed |
| S23 | 866 | 866 | 866 | 0 | 0 | 0 | 0 | 0 | 39.0 s | 321 s | DONE, all passed |
| S24 | 1126 | 1198 | 1126 | 0 | 0 | 0 | 0 | 0 | 54.9 s | 303 s | DONE, all passed |
| S25 | 689 | 699 | 689 | 0 | 0 | 0 | 0 | 0 | 26.2 s | 302 s | DONE, all passed |
| S26 | 700 | 700 | 700 | 0 | 0 | 0 | 0 | 0 | 25.4 s | 304 s | DONE, all passed |
| S27 | 987 | 1000 | 987 | 0 | 0 | 0 | 0 | 0 | 35.5 s | 322 s | DONE, all passed |
| S5f | 66 | 100 | 66 | 0 | 0 | 0 | 0 | 0 | 6.6 s | 302 s | DONE, all passed |
| S5v | 70 | 100 | 70 | 0 | 0 | 0 | 0 | 0 | 4.3 s | 301 s | DONE, all passed |
| S30 | 915 | 938 | 915 | 0 | 0 | 0 | 0 | 0 | 38.6 s | 306 s | DONE, all passed |
| S31 | 648 | 648 | 648 | 0 | 0 | 0 | 0 | 0 | 22.6 s | 304 s | DONE, all passed |
| S32 | 678 | 590 | 262 | 278 | 0 | 0 | 1 | 137 | 34.8 s | 320 s | STOPPED EARLY by dEQP at #589 |
| TF1 | 444 | 5 | 4 | 0 | 0 | 0 | 1 | 439 | 0.4 s | 306 s | STOPPED EARLY by dEQP at #4 |
| TF2 | 438 | 12 | 11 | 0 | 0 | 0 | 1 | 426 | 0.8 s | 302 s | STOPPED EARLY by dEQP at #11 |
| UB1 | 968 | 968 | 968 | 0 | 0 | 0 | 0 | 0 | 31.4 s | 305 s | DONE, all passed |
| UB2 | 755 | 757 | 754 | 1 | 0 | 0 | 0 | 0 | 39.2 s | 306 s | DONE |
| UB3 | 632 | 632 | 628 | 4 | 0 | 0 | 0 | 0 | 52.0 s | 307 s | DONE |
| UA1 | 759 | 760 | 758 | 1 | 0 | 0 | 0 | 0 | 31.0 s | 306 s | DONE |
| UA2 | 1014 | 1014 | 1014 | 0 | 0 | 0 | 0 | 0 | 42.7 s | 301 s | DONE, all passed |
| UA3 | 408 | 408 | 408 | 0 | 0 | 0 | 0 | 0 | 14.8 s | 345 s | DONE, all passed |
| VA1 | 203 | 203 | 203 | 0 | 0 | 0 | 0 | 0 | 20.7 s | 305 s | DONE, all passed |
| VA2 | 802 | 802 | 802 | 0 | 0 | 0 | 0 | 0 | 50.7 s | 304 s | DONE, all passed |
| total | 17578 | 16814 | 16289 | 284 | 0 | 0 | 3 | 1002 | | | |

"begun" above "mustpass" (S17, S24, S25, S27, S5f, S5v, S30, UB2, UA1) is
the filter matching cases outside the mustpass list; they are left out of
the other columns. "none" is the mustpass cases of an aborted image that
never began. The `--table` view over all 26 consoles is `table.txt` (the
same TOTAL row, then every non-Pass case by name). The longest run, S24 at
54.9 s, is well inside the 200 s window, so no image needs splitting for
time.

### Why S32, TF1 and TF2 stopped early

No image hung. In each of the three, the V3D back end refused one draw,
the GL core reported `GL_OUT_OF_MEMORY` for it, dEQP recorded a
ResourceError and ended the run (`Test run was ABORTED!`, `complete=0`).
Each console's last case is the one named here:

- S32 at case #589 `shaders.texture_functions.texturesize.sampler2dshadow_fragment`:
  `yakogl/v3d: draw refused: a texture of 32-bit channels (R32F, RGBA32F,
  RGBA32UI, RGBA32I) read by a look-up compiled for the 16-bit TMU output`.
  The rest of `texture_functions.texturesize.*` and everything after it in
  the filter never began (137 mustpass cases without a result).
- TF1 at case #4 `transform_feedback.array.interleaved.lines.highp_ivec4`
  and TF2 at case #11
  `transform_feedback.basic_types.interleaved.lines.highp_mat4`: `yakogl/v3d:
  draw refused: the job builder refused the draw (a uniform, program or
  primitive it does not write)` (`es3fTransformFeedbackTests.cpp:1290`).
  439 and 426 mustpass cases never began. The 15 transform-feedback cases
  that ran before the refusal all passed.

The S32 refusal is RC2 of `docs/es3-cts-failures.md` (the 32-bit sampling
classes). The TF refusal is a V3D job-builder gap for interleaved
transform feedback of wide types (ivec4, mat4) with line primitives; the
host passes these cases (`board-recipe-pi4.md` section 7). Until each is
fixed, the image needs splitting at that case to get results for the cases
behind it.

### The other non-Pass cases, by cause

- `shaders.texture_functions` (S32, 313 Fail in the console, 278 of them in
  the mustpass): the V3D compiler refuses the program, so dEQP reports
  "Failed to compile shaders" (RC2):
  - 206 `Program(NotYet("a texture look-up other than 2D or cube"))`
    (2D-array and 3D samplers, in every function group).
  - 56 `Program(NotYet("a texture look-up with gradients"))` (`texturegrad*`,
    `textureprojgrad*`).
  - 33 `Program(NotYet("a shadow look-up"))`.
  - 10 `Program(NotYet("textureSize of a 3D or array sampler"))`.
  - 8 more Fail "Got unexpected texture size" in `texturesize.*`.
- `ubo` (UB2 1, UB3 4 Fail): the online compiler's scratch arena runs out
  ("scratch arena (0 bytes left); raise
  shaderc::online::store::SCRATCH_BYTES"): `random.all_per_block_buffers.28`
  and `single_struct_array.{per_block_buffer,single_buffer}.{shared,std140}_instance_array_both`.
- `uniform_api` (UA1, 2 Fail in the console, 1 in the mustpass):
  `random.83` `Program(Select(OutOfScratch))` and `random.23` (not in the
  mustpass) `Program(Sched(OutOfScratch))`: the same compiler limit.

Groups that pass in full: `shaders.operator` (S17-S24, 6,402 mustpass
cases), `shaders.precision`, `preprocessor`, `qualification_order` (S25),
`shaders.random` (S26, S27, S5f, S5v), `return`, `scoping`, `struct`,
`switch`, `swizzle_math_operations` (S30), `swizzles` (S31),
`ubo.single_basic_type` (UB1), `uniform_api.value.assigned` (UA2),
`uniform_api.value.initial` (UA3), `vertex_arrays` 1,005 / 1,005 (VA1, VA2).

## Method

The board runner could not capture the console from this task machine:
the Pi 4's UART bridge takes one client at a time, and the leased serial
console (an MCP server that connected forwards) held that slot. As in
`../2026-10-31-pi4-gles3-a/README.md`, this run did `tools/board.sh pi4`'s
steps by hand, in its order:

1. The images: `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh
   tools/cts/pi4-gles3-tree.txt build/deqp/pi4-gles3-009f62a5 <the 26
   labels>` from a clean checkout of origin/main `009f62a5`: 26 lines
   `<LABEL> rc=0 sha256=... build=009f62a5`, every log ending with the
   `--gles3` conformance line.
2. `tools/board.sh pi4 run --dry-run <zephyr.bin> <dir> <LABEL>`: the local
   image checks (AArch64 header, the 256 MiB window, the PM watchdog
   marker `board: watchdog armed`). Every image passed (`rc 0`); nothing was
   bypassed.
3. Prechecks over ssh as `common_prechecks` and `pi4_prechecks`: `sudo -n`,
   boot_id, `who` (no session from another machine; this session's own
   logins arrive through connected's port forward), `/boot/firmware` free
   space at least twice the image, no leftover `yakogl-*.bin`, no
   `tryboot.txt`, no `config-linux.txt`.
4. Staging as `pi4_stage`: the image to `/boot/firmware/yakogl-<LABEL>.bin`
   through `mktemp -d` and `install -m 0755`, `tryboot.txt` written with
   `pi4_config`'s lines (`kernel=`, `arm_64bit=1`, `enable_uart=1`,
   `uart_2ndstage=1`, the panel's `hdmi_group`/`hdmi_mode`/`hdmi_timings`),
   both read back (sha256 and text) before the reboot.
5. `sudo reboot "0 tryboot"` (through `systemd-run`, so the ssh session
   ends cleanly), then the console read through the leased serial console:
   `serial_expect` on the end line `v3d_deqp: build <sha8>
   (DONE|INCOMPLETE|STOPPED)`, bounded by board.sh's 420 s, then
   `serial_read` of the whole run. The replies were saved to disk by the
   tool itself and converted by a script, never copied by hand. Every image
   printed `board: watchdog armed` before Zephyr started; each run ended
   long before its 240 s lifetime, and the PM watchdog reset the board at
   about 250 s.
6. Linux back (a new boot_id, 301-345 s after the reboot), then `pi4_undo`:
   the staged image and `tryboot.txt` removed (there was none before), read
   back. Every cycle read back as before; `config.txt` was not touched, and
   the board boots its normal configuration.

One GL client at a time: each image ran alone, and the next one was staged
only after Linux was back and the previous one's files were gone. The
consoles hold no address: the capture starts at the firmware's first line
after Linux's `reboot: Restarting system` and ends at the image's end line,
before Linux boots again. `BOARD_HOST`, the UART bridge and this machine's
addresses appear nowhere.

## Files

Every `.txt` starts with the three `#` header lines the other log text files
carry (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line
  of its boot to the `v3d_deqp: build 009f62a5 ...` end line (the watchdog
  reset prints nothing after it), CRs and NULs dropped, one line per row,
  no time stamps. Every `deqp:` result line, dEQP's own per-case lines and
  the `v3d_deqp: case #N` lines are there.
- `<LABEL>.tally.txt`: `python3 tools/cts/deqp-uart-tally.py --mustpass
  tools/cts/gles3-mustpass.txt <LABEL>.console.txt`.
- `table.txt`: the same with `--table` over the 26 consoles.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the boot_id before,
  the reboot time, the seconds until Linux was back, the clean-up read-back,
  the `cases in` line and the end line.
