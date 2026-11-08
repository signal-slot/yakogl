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

## What is not done

The attach no longer fails; the draw still does. `backend::check` refuses a
draw with `num_targets > 1` (`NOT_BUILT_TARGET_COUNT`), because `open_frame`
opens a single-target frame (`begin_frame`), not `begin_frame_targets` with
the framebuffer's other `GlesDrawTarget`s, and the retention keys, preload and
clear paths know one target. Case 55 therefore moves from FAIL to REFUSED
(draw) until that is wired; the frame side needs nothing new (`frame_mrt_tests`
has the reference-exact two- and three-target frames). Advertising four also
means the dEQP MRT cases now reach the draw instead of stopping at the query.
