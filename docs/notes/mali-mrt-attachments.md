<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: the second colour attachment (gl_test case 55, issue #32)

## The symptom

`panthor-gltest --sweep` case 55 `es3mrt` failed with GL error 0x500
(GL_INVALID_ENUM) from `glFramebufferTexture2D(GL_COLOR_ATTACHMENT1)`, before
any draw (tools/cts/log/2026-11-07-t6-step4a/).

## The cause

Not the shader and not the frame builder: the count the back end reports.
`mali::backend::DRAW_TARGETS` was 1. The GL core takes it as
`backend::draw_targets()`, and from that `context::max_draw_buffers()` /
`max_color_attachments()` (GL_MAX_DRAW_BUFFERS, GL_MAX_COLOR_ATTACHMENTS).
`framebuffer.rs` accepts GL_COLOR_ATTACHMENTi only for `i <
max_color_attachments()` and answers GL_INVALID_ENUM for the rest, so with 1
the point COLOR_ATTACHMENT1 did not exist. The value was written when "the
skeleton draws nothing"; the frame builder has taken several targets since
(`frame::begin_frame_targets`, `frame_mrt_tests.rs`, pinned byte for byte to
Mesa 26.2.1 on the drm-shim for gl_test case 55's own program and case 106's
three-output twin).

## The fix

`DRAW_TARGETS = 4` in `src/mali/backend.rs`: ES 3.0's minimum (Table 6.34) and
the most the GL core carries (`GLES_MAX_TARGETS`). The hardware has eight
(`MALI_BIFROST_BLEND_MAX_RT`); Mesa 26.2.1 reports `pan_get_max_cbufs`
(pan_screen.c:700-701) the same way, from the hardware. Nothing in `src/gl`
changed. The shader's `gl_MaxDrawBuffers` follows (`backend::draw_targets`).

Host test: `case_55s_second_colour_attachment_raises_no_gl_error`
(`src/mali/backend_tests.rs`) takes case 55's attach sequence (two RGBA
textures, `glFramebufferTexture2D` on attachments 0 and 1) through the Mali
back end: no GL error, framebuffer complete, both queries 4, attachment 3 is a
point and 4 is not. It fails with 0x500 at `DRAW_TARGETS = 1`.

## The draw (the second step)

After the fix above the attach no longer failed, but the draw did:
`backend::check` refused a draw with `num_targets > 1`
(`NOT_BUILT_TARGET_COUNT`), because `open_frame` opened a single-target frame
(`begin_frame`), and the retention keys knew one target. Case 55 moved from
FAIL to REFUSED (draw). The back end now draws it (`src/mali/backend.rs`):

- `TargetKey::more` holds targets 1.. (memory, size, row stride, format), so
  a draw into another set of targets ends the open frame, as one into
  another target 0 does. `open_frame` opens the frame with all of them
  (`colour_targets`, `frame::begin_frame_targets`); the frame builder's end
  writes one render target descriptor and one blend descriptor a target.
- What a frame keeps per target: the sample store is dropped when the frame
  writes any of its targets, and a released texture ends the open frame when
  it is any of its targets (`TargetKey::writes`).
- `check_targets` checks each target's format (`NOT_BUILT_TARGET_FORMAT`, the
  same four formats as target 0) and size. `out_shift` is still refused for
  every draw (`NOT_BUILT_OUTPUT_SHIFT`).
- `src/mali/program.rs`: the fragment meta took `colour_outputs` as 1 for
  every program. It now takes the record's (`MaliShader::colour_outputs`,
  bit 0 always), so DCD1's render target mask is 0x3 for case 55, as Mesa
  sets it. Before, a two-output program drew with mask 0x1. A single-target
  draw of a program that writes an output past its targets is now refused
  by `frame::NOT_BUILT_FS_OUTPUTS`. It used to build with a blend descriptor
  table of one, which the program's second `BLEND` read past.

Targets 1.. carry no row pitch (`GlesDrawTarget` has none). They are taken
at the GL core's linear pitch for their width: the pair-padded width on 64
bytes (`linear_row_stride`, `gl::context::linear_pitch`).

Reference: the frame of `case_55s_draw_into_two_targets_is_mesa_s_frame`
(`src/mali/backend_tests.rs`) is compared with Mesa 26.2.1 on the drm-shim,
tools/mali/testdata/fsout_55_es3mrt.trace (the record `frame_mrt_tests.rs`
pins, gl_ref.py `--mrt 2`). The program is case 55's own (mrt2.*), and the
draw goes through `mali_draw`. These match the trace exactly: both blend
descriptors, the BLEND_DESC count 2, DCD1 (r58 0x3FFFF), the FBD pointer's
and the parameters' render target count 2, Color Buffer Allocation and
Effective Tile Size, and each render target. The render target's named
difference stays (linear at the target's own memory, where Mesa's is AFBC).
No new drm-shim run was needed: the draw is the one the record already holds.
`case_55s_draw_writes_both_attachments` runs case 55 through the GL API on
the mock: the attaches, glClear, glDrawArrays and glFinish give no GL error,
and the result is one frame of two render targets at the two textures, with
DCD1 mask 0x3. Case 55's fragment shader compiles (`case55_two_fragment_outputs_are_byte_equal_to_mesa_26_2_1`,
src/shaderc/online/mali/tests.rs). On the host, case 55 meets no refusal.

## What is not done

A draw of several targets is still refused, each case by name:

| Case | Refusal |
|---|---|
| A frame that does not start from a whole colour clear (the colour preload draws back target 0 only) | `NOT_BUILT_TARGETS_PRELOAD` |
| A masked, scissored or per-draw-buffer clear, or a clear after a draw (the clear quad writes target 0 only) | `NOT_BUILT_TARGETS_CLEAR` |
| Targets of different sizes | `NOT_BUILT_TARGET_SIZES` |
| A GL_NONE draw buffer between targets | `NOT_BUILT_TARGET_GAP` |
| Target 0 at another pitch than the linear pitch for its width (a mip level) | `NOT_BUILT_TARGETS_PITCH` |
| 4x, a Z/S preload, a blend shader | `frame::NOT_BUILT_MRT` |

The preload refusal is the one dEQP's MRT cases are most likely to meet next.
The pitch check guards target 0 only. A target 1.. that is a level past 0
of a texture with a chain is laid out at level 0's pitch, which the draw
cannot see, so it would be written at the wrong stride (inside its own
storage). Fixing that takes a pitch in `GlesDrawTarget`, which is a GL core
change.
