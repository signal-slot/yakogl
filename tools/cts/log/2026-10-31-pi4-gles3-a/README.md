<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 images FO2-FO6, FR1, FR2, LT, M1, PR, R1, SQ at 0ff58329

Run 2026-11-01 19:41-20:47 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr
af17c0c8, tryboot), item pi4-deqp-a of plan #155 (issue #2). The twelve
images are rows 9-20 of `tools/cts/pi4-gles3-tree.txt`, built from
origin/main `0ff58329` (`0ff583297427e87d37ae34e1858a868b849213e9`) with the
gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (VK-GL-CTS
93bca01, Zephyr SDK 1.0.1), each 161,836 KB of the 256 MiB window (61.74 %).
The directory name carries the plan's date; the run is of 2026-11-01.

## Result

5,749 mustpass cases in the twelve images; 5,650 with a result: **5,343
pass, 97 fail, 204 NotSupported, 4 QualityWarning, 2 ResourceError, 99 never
began** (dEQP aborted two images). 0 GPU recoveries, no job hung, no reset
before the end of any run. Per image (`<LABEL>.tally.txt`; "begun" is the
image's own counter over its whole filter, the other columns are the
mustpass view):

| image | mustpass | begun | pass | fail | n.s. | QW | other | none | run | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| FO2 | 475 | 475 | 475 | 0 | 0 | 0 | 0 | 0 | 22.0 s | DONE, all passed |
| FO3 | 227 | 227 | 227 | 0 | 0 | 0 | 0 | 0 | 12.5 s | DONE, all passed |
| FO4 | 450 | 450 | 450 | 0 | 0 | 0 | 0 | 0 | 24.1 s | DONE, all passed |
| FO5 | 475 | 475 | 475 | 0 | 0 | 0 | 0 | 0 | 45.2 s | DONE, all passed |
| FO6 | 866 | 835 | 828 | 0 | 0 | 0 | 1 | 37 | 55.7 s | STOPPED EARLY by dEQP at #834 |
| FR1 | 420 | 420 | 360 | 12 | 48 | 0 | 0 | 0 | 13.5 s | DONE |
| FR2 | 520 | 520 | 422 | 31 | 67 | 0 | 0 | 0 | 15.0 s | DONE |
| LT | 56 | 58 | 55 | 1 | 0 | 0 | 0 | 0 | 1.5 s | DONE |
| M1 | 507 | 449 | 379 | 11 | 54 | 0 | 1 | 62 | 15.1 s | STOPPED EARLY by dEQP at #448 |
| PR | 504 | 504 | 504 | 0 | 0 | 0 | 0 | 0 | 22.4 s | DONE, all passed |
| R1 | 303 | 311 | 257 | 16 | 26 | 4 | 0 | 0 | 17.1 s | DONE |
| SQ | 946 | 952 | 911 | 26 | 9 | 0 | 0 | 0 | 27.9 s | DONE |
| total | 5749 | 5676 | 5343 | 97 | 204 | 4 | 2 | 99 | | |

"begun" above "mustpass" (LT, R1, SQ) is the filter matching cases outside
the mustpass list; they are left out of the other columns. "none" is the
cases of an aborted image that never began. The `--table` view over all
twelve consoles is `table.txt` (the same TOTAL row, then every non-Pass
case by name).

### Why FO6 and M1 stopped early

Both aborted on the same cause. A scissored or masked clear of an integer
colour target is refused by the V3D back end (`yakogl/v3d: draw refused: a
scissored or masked colour clear of an integer colour target`, then `yakogl:
the pending clear was not recorded (-22)`), the GL core reports
`GL_OUT_OF_MEMORY`, dEQP records a ResourceError and ends the run (`Test
run was ABORTED!`, `complete=0`):

- FO6 at case #834 `fragment_ops.scissor.clear_int_buffer`
  (`glsScissorTests.cpp:745`): 31 later cases never began (the rest of
  `fragment_ops.scissor.*` and `fragment_ops.stencil.*`; 37 mustpass cases
  without a result).
- M1 at case #448 `pbo.renderbuffer.rg16i_clears` (`glClearBufferiv`,
  `es3fPixelBufferObjectTests.cpp:345`): 58 later cases never began (the
  rest of `pbo.*` and all of `polygon_offset.*`; 62 mustpass cases without
  a result).

This is RC1 of `docs/es3-cts-failures.md` ("an integer clear program for a
scissored or masked clear", plan item v3d-int-clear). Until it lands, the
two images need splitting at those cases to get results for the cases
behind them.

### The other non-Pass cases, by cause

- `fragment_out` (FR1 12 fail, 48 n.s.; FR2 31 fail, 67 n.s.):
  - 115 NotSupported "Unsupported attachment format": `r11f_g11f_b10f`,
    `r32f`, `rg32f`, `rgba32f` render targets (48 in `array.float`, 48 in
    `basic.float`, 19 in `random`).
  - 12 Fail "Image comparison failed": every `array.float.rg16f_*` case
    (`rgba16f` and `r16f` pass; `rg16f` as a render target renders wrong).
  - 26 Fail in `random.*`: `glFramebufferRenderbuffer` returns
    `GL_INVALID_ENUM` ("GL error 0x500 in glFramebufferRenderbuffer",
    `es3fFragmentOutputTests.cpp:286`): a fourth colour attachment, past the
    V3D's three draw buffers (`GL_MAX_DRAW_BUFFERS` 3).
  - 5 Fail "Image comparison failed" in `random.*`.
- `multisample` (M1): 48 NotSupported (16 "No multisample buffers" on the
  default framebuffer, 32 "Maximum sample count returned by
  glGetInternalformativ() for GL_RGBA8 is only 1"); 4 Fail "Failed" in
  `fbo_max_samples.*` (the sample-coverage cases on a 1-sample FBO).
- `negative_api` (M1): 7 Fail "Got invalid error" (`buffer.blit_framebuffer`,
  `buffer.blit_framebuffer_multisample`,
  `buffer.read_pixels_fbo_format_mismatch`,
  `shader.uniformui_incompatible_type`, `shader.uniformuiv_incompatible_type`,
  `texture.texsubimage2d`, `texture.texsubimage3d`); 4 NotSupported
  (`GL_EXT_draw_buffers_indexed`).
- `pbo` (M1): 2 NotSupported `renderbuffer.r11f_g11f_b10f_*`.
- `lifetime` (LT): 1 Fail `attach.deleted_output.buffer_transform_feedback`
  ("Writing to container with deleted attachment modified a new object").
  `attach.deleted_input.buffer_vertex_array` also fails
  (`GL_INVALID_OPERATION` on the draw) but is not in the mustpass. The
  group's init with transform feedback runs (no `deqp: fatal`).
- `rasterization` (R1): 12 NotSupported "Test requires 4x msaa rbo"; 2 Fail
  `fbo.rbo_multisample_max.{interpolation,primitives}.lines_wide`; 4
  QualityWarning "Interpolation was calculated using projected
  coordinates" (`*.interpolation.*.lines`).
- `samplers` (R1): 14 Fail "Failed to compile shaders":
  `Program(NotYet("a texture look-up other than 2D or cube"))`, every
  `single_tex_3d.*` and `multi_tex_3d.*` case (RC2, items v3d-tex-3d-array
  and v3d-qpu-array-3d).
- `shader_api` (R1): 14 NotSupported `program_binary.*`.
- `state_query` (SQ): 26 Fail, 9 NotSupported (`indexed.*`,
  `GL_EXT_draw_buffers_indexed`). The fails: `integers.*_getfloat` and
  `*_getinteger64` of `max_color_attachments`, `max_draw_buffers`,
  `max_fragment_input_components`, `max_samples`, `max_varying_components`,
  `max_varying_vectors`, `max_vertex_output_components` (the `_getinteger`
  and `_getboolean` forms of the same values pass: the float and 64-bit
  query paths return a different value), `integers.compressed_texture_formats_*`
  (all three forms), `integers64.max_combined_{fragment,vertex}_uniform_components_*`
  and `integers64.max_uniform_block_size_*` (`_getfloat` and `_getinteger`),
  `rbo.renderbuffer_component_size_color`,
  `shader.program_active_uniform_types`, `shader.program_binary`.

Groups that pass in full: `fragment_ops.blend.*` (FO2-FO5, 1,627 cases),
`occlusion_query` 62/62 (gl_test 196's defect, fixed in v3d-oq-zero, does
not show here), `primitive_restart` 504, `rasterizer_discard` 51,
`read_pixels` 48, `vertex_array_objects` 10.

## Method

The board runner could not capture the console from this task machine: the
Pi 4's UART bridge takes one client at a time, and the leased serial
console (an MCP server connected's forwards) held that slot, so
`tools/am62p_netboot_uart.py probe` answered `busy` and `tools/board.sh pi4
tree` would have refused at `pi4_capture_start` (image removed again, no
reboot). The AM62P logs of 2026-10-31 (`*-pr-negative`, `*-pr-neg175`) met
the same limit and ran the board steps by hand; this run did the same, with
`board.sh`'s own steps in its order, from the origin/main worktree:

1. `tools/board.sh pi4 run --dry-run <zephyr.bin> <dir> <LABEL>`: the local
   image checks (AArch64 header, the 256 MiB window, the PM watchdog marker
   `board: watchdog armed`). Every image passed (`rc 0`); nothing was
   bypassed.
2. Prechecks over ssh as `common_prechecks` and `pi4_prechecks`: `sudo -n`,
   boot_id, `who` (no session from another machine; this session's own
   logins arrive through connected's port forward and were counted as its
   own), `/boot/firmware` free space at least twice the image, no leftover
   `yakogl-*.bin`, no `config-linux.txt`.
3. Staging as `pi4_stage`: the image to `/boot/firmware/yakogl-<LABEL>.bin`
   through `mktemp -d` and `install -m 0755`, `tryboot.txt` written with
   `pi4_config`'s lines (`kernel=`, `arm_64bit=1`, `enable_uart=1`,
   `uart_2ndstage=1`, the panel's `hdmi_group`/`hdmi_mode`/`hdmi_timings`),
   both read back (sha256 and text) before the reboot.
4. `sudo reboot "0 tryboot"`; the console read through the leased serial
   console (`serial_expect` on the end line
   `v3d_deqp: build <sha8> (DONE|INCOMPLETE|STOPPED)`, then `serial_read`
   of the whole run, saved to disk without a copy by hand).
5. Linux back (a new boot_id, 296-302 s after the reboot every time: the
   image's 240 s lifetime, the PM watchdog reset, the Linux boot), then
   `pi4_undo`: the staged image removed and `tryboot.txt` removed (there
   was none before), read back. Every cycle read back as before; the board
   boots its normal configuration (`config.txt` untouched throughout).

One GL client at a time: each image ran alone, the next one staged only
after Linux was back and the previous one's files were gone. Addresses in
the copies are redacted as `tools/board.sh redact_dir` does (IPv4 to
192.0.2.n, the board's host name to `<board-host>`; the Pi's getty banner
also prints an IPv6 address, redacted to `<ipv6>`); `BOARD_HOST`, the UART
bridge and this machine's addresses appear nowhere.

## Files

Every `.txt` starts with the three `#` header lines the other log text files
carry (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line
  of its boot to the `v3d_deqp: build 0ff58329 ...` end line (the watchdog
  reset prints nothing after it), CRs and NULs dropped, one line per row,
  no time stamps. Every `deqp:` result line, dEQP's own per-case lines and
  the `v3d_deqp: case #N` lines are there.
- `<LABEL>.tally.txt`: `python3 tools/cts/deqp-uart-tally.py --mustpass
  tools/cts/gles3-mustpass.txt <LABEL>.console.txt`.
- `table.txt`: the same with `--table` over the twelve consoles.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the boot_id before,
  the reboot time, the seconds until Linux was back, the clean-up read-back,
  the `cases in` line and the end line.
