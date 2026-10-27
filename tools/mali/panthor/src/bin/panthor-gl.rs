// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! panthor-gl: milestone L2 of driver/docs/nanopc-t6-mali.md. A GL program
//! through YakoGL end to end on the NanoPC-T6: EGL and the GL entry points
//! of the library, its `gpu-mali` backend (src/mali/backend.rs) building the
//! frame (src/mali/frame.rs) in the OSAL's buffers at their GPU VAs, the
//! shaders compiled by our Valhall back end at glCompileShader time, and the
//! development OSAL ([`yakogl_mali_panthor::osal`]) submitting to panthor.
//! No Mesa and no trace bytes.
//!
//! The program: eglInitialize, a 64x64 pbuffer, an ES 2 context; a 64x64
//! RGBA8 texture filled with a pattern (neither a clear nor a draw colour),
//! attached to an FBO; glClearColor(0, 0, 1, 1) + glClear; tri.vert and
//! tri.frag (tools/mali/testdata) compiled and linked; glDrawArrays of the
//! two triangles over the left half; glReadPixels of the whole FBO.
//!
//! Usage: panthor-gl [--green] [--varying] [--uniform-green] [--uniform-red] [--mat4]
//!        [--color-uv] [--clear-only] [--depth] [--nodepth] [--clearhide] [--mask] [--nocolour]
//!        [--scissor-clear] [--lines] [--line-strip] [--line-loop] [--tri-strip] [--tri-fan]
//!        [--flip-tri] [--flip-frontcw] [--flip-viewport] [--flip-scissor-draw] [--flip-scissor-clear]
//!        [--flip-manylines] [--flip-manylines-colour1] [--window-tri] [--window-mid]
//!        [--window-xrgb-red] [--window-xrgb-mid] [--window-565-red] [--window-565-mid]
//!        [--window-preload] [--window-xrgb-preload] [--window-565-preload]
//!        [--idx-u16-client] [--idx-u16-ibo] [--idx-u16-offset] [--idx-strip-ibo] [--idx-u8-ibo]
//!        [--tex-{nearest,linear}-{clamp,repeat,mirror}[-16]] [--tex-solid W]
//!        [--mip-{nearest,linear-nearest,trilinear}[-8]] [--fmt-{etc1,etc2rgba,srgb}]
//!        [--preload-{half,strip}] [--preload-zs-{depth,stencil,depth-sclear,stencil-zclear}]
//!        [--blend-<over|add|premul|replace|sub|revsub|mul|twosd|const|sep>]
//!        [--poff-<flat_0_m4|flat_0_4|flat_0_m1|flat_m1_0|tilt_m1_0|tilt_1_0|tilt_m0p5_m1>]
//!        [--msaa-4x] [--msaa-1x] [--msaa-cov50] [--msaa-cov50inv]
//!        [--points-<const|attr|uniform|tri_psiz|nopsiz>] [--pcoord-fbo] [--pcoord-pb]
//!        [--vcomp-<b|xy|xz|w|zw|xyz|axby|axybzw>] [--fsconst-<h|hc|t|q|hf>]
//!        [--fneg-<n|nm|m1|om|sub|ns|hm|mh>] [--fabs-<a|an|na|as|aa|oa|am>]
//!        [--fragcoord] [--fragcoord-flip]
//!        [--fb /dev/fb0] [--scale N]
//!
//! - default: expect red 2048, blue 2048, other 0, misplaced 0.
//! - `--green`: tri.frag's constant colour changed to green before
//!   glShaderSource, so the program bytes are made now: green 2048, blue 2048.
//! - `--varying`: tri_varying.vert / .frag (tools/mali/testdata) with the
//!   colour attribute green at every vertex: the IDVS varying program carries
//!   it to the fragment shader, green 2048, blue 2048.
//! - `--uniform-green`, `--uniform-red`: tri.vert and sel_u_color.frag (one
//!   mediump vec4 uniform written as the colour, tools/mali/testdata), the
//!   colour set by glUniform4f: the same program bytes for both, only the
//!   fp16 push words differ. Green (or red) 2048, blue 2048.
//! - `--mat4`: ctl_mat4.vert (`u_mvp * a_pos`) and tri.frag, u_mvp set by
//!   glUniformMatrix4fv to a translation by +0.5 in x: the two triangles
//!   move from x in [-1, 0] to [-0.5, 0.5], window columns [16, 48)
//!   (pixel centres at x + 0.5; the edges fall on 16.0 and 48.0), so red
//!   2048 in those columns, blue 2048 outside them.
//! - `--color-uv`: gl_test's own two-varying pair (color_uv.vert / .frag,
//!   tools/shaders/gl_test_es100: `v_color * vec4(v_uv, 1.0, 1.0)`), with
//!   per vertex a_uv = (x + 1, (y + 1) / 2) and a_color = (1, 0.5, (y + 1)
//!   / 2, 1). Both varyings are affine in the window position, so the
//!   prediction is per pixel: at pixel (px, py) of the left half R =
//!   round((2px + 1) 255 / 64), G = round((2py + 1) 255 / 256), B =
//!   round((2py + 1) 255 / 128), A = 255 (each value fp16 holds exactly,
//!   no one on a rounding tie: (2k + 1) 255 is odd); the right half the
//!   clear blue. Every pixel is compared, with no tolerance.
//! - `--clear-only`: no program and no draw: blue 4096.
//! - `--depth`, `--nodepth`, `--clearhide`, `--mask`, `--nocolour`: the program of the
//!   board's Mesa reference 0dce654a8:tools/mali/board-ref/2026-10-19-frame-depth/zs.c,
//!   mode for mode, on the same FBO with a GL_DEPTH24_STENCIL8_OES
//!   renderbuffer on both points; Mesa's read-backs of these modes were 0
//!   pixels off the same predictions. `--depth`: cleared blue, depth 1.0,
//!   stencil 0x5a, depth test LESS, green at z -0.5 over x [-0.5, 1], then
//!   red at z +0.5 over x [-1, 0.5]: columns 0-15 red, 16-63 green (the
//!   red is behind the green), one frame of two draws. `--nodepth`: the
//!   test off, red drawn last: columns 0-47 red, 48-63 green. `--clearhide`:
//!   depth cleared to 0.25, red at z 0 (window depth 0.5) over the whole
//!   target with LESS: blue 4096. `--mask`: cleared blue, then
//!   glColorMask(R only) and a white clear (the clear quad): magenta 4096.
//!   `--nocolour`: depth test LESS, green at z -0.5 over x [-0.5, 1] with
//!   glColorMask(none) (depth only, no fragment shader), then red at z +0.5
//!   over the whole target: columns 0-15 red, the rest blue.
//! - `--scissor-clear`: cleared blue, then a red clear with the scissor
//!   test on over (16, 16, 32, 32) (the clear quad): red 1024 in that box,
//!   blue 3072.
//! - `--lines`, `--line-strip`, `--line-loop`, `--tri-strip`, `--tri-fan`:
//!   the program of the board's Mesa reference
//!   0dce654a8:tools/mali/board-ref/2026-10-19-primitives/prims.c, mode for mode:
//!   cleared blue, tri.vert and tri.frag, one red glDrawArrays in that mode
//!   at prims.c's vertices. The prediction is Mesa's read-back of the same
//!   calls: its red count and the FNV-1a hash of the whole target (the same
//!   bytes as Mesa's, pixel for pixel), printed with the red pixels of row
//!   20 and column 40 as prims.c prints them.
//! - `--points-<mode>`: the program of the board's Mesa reference
//!   0dce654a8:tools/mali/board-ref/2026-10-19-points/points.c, mode for mode: cleared
//!   blue, points.c's vertex shader and its red fragment shader, three
//!   points at (8, 8), (32, 32), (56, 56) with gl_PointSize 4.0, from an
//!   attribute (2, 5, 8) or a uniform (3), the 4.0 shader drawing the left
//!   half as a triangle strip, or a shader that does not write
//!   gl_PointSize. The prediction is Mesa's read-back of the same calls (red
//!   count and FNV-1a of the whole target), printed as the triangle-strip
//!   modes print theirs.
//! - `--vcomp-<mode>`: the program of the board's Mesa reference
//!   0dce654a8:tools/mali/board-ref/2026-10-19-varying-compact/vcomp.c, mode for mode:
//!   tools/mali/testdata/cmp_vs4.vert (or cmp_vs2x4.vert) and
//!   cmp_<mode>.frag, a fragment shader that reads part of its varyings, so
//!   the link builds the program's vertex shader for those reads; two
//!   triangles over the FBO cleared black, each varying component its own
//!   value (one fp16 holds exactly, the same at every vertex). The
//!   prediction is Mesa's read-back: the pixels at (8, 8), (56, 8), (8, 56),
//!   (56, 56) and the FNV-1a of the whole target.
//! - `--fsconst-<mode>`: the program of the board's Mesa reference
//!   0dce654a8:tools/mali/board-ref/2026-10-19-fs-const-mul/fsconst.c, mode for mode:
//!   vcomp.c's draw with tools/mali/testdata/sel_f_const_<mode>.frag, a
//!   fragment shader product of v_color with a constant. The prediction is
//!   Mesa's read-back, as for `--vcomp-<mode>`.
//! - `--fneg-<mode>`: the same for the board's Mesa reference
//!   0dce654a8:tools/mali/board-ref/2026-10-19-fs-neg/fneg.c and
//!   tools/mali/testdata/sel_f_neg_<mode>.frag, a fragment shader fneg.
//! - `--fabs-<mode>`: the same for 0dce654a8:tools/mali/board-ref/2026-10-19-fs-abs/
//!   fabs.c and sel_f_abs_<mode>.frag, with fabs.c's v_color (three lanes
//!   negative).
//! - `--pcoord-fbo`, `--pcoord-pb`: gl_test case 83's draw, the program of
//!   the board's Mesa reference 0dce654a8:tools/mali/board-ref/2026-10-19-pointcoord/
//!   pcoord.c: one point of gl_PointSize 16 at the pixel corner (32, 32),
//!   its fragment shader writing (s, t, 0, 1), into an FBO texture or an
//!   EGL pbuffer (cleared black). The prediction is Mesa's read-back, the
//!   same for both: FNV-1a fb970d45, the sprite's corners GL (24, 39)
//!   ff000808, (39, 39) ff0008f7, (24, 24) ff00f708, (39, 24) ff00f7f7 and
//!   (23, 32) outside it black.
//! - `--flip-tri`, `--flip-frontcw`, `--flip-viewport`, `--flip-scissor-draw`,
//!   `--flip-scissor-clear`: a top-down target, the program of the board's
//!   Mesa reference 0dce654a8:tools/mali/board-ref/2026-10-19-flip/flip.c mode for
//!   mode. No EGL: `gl_context_create(64, 48)`, as gl_test's harness makes
//!   its context, whose default surface is stored top-down (the first row
//!   in memory is the top of the picture, `GlContext::default_y_up` false)
//!   and drawn with the flipped viewport. Cleared blue, then in red: the
//!   triangle (8, 8) (40, 8) (8, 40) (`--flip-frontcw`: with
//!   glFrontFace(GL_CW)); the full quad in glViewport(8, 4, 32, 16); the full
//!   quad with glScissor(8, 4, 16, 20); a red clear with that scissor. The
//!   prediction is Mesa's read-back of the same calls into a 64x48 pbuffer
//!   (a window framebuffer to Mesa): its red count and the FNV-1a hash of
//!   the glReadPixels bytes. And the surface is top-down: each of its 48
//!   rows in memory is the read-back's row mirrored (a stack that drew and
//!   read y-up would match the hash and fail this).
//! - `--flip-manylines`, `--flip-manylines-colour1`: gl_test case 56's calls
//!   (driver/tools/pvrlab/gl_test.c) on a 64x64 top-down default surface,
//!   with its shader pair (color_uv.vert/.frag, byte-equal to
//!   gl_test_shaders_es100.h) linked without glBindAttribLocation: 2, 4, 8,
//!   16 and 20 horizontal GL_LINES at top-down rows 2 + 3i, attribute arrays
//!   0 and 2 enabled. The case never enables attribute 1 (a_color), which
//!   then reads its current value (0, 0, 0, 1). The prediction is the board's
//!   Mesa on the same calls into a pbuffer
//!   (0dce654a8:tools/mali/board-ref/2026-10-19-flip/manylines.c): 0 rows lit, the
//!   line pixel ff000000. `-colour1` enables attribute 1 with the vertices'
//!   (1, 1, 1, 1): every row lit, the pixel ffffffff.
//! - `--window-tri`: an EGL window surface over the program's own pool
//!   memory, as gl_test's window cases make one: a 64x48 `gpu_alloc`
//!   filled with a pattern, its `phys` in a gles_native_window (RGBA8888,
//!   256-byte rows), eglCreateWindowSurface single-buffered (the OSAL's
//!   `map_phys`: the same pages at a second VA and CPU address). Cleared
//!   blue, flip.c's red triangle, glFinish. Expect Mesa's pbuffer read-back
//!   of flip.c "tri" (red 496, FNV-1a 890abd45), and the program's own
//!   allocation, read through its own CPU view and not the window's, to
//!   hold the picture top-down (48 of 48 rows the read-back's mirrored):
//!   the GPU wrote the caller's pages.
//! - `--window-preload`, `--window-xrgb-preload`, `--window-565-preload`:
//!   the window of that format with the program of 0dce654a8:tools/mali/board-ref/
//!   2026-10-19-preload-formats/preload_fmt.c (red over columns 0-31,
//!   glFinish, green over 16-47 in a frame without a clear, which the
//!   colour preload starts from the old image): Mesa's fnv1a 9a3071c5 in
//!   every format, and the program's own allocation as for `--window-tri`.
//! - `--window-mid`, `--window-xrgb-red`, `--window-xrgb-mid`, `--window-565-red`,
//!   `--window-565-mid`: `--window-tri` in a window of that native format
//!   (RGBA8888, XRGB8888 B,G,R,X bytes, RGB565) with the config of its
//!   sizes, the triangle red or the mid colour (0.5, 0.25, 0.75), the
//!   program of 0dce654a8:tools/mali/board-ref/2026-10-19-formats/fmt.c. The
//!   prediction is Mesa's read-back of fmt.c in the same config and colour
//!   (red 890abd45 in every format; mid b343e245 in 8888 and 04c57ea5 in
//!   RGB565, which Mesa dithers into four colours), and the program's own
//!   allocation, read in the window's layout, holding the read-back
//!   top-down (48 of 48 rows).
//! - `--blend-over`, `--blend-add`, `--blend-premul`, `--blend-replace`,
//!   `--blend-sub`, `--blend-revsub`, `--blend-mul`, `--blend-twosd`,
//!   `--blend-const`, `--blend-sep`: the program of the board's Mesa
//!   reference 0dce654a8:tools/mali/board-ref/2026-10-19-blend/blend.c mode for mode:
//!   the FBO cleared to (0.25, 0.5, 0.75, 0.5), a quad over the left half in
//!   (1.0, 0.5, 0.25, 0.75) with that blend state. The prediction is Mesa's
//!   read-back (the FNV-1a in [`BLENDS`]).
//! - `--poff-<mode>`: the program of the board's Mesa reference
//!   0dce654a8:tools/mali/board-ref/2026-10-19-polyoffset/poff.c mode for mode: the FBO
//!   with the D24S8 renderbuffer cleared black and depth 1.0, LESS, a blue
//!   quad, then a green one on the same plane (flat z = 0, or tilted
//!   z = x / 2) with GL_POLYGON_OFFSET_FILL and glPolygonOffset(factor,
//!   units) of the mode's name (m minus, p point). The prediction is Mesa's
//!   read-back: all green (c38d1dc5) or all blue (bdf93dc5).
//! - `--msaa-4x`, `--msaa-1x`: gl_test case 16's draw (a magenta triangle over
//!   the lower right half, its edge the diagonal, on black) into a 64x64
//!   pbuffer of the EGL_SAMPLES 4 config, or of the single-sampled one. The
//!   prediction is the board's Mesa on the same draw
//!   (0dce654a8:tools/mali/board-ref/2026-10-19-msaa/msaa.c, its 4x render-to-texture
//!   and its own 4x pbuffer alike): 4x 3c0190c5, the 64 diagonal pixels
//!   half covered (ff800080); 1x 570010c5. `--msaa-cov50` and
//!   `--msaa-cov50inv`: the 4x draw with GL_SAMPLE_COVERAGE on,
//!   glSampleCoverage(0.5, GL_FALSE / GL_TRUE); Mesa d8c31dc5 / a8c31dc5.
//! - `--idx-u16-client`, `--idx-u16-ibo`, `--idx-u16-offset`, `--idx-strip-ibo`,
//!   `--idx-u8-ibo`: the program of the board's Mesa reference
//!   0dce654a8:tools/mali/board-ref/2026-10-19-indexed/idx.c mode for mode: eight
//!   vertices (0-3 the left half's corners, 4-7 the right half's), one red
//!   glDrawElements naming some of them, with client-side u16 indices,
//!   u16 indices in an element buffer (from offset 0, or from byte 6 of a
//!   longer one), a u16 strip, or u8 indices. The prediction is Mesa's
//!   read-back: red 2048, FNV-1a 8eb2fdc5 (the right half) or f350fdc5
//!   (the strip, the left half).
//! - `--delete-held`: the OSAL's deferred free (`gpu_free`'s contract,
//!   include/gles_osal.h). The red triangle drawn into the FBO's texture A
//!   and glFlush (the frame submitted, not waited for); glDeleteTextures(A):
//!   exactly 1 BO held for a kick (A's store, held to the kick being
//!   recorded); then a second FBO (texture B) cleared blue and read back,
//!   which completes that kick: 0 held, B blue 4096.
//! - `--tex-nearest-clamp`, `--tex-linear-clamp`, `--tex-nearest-repeat`,
//!   `--tex-linear-repeat` (and each with `-16`): the program of the
//!   board's Mesa reference 0dce654a8:tools/mali/board-ref/2026-10-19-texture/tex.c,
//!   mode for mode: sel_t_tex's pair (tools/mali/testdata:
//!   `texture2D(u_tex, v_uv)`), a full-screen quad whose coordinate runs
//!   from -0.5 to 1.5, an 8x8 (or 16x16) RGBA8 texture with texel (i, j) =
//!   (16 i + 8, 16 j + 8, 128, 255), GL_NEAREST or GL_LINEAR, GL_CLAMP_TO_EDGE
//!   or GL_REPEAT. Every pixel is predicted exactly (tex.c's `predict`) and
//!   the FNV-1a hash of the read-back is Mesa's for the same calls. The
//!   8-wide texture's rows are 32 bytes in the GL core's storage, the
//!   16-wide one's 64 (Mesa's linear layout aligns a row to 64 bytes).
//!   `--tex-nearest-mirror`, `--tex-linear-mirror` (and `-16`): the same with
//!   GL_MIRRORED_REPEAT (0dce654a8:tools/mali/board-ref/2026-10-19-texture-mips/tex.c).
//! - `--mip-nearest`, `--mip-linear-nearest`, `--mip-trilinear` (and each
//!   with `-8`): the program of the board's Mesa reference
//!   0dce654a8:tools/mali/board-ref/2026-10-19-texture-mips/mip.c: a 32x32 (or 8x8)
//!   texture, its whole chain uploaded, level l one solid colour; four bands
//!   of 16 rows, band b at 2^b texels of level 0 a pixel (the level of
//!   detail exactly b): band b is level b's colour at every pixel, and the
//!   hash Mesa's. The 32-wide chain is sampled in place (rows of 128 bytes
//!   at every level), the 8-wide one copied.
//! - `--fmt-etc1`, `--fmt-etc2rgba`, `--fmt-srgb`: the program of the
//!   board's Mesa reference 0dce654a8:tools/mali/board-ref/2026-10-19-texture-formats/
//!   fmt.c: a full-screen quad, GL_NEAREST, a 16x8 GL_ETC1_RGB8_OES texture
//!   of gl_test case 20's flat blocks, an 8x8 GL_COMPRESSED_RGBA8_ETC2_EAC
//!   one (alpha 255 / 128 / 64 / 0), a 4x4 GL_SRGB8_ALPHA8 one: every pixel
//!   fmt.c's prediction (an sRGB value exactly the double-precision one, as
//!   Mesa read back) and the hash Mesa's.
//! - `--tex-solid W`: sel_t_tex's pair over the blue clear, a W x 4 RGBA8
//!   texture every texel green, GL_NEAREST, clamped: every pixel green
//!   (4096). The minimal case of a texture's row stride: the GL core's
//!   storage is W texels a row (W even), 4 W bytes.
//! - `--preload-half`, `--preload-strip`: the program of the board's Mesa
//!   reference 0dce654a8:tools/mali/board-ref/2026-10-19-preload/preload.c: over the
//!   blue clear red on columns 0-31, glFinish, then green on 32-63 (half) or
//!   16-47 (strip) in a frame without a clear, which the colour preload
//!   starts from the first frame's image: every pixel preload.c's
//!   prediction and the hash Mesa's.
//! - `--preload-zs-depth`, `-stencil`, `-depth-sclear`, `-stencil-zclear`:
//!   the program of 0dce654a8:tools/mali/board-ref/2026-10-19-preload-zs/preload_zs.c
//!   on the FBO with a D24S8: frame 1 leaves columns 0-31 at depth 0.5 and
//!   stencil 1, frame 2 draws green over all without a clear (or after
//!   clearing the plane it does not test), depth- or stencil-tested, so
//!   only columns 32-63 pass: every pixel preload_zs.c's prediction and the
//!   hash Mesa's.
//! - `--fragcoord`, `--fragcoord-flip`: the program of the board's Mesa
//!   reference 0dce654a8:tools/mali/board-ref/2026-10-22-s4-fragcoord/fragcoord.c
//!   (#1384 S4, gl_FragCoord): gl_FragCoord.xy copied to a highp variable,
//!   times 1/64, into red and green, a full-screen quad into a 64x48 FBO
//!   (`--fragcoord`, the y transform (1, 0)) or into the 64x48 top-down
//!   default surface as the `--flip-*` modes make it (`--fragcoord-flip`,
//!   (-1, 48)). gl_FragCoord counts y from the bottom on both, so the
//!   read-back is the same: R = round((2x + 1) 255 / 128), G the same of y,
//!   B 0, A 255 at every pixel, Mesa's FNV-1a 99cb74c5 on both of its
//!   targets (the FBO and the pbuffer, which it draws flipped).
//! - `--fb <path>`: after the read-back, the picture at N x (default 8) in a
//!   white border at the framebuffer's top-left, nothing else written.
//!
//! A GL error, lost GPU work (`yakogl_gpu_work_lost`), a refused draw, a
//! group the kernel marks timed out or faulted, or an unusable VM is a
//! failure, whatever the pixels.

use std::ffi::CString;
use std::fs::OpenOptions;
use std::os::unix::fs::FileExt;
use std::process::ExitCode;
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::{glDrawArrays, glDrawElements};
use yakogl::gl::egl::{
    eglChooseConfig, eglCreateContext, eglCreatePbufferSurface, eglGetDisplay, eglInitialize, eglMakeCurrent,
};
use yakogl::gl::egl_consts::*;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glFramebufferRenderbuffer, glFramebufferTexture2D,
    glGenFramebuffers, glGenRenderbuffers, glReadPixels, glRenderbufferStorage,
};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetAttribLocation, glGetProgramInfoLog, glGetProgramiv,
    glGetUniformLocation, glLinkProgram, glUniform1f, glUniform1i, glUniform2f, glUniform4f, glUniform4fv,
    glUniformMatrix4fv, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderInfoLog, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{
    glBlendColor, glBlendEquation, glBlendFunc, glBlendFuncSeparate, glClear, glClearColor, glClearDepthf,
    glClearStencil, glColorMask, glDepthFunc, glDepthMask, glDisable, glEnable, glFinish, glFlush, glFrontFace,
    glGetError, glPolygonOffset, glSampleCoverage, glScissor, glStencilFunc, glStencilOp, glViewport,
};
use yakogl::gl::texture::{
    glBindTexture, glCompressedTexImage2D, glDeleteTextures, glGenTextures, glTexImage2D, glTexParameteri,
};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl_mali_panthor::l1;
use yakogl_mali_panthor::osal;

const TRI_VS: &str = include_str!("../../../testdata/tri.vert");
const TRI_FS: &str = include_str!("../../../testdata/tri.frag");
const TV_VS: &str = include_str!("../../../testdata/tri_varying.vert");
const TV_FS: &str = include_str!("../../../testdata/tri_varying.frag");
const COLOR_FS: &str = include_str!("../../../testdata/sel_u_color.frag");
const MAT4_VS: &str = include_str!("../../../testdata/ctl_mat4.vert");
const CUV_VS: &str = include_str!("../../../../shaders/gl_test_es100/color_uv.vert");
const CUV_FS: &str = include_str!("../../../../shaders/gl_test_es100/color_uv.frag");
const POINTS_VS: &str = include_str!("../../../../shaders/gl_test_es100/points.vert");
const POINTCOORD_FS: &str = include_str!("../../../../shaders/es100/pointcoord.frag");
const FVG_VS: &str = include_str!("../../../testdata/fvg_fill_color_nodiscard_noscissor.vert");
const FVG_FC_FS: &str = include_str!("../../../testdata/fvg_fill_color_nodiscard_noscissor.frag");
const FVG_FCS_FS: &str = include_str!("../../../testdata/fvg_fill_color_nodiscard.frag");
/// `--mat4`'s u_mvp, column-major: the identity with +0.5 in x.
const MAT4_SHIFT: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 1.0];
/// tri_varying's per-vertex colour for `--varying`: green at all six
/// vertices, so the interpolated varying is green everywhere it is drawn.
const GREEN_VERTICES: [f32; 18] =
    [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
const W: usize = 64;
const H: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Red,
    Green,
    Varying,
    UniformGreen,
    UniformRed,
    Mat4,
    ColorUv,
    ClearOnly,
    DeleteHeld,
    Depth,
    NoDepth,
    ClearHide,
    Mask,
    NoColour,
    ScissorClear,
    Lines,
    LineStrip,
    LineLoop,
    TriStrip,
    TriFan,
    FlipTri,
    FlipFrontCw,
    FlipViewport,
    FlipScissorDraw,
    FlipScissorClear,
    FlipManyLines,
    FlipManyLinesColour1,
    /// tex.c's mode [`TEX_MODES`]`[i]`.
    Tex(usize),
    /// `--tex-solid W`.
    TexSolid(usize),
    /// mip.c's mode [`MIP_MODES`]`[i]`.
    Mip(usize),
    /// fmt.c's mode: 0 etc1, 1 etc2rgba, 2 srgb.
    Fmt(usize),
    /// preload.c's mode: `strip` (true) or `half`.
    Preload(bool),
    /// preload_zs.c's mode [`PRELOAD_ZS_MODES`]`[i]`.
    PreloadZs(usize),
    WindowTri,
    WindowMid,
    WindowXrgbRed,
    WindowXrgbMid,
    Window565Red,
    /// preload_fmt.c in the window's layout: `--window-<fmt>-preload`.
    WindowPreload(u32),
    Window565Mid,
    IdxU16Client,
    IdxU16Ibo,
    IdxU16Offset,
    IdxStripIbo,
    IdxU8Ibo,
    /// A blend.c mode, by its index in [`BLENDS`].
    Blend(usize),
    /// A poff.c mode, by its index in [`POFFS`].
    Poff(usize),
    /// msaa.c's draw into a pbuffer of this many samples.
    Msaa(u32),
    /// msaa.c's 4x draw with glSampleCoverage(0.5, invert).
    MsaaCov(bool),
    /// A points.c mode, by its index in [`POINT_MODES`].
    Points(usize),
    /// pcoord.c's draw into a pbuffer (true) or the FBO.
    Pcoord(bool),
    /// A vcomp.c mode, by its index in [`VCOMP_MODES`].
    Vcomp(usize),
    /// An fsconst.c mode, by its index in [`FSCONST_MODES`].
    Fsconst(usize),
    /// An fneg.c mode, by its index in [`FNEG_MODES`].
    Fneg(usize),
    /// An fabs.c mode, by its index in [`FABS_MODES`].
    Fabs(usize),
    /// femtovg's solid fill (fill_color.nodiscard.noscissor), #1384 S4.
    FvgFillColor,
    /// femtovg's solid fill under a scissor (fill_color.nodiscard, the
    /// `if` of scissorMask), #1384 S5.
    FvgFillColorScissor,
    /// fragcoord.c's draw into the FBO (false) or the top-down default
    /// surface (true), #1384 S4.
    FragCoord(bool),
}

const CMP_VS4: &str = include_str!("../../../testdata/cmp_vs4.vert");
const CMP_VS2X4: &str = include_str!("../../../testdata/cmp_vs2x4.vert");

/// vcomp.c's modes: the name, the fragment shader, whether the vertex
/// shader is cmp_vs2x4.vert, and Mesa's read-back on the board
/// (0dce654a8:tools/mali/board-ref/2026-10-19-varying-compact/mesa): the four pixels
/// and the FNV-1a.
const VCOMP_MODES: [(&str, &str, bool, [u32; 4], u32); 8] = [
    ("b", include_str!("../../../testdata/cmp_b.frag"), false, [0xff408060; 4], 0x21831dc5),
    ("xy", include_str!("../../../testdata/cmp_xy.frag"), false, [0xff40bf40; 4], 0x46cd1dc5),
    ("xz", include_str!("../../../testdata/cmp_xz.frag"), false, [0xff406040; 4], 0xe3831dc5),
    ("w", include_str!("../../../testdata/cmp_w.frag"), false, [0xff408020; 4], 0x46031dc5),
    ("zw", include_str!("../../../testdata/cmp_zw.frag"), false, [0xff402060; 4], 0x25431dc5),
    ("xyz", include_str!("../../../testdata/cmp_xyz.frag"), false, [0xff60bf40; 4], 0xa62d1dc5),
    ("axby", include_str!("../../../testdata/cmp_axby.frag"), true, [0xff40df40; 4], 0x0d8d1dc5),
    ("axybzw", include_str!("../../../testdata/cmp_axybzw.frag"), true, [0xef10bf40; 4], 0x78dd1dc5),
];

/// fsconst.c's modes, in [`VCOMP_MODES`]' form: the fragment shader is the
/// shim reference's, and Mesa's read-back on the board
/// (0dce654a8:tools/mali/board-ref/2026-10-19-fs-const-mul/mesa).
const FSCONST_MODES: [(&str, &str, bool, [u32; 4], u32); 5] = [
    ("h", include_str!("../../../testdata/sel_f_const_h.frag"), false, [0x10306020; 4], 0x3b499dc5),
    ("hc", include_str!("../../../testdata/sel_f_const_hc.frag"), false, [0x10306020; 4], 0x3b499dc5),
    ("t", include_str!("../../../testdata/sel_f_const_t.frag"), false, [0x40bfff80; 4], 0x60167dc5),
    ("q", include_str!("../../../testdata/sel_f_const_q.frag"), false, [0x20bf3020; 4], 0x03581dc5),
    ("hf", include_str!("../../../testdata/sel_f_const_hf.frag"), false, [0x50709f60; 4], 0x2e971dc5),
];

/// fneg.c's modes, in [`VCOMP_MODES`]' form, with Mesa's read-back on the
/// board (0dce654a8:tools/mali/board-ref/2026-10-19-fs-neg/mesa).
const FNEG_MODES: [(&str, &str, bool, [u32; 4], u32); 8] = [
    ("n", include_str!("../../../testdata/sel_f_neg_n.frag"), false, [0x00000000; 4], 0x38699dc5),
    ("nm", include_str!("../../../testdata/sel_f_neg_nm.frag"), false, [0x00000000; 4], 0x38699dc5),
    ("m1", include_str!("../../../testdata/sel_f_neg_m1.frag"), false, [0x10300000; 4], 0xdd499dc5),
    ("om", include_str!("../../../testdata/sel_f_neg_om.frag"), false, [0xdf9f40bf; 4], 0x63631dc5),
    ("sub", include_str!("../../../testdata/sel_f_neg_sub.frag"), false, [0xff409f00; 4], 0x170d1dc5),
    ("ns", include_str!("../../../testdata/sel_f_neg_ns.frag"), false, [0x9f00009f; 4], 0x89eabdc5),
    ("hm", include_str!("../../../testdata/sel_f_neg_hm.frag"), false, [0x00002000; 4], 0x9b699dc5),
    ("mh", include_str!("../../../testdata/sel_f_neg_mh.frag"), false, [0x70502060; 4], 0xc6499dc5),
];

/// fabs.c's modes, in [`VCOMP_MODES`]' form, with Mesa's read-back on the
/// board (0dce654a8:tools/mali/board-ref/2026-10-19-fs-abs/mesa).
const FABS_MODES: [(&str, &str, bool, [u32; 4], u32); 7] = [
    ("a", include_str!("../../../testdata/sel_f_abs_a.frag"), false, [0x2060bf40; 4], 0x5ca71dc5),
    ("an", include_str!("../../../testdata/sel_f_abs_an.frag"), false, [0x2060bf40; 4], 0x5ca71dc5),
    ("na", include_str!("../../../testdata/sel_f_abs_na.frag"), false, [0x00000000; 4], 0x38699dc5),
    ("as", include_str!("../../../testdata/sel_f_abs_as.frag"), false, [0x9fdf40bf; 4], 0x48631dc5),
    ("aa", include_str!("../../../testdata/sel_f_abs_aa.frag"), false, [0x60204040; 4], 0x7b699dc5),
    ("oa", include_str!("../../../testdata/sel_f_abs_oa.frag"), false, [0x00008000; 4], 0xa0699dc5),
    ("am", include_str!("../../../testdata/sel_f_abs_am.frag"), false, [0x10306020; 4], 0x3b499dc5),
];

/// vcomp.c's v_color (and a_color of cmp_vs2x4.vert's v_a), fsconst.c's
/// and fneg.c's.
const VCOMP_COLOR: [f32; 4] = [0.25, 0.75, 0.375, 0.125];
/// fabs.c's v_color: three lanes negative, so a missing fabs shows.
const FABS_COLOR: [f32; 4] = [-0.25, 0.75, -0.375, -0.125];

/// points.c's window (x, y) of each point, as clip space.
const POINT_XY: [f32; 6] = [cx(8.0), cx(8.0), cx(32.0), cx(32.0), cx(56.0), cx(56.0)];
const POINT_STRIP: [f32; 8] = [-1.0, -1.0, 0.0, -1.0, -1.0, 1.0, 0.0, 1.0];
const POINT_SIZES: [f32; 3] = [2.0, 5.0, 8.0];
const PSIZ_CONST_VS: &str = "attribute vec2 pos; void main(){ gl_Position = vec4(pos, 0.0, 1.0); gl_PointSize = 4.0; }";

/// points.c's modes: the name, the draw (its vertex shader and point size
/// source), and the board's Mesa read-back
/// (0dce654a8:tools/mali/board-ref/2026-10-19-points/mesa, `pixels 4096: red N other
/// 0 fnv1a H`).
const POINT_MODES: [(&str, Prim); 5] = [
    (
        "const",
        Prim {
            gl: GL_POINTS,
            vertices: &POINT_XY,
            vs: PSIZ_CONST_VS,
            size: PointSize::Shader,
            red: 48,
            fnv1a: 0x6b293745,
        },
    ),
    (
        "attr",
        Prim {
            gl: GL_POINTS,
            vertices: &POINT_XY,
            vs: "attribute vec2 pos; attribute float size; void main(){ gl_Position = vec4(pos, 0.0, 1.0); gl_PointSize = size; }",
            size: PointSize::Attribute(&POINT_SIZES),
            red: 93,
            fnv1a: 0xa70f8bcd,
        },
    ),
    (
        "uniform",
        Prim {
            gl: GL_POINTS,
            vertices: &POINT_XY,
            vs: "attribute vec2 pos; uniform float u_size; void main(){ gl_Position = vec4(pos, 0.0, 1.0); gl_PointSize = u_size; }",
            size: PointSize::Uniform(3.0),
            red: 27,
            fnv1a: 0x837d097d,
        },
    ),
    (
        "tri_psiz",
        Prim {
            gl: GL_TRIANGLE_STRIP,
            vertices: &POINT_STRIP,
            vs: PSIZ_CONST_VS,
            size: PointSize::Shader,
            red: 2048,
            fnv1a: 0xf350fdc5,
        },
    ),
    (
        "nopsiz",
        Prim {
            gl: GL_POINTS,
            vertices: &POINT_XY,
            vs: "attribute vec2 pos; void main(){ gl_Position = vec4(pos, 0.0, 1.0); }",
            size: PointSize::Shader,
            red: 3,
            fnv1a: 0x33b163bd,
        },
    ),
];

/// tex.c's wraps.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wrap {
    Clamp,
    Repeat,
    Mirror,
}

/// One tex.c mode: its flag, the texture's side, its filter and wrap, and
/// Mesa's FNV-1a of the read-back on the board
/// (0dce654a8:tools/mali/board-ref/2026-10-19-texture/mesa, `mesa-<mode>-<side>`; the
/// mirrored ones 2026-10-19-texture-mips/mesa, `mesa-tex-<mode>-<side>`).
struct TexMode {
    flag: &'static str,
    what: &'static str,
    side: usize,
    linear: bool,
    wrap: Wrap,
    fnv1a: u32,
}

const TEX_MODES: [TexMode; 12] = [
    TexMode {
        flag: "--tex-nearest-clamp",
        what: "GL tex.c nearest_clamp, 8x8 (expect 4096 of 4096 as predicted, Mesa's fnv1a 3d8d5dc5)",
        side: 8,
        linear: false,
        wrap: Wrap::Clamp,
        fnv1a: 0x3d8d5dc5,
    },
    TexMode {
        flag: "--tex-linear-clamp",
        what: "GL tex.c linear_clamp, 8x8 (expect 4096 of 4096 as predicted, Mesa's fnv1a 68741085)",
        side: 8,
        linear: true,
        wrap: Wrap::Clamp,
        fnv1a: 0x68741085,
    },
    TexMode {
        flag: "--tex-nearest-repeat",
        what: "GL tex.c nearest_repeat, 8x8 (expect 4096 of 4096 as predicted, Mesa's fnv1a 92bdddc5)",
        side: 8,
        linear: false,
        wrap: Wrap::Repeat,
        fnv1a: 0x92bdddc5,
    },
    TexMode {
        flag: "--tex-linear-repeat",
        what: "GL tex.c linear_repeat, 8x8 (expect 4096 of 4096 as predicted, Mesa's fnv1a 4099cac5)",
        side: 8,
        linear: true,
        wrap: Wrap::Repeat,
        fnv1a: 0x4099cac5,
    },
    TexMode {
        flag: "--tex-nearest-clamp-16",
        what: "GL tex.c nearest_clamp, 16x16 (expect 4096 of 4096 as predicted, Mesa's fnv1a 00e0ddc5)",
        side: 16,
        linear: false,
        wrap: Wrap::Clamp,
        fnv1a: 0x00e0ddc5,
    },
    TexMode {
        flag: "--tex-linear-clamp-16",
        what: "GL tex.c linear_clamp, 16x16 (expect 4096 of 4096 as predicted, Mesa's fnv1a 6568f675)",
        side: 16,
        linear: true,
        wrap: Wrap::Clamp,
        fnv1a: 0x6568f675,
    },
    TexMode {
        flag: "--tex-nearest-repeat-16",
        what: "GL tex.c nearest_repeat, 16x16 (expect 4096 of 4096 as predicted, Mesa's fnv1a 48ed1dc5)",
        side: 16,
        linear: false,
        wrap: Wrap::Repeat,
        fnv1a: 0x48ed1dc5,
    },
    TexMode {
        flag: "--tex-linear-repeat-16",
        what: "GL tex.c linear_repeat, 16x16 (expect 4096 of 4096 as predicted, Mesa's fnv1a bf1b35c5)",
        side: 16,
        linear: true,
        wrap: Wrap::Repeat,
        fnv1a: 0xbf1b35c5,
    },
    TexMode {
        flag: "--tex-nearest-mirror",
        what: "GL tex.c nearest_mirror, 8x8 (expect 4096 of 4096 as predicted, Mesa's fnv1a 94881dc5)",
        side: 8,
        linear: false,
        wrap: Wrap::Mirror,
        fnv1a: 0x94881dc5,
    },
    TexMode {
        flag: "--tex-linear-mirror",
        what: "GL tex.c linear_mirror, 8x8 (expect 4096 of 4096 as predicted, Mesa's fnv1a af430a45)",
        side: 8,
        linear: true,
        wrap: Wrap::Mirror,
        fnv1a: 0xaf430a45,
    },
    TexMode {
        flag: "--tex-nearest-mirror-16",
        what: "GL tex.c nearest_mirror, 16x16 (expect 4096 of 4096 as predicted, Mesa's fnv1a 6f985dc5)",
        side: 16,
        linear: false,
        wrap: Wrap::Mirror,
        fnv1a: 0x6f985dc5,
    },
    TexMode {
        flag: "--tex-linear-mirror-16",
        what: "GL tex.c linear_mirror, 16x16 (expect 4096 of 4096 as predicted, Mesa's fnv1a fd04af45)",
        side: 16,
        linear: true,
        wrap: Wrap::Mirror,
        fnv1a: 0xfd04af45,
    },
];

/// One mip.c mode: its flag, the chain's side, the minification and
/// magnification filters.
struct MipMode {
    flag: &'static str,
    what: &'static str,
    side: usize,
    min: GLenum,
    mag: GLenum,
}

/// mip.c's levels' colours, level 0 first.
const MIP_LEVEL: [[u8; 4]; 6] =
    [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255], [0, 255, 255, 255], [255, 0, 255, 255]];

/// Mesa's FNV-1a of every mip.c run on the board (all six: the picture is
/// the same, level b in band b; 0dce654a8:tools/mali/board-ref/2026-10-19-texture-mips/mesa).
const MIP_FNV1A: u32 = 0xc732cdc5;

const MIP_MODES: [MipMode; 6] = [
    MipMode {
        flag: "--mip-nearest",
        what: "GL mip.c nearest, 32x32 chain (expect 4096 as predicted, Mesa's fnv1a c732cdc5)",
        side: 32,
        min: GL_NEAREST_MIPMAP_NEAREST,
        mag: GL_NEAREST,
    },
    MipMode {
        flag: "--mip-linear-nearest",
        what: "GL mip.c linear_nearest, 32x32 chain (expect 4096 as predicted, Mesa's fnv1a c732cdc5)",
        side: 32,
        min: GL_LINEAR_MIPMAP_NEAREST,
        mag: GL_LINEAR,
    },
    MipMode {
        flag: "--mip-trilinear",
        what: "GL mip.c trilinear, 32x32 chain (expect 4096 as predicted, Mesa's fnv1a c732cdc5)",
        side: 32,
        min: GL_LINEAR_MIPMAP_LINEAR,
        mag: GL_LINEAR,
    },
    MipMode {
        flag: "--mip-nearest-8",
        what: "GL mip.c nearest, 8x8 chain (expect 4096 as predicted, Mesa's fnv1a c732cdc5)",
        side: 8,
        min: GL_NEAREST_MIPMAP_NEAREST,
        mag: GL_NEAREST,
    },
    MipMode {
        flag: "--mip-linear-nearest-8",
        what: "GL mip.c linear_nearest, 8x8 chain (expect 4096 as predicted, Mesa's fnv1a c732cdc5)",
        side: 8,
        min: GL_LINEAR_MIPMAP_NEAREST,
        mag: GL_LINEAR,
    },
    MipMode {
        flag: "--mip-trilinear-8",
        what: "GL mip.c trilinear, 8x8 chain (expect 4096 as predicted, Mesa's fnv1a c732cdc5)",
        side: 8,
        min: GL_LINEAR_MIPMAP_LINEAR,
        mag: GL_LINEAR,
    },
];

/// fmt.c's modes: the flag and Mesa's FNV-1a on the board
/// (0dce654a8:tools/mali/board-ref/2026-10-19-texture-formats/mesa).
const FMT_MODES: [(&str, &str, u32); 3] = [
    ("--fmt-etc1", "GL fmt.c etc1, 16x8 ETC1 (expect 4096 as predicted, Mesa's fnv1a 76cc0dc5)", 0x76cc0dc5),
    (
        "--fmt-etc2rgba",
        "GL fmt.c etc2rgba, 8x8 ETC2 RGBA8 (expect 4096 as predicted, Mesa's fnv1a dd58ddc5)",
        0xdd58ddc5,
    ),
    ("--fmt-srgb", "GL fmt.c srgb, 4x4 sRGB8_A8 (expect 4096 as predicted, Mesa's fnv1a 991e59c5)", 0x991e59c5),
];

/// fmt.c's block colours (4 bits a channel) and EAC alphas.
const FMT_COLS: [[u8; 3]; 8] =
    [[15, 0, 0], [0, 15, 0], [0, 0, 15], [15, 15, 15], [15, 15, 0], [0, 15, 15], [15, 0, 15], [8, 8, 8]];
const FMT_ALPHAS: [u8; 4] = [255, 128, 64, 0];
/// fmt.c's sRGB values and their linear values (round(255 lin(v / 255)),
/// ES 3.0 eq. 3.24, double precision).
const FMT_SRGB: [u8; 16] = [0, 10, 20, 40, 60, 80, 100, 120, 140, 160, 180, 200, 220, 235, 250, 255];
const FMT_LINEAR: [u8; 16] = [0, 1, 2, 5, 12, 20, 32, 48, 67, 90, 116, 147, 183, 212, 244, 255];

/// A flat ETC1 block: the base colour, table 0, every index 0.
fn etc1_flat(c: [u8; 3]) -> [u8; 8] {
    [c[0] << 4 | c[0], c[1] << 4 | c[1], c[2] << 4 | c[2], 0, 0, 0, 0, 0]
}

/// A flat block's decoded channel: c * 17 + 2, at most 255.
fn etc1_value(c: u8) -> u8 {
    (u32::from(c) * 17 + 2).min(255) as u8
}

const TEX_VS: &str = include_str!("../../../testdata/sel_t_tex.vert");
const TEX_FS: &str = include_str!("../../../testdata/sel_t_tex.frag");

impl TexMode {
    /// Texel channel value for texel index k.
    fn channel(k: usize) -> u8 {
        (16 * k + 8) as u8
    }

    /// The texel index for a texel coordinate index n under the wrap.
    fn wrap(&self, n: i32) -> usize {
        let t = self.side as i32;
        (match self.wrap {
            Wrap::Repeat => n.rem_euclid(t),
            Wrap::Mirror => {
                let m = n.rem_euclid(2 * t);
                if m < t {
                    m
                } else {
                    2 * t - 1 - m
                }
            }
            Wrap::Clamp => n.clamp(0, t - 1),
        }) as usize
    }

    /// tex.c's `predict`: the 8-bit value along one axis at pixel p.
    fn predict(&self, p: usize) -> u8 {
        /* s in units of 1/d texel: (2p + 1) - 32, d = 64 / side */
        let d = (64 / self.side) as i32;
        let sd = 2 * p as i32 + 1 - 32;
        if !self.linear {
            return Self::channel(self.wrap(sd.div_euclid(d)));
        }
        let td = sd - d / 2;
        let n0 = td.div_euclid(d);
        let wd = td - d * n0;
        let (a, b) = (i32::from(Self::channel(self.wrap(n0))), i32::from(Self::channel(self.wrap(n0 + 1))));
        (a + (b - a) * wd / d) as u8
    }
}
/// poff.c's modes: the name, the tilted plane, glPolygonOffset's (factor,
/// units), and the board's Mesa read-back
/// (0dce654a8:tools/mali/board-ref/2026-10-19-polyoffset/mesa).
const POFFS: [(&str, bool, f32, f32, u32); 7] = [
    ("flat_0_m4", false, 0.0, -4.0, 0xc38d1dc5),
    ("flat_0_4", false, 0.0, 4.0, 0xbdf93dc5),
    ("flat_0_m1", false, 0.0, -1.0, 0xc38d1dc5),
    ("flat_m1_0", false, -1.0, 0.0, 0xbdf93dc5),
    ("tilt_m1_0", true, -1.0, 0.0, 0xc38d1dc5),
    ("tilt_1_0", true, 1.0, 0.0, 0xbdf93dc5),
    ("tilt_m0p5_m1", true, -0.5, -1.0, 0xc38d1dc5),
];

/// blend.c's modes and the board's Mesa read-back of each
/// (0dce654a8:tools/mali/board-ref/2026-10-19-blend/mesa, `pixels 4096: fnv1a H`).
const BLENDS: [(&str, u32); 10] = [
    ("over", 0x99996dc5),
    ("add", 0x026ffdc5),
    ("premul", 0xba37fdc5),
    ("replace", 0xffc16dc5),
    ("sub", 0x09c16dc5),
    ("revsub", 0x30f81dc5),
    ("mul", 0x39915dc5),
    ("twosd", 0x32181dc5),
    ("const", 0x711e1dc5),
    ("sep", 0x52196dc5),
];

/// The top-down target's size: flip.c's pbuffer, not square, so that a
/// height used for a width shows.
const FLIP_W: usize = 64;
const FLIP_H: usize = 48;

impl Mode {
    /// A flip.c mode: Mesa's read-back on the board
    /// (0dce654a8:tools/mali/board-ref/2026-10-19-flip/mesa, the pbuffer runs:
    /// `pixels 3072: red N other 0 fnv1a H`).
    fn flip(self) -> Option<(usize, u32)> {
        Some(match self {
            Mode::FlipTri | Mode::FlipFrontCw => (496, 0x890abd45),
            Mode::FlipViewport => (512, 0xfd5285c5),
            Mode::FlipScissorDraw | Mode::FlipScissorClear => (320, 0x073d8bc5),
            _ => return None,
        })
    }
}

/// prims.c's window x or y as clip space (x = 32 * (clip + 1)).
const fn cx(x: f32) -> f32 {
    x / 32.0 - 1.0
}

/// One primitive mode of prims.c: the GL mode, its vertices, and Mesa's
/// read-back on the board (0dce654a8:tools/mali/board-ref/2026-10-19-primitives/mesa,
/// `pixels 4096: red N other 0 fnv1a H`).
struct Prim {
    gl: GLenum,
    vertices: &'static [f32],
    /// The vertex shader (prims.c's modes: tri.vert, which draws the same).
    vs: &'static str,
    size: PointSize,
    red: usize,
    fnv1a: u32,
}

/// Where a points.c shader's gl_PointSize comes from beside the shader.
#[derive(Clone, Copy)]
enum PointSize {
    /// The shader alone (a constant, or not written).
    Shader,
    /// Attribute 1, `size`, per vertex.
    Attribute(&'static [f32]),
    /// The uniform `u_size`.
    Uniform(f32),
}

impl Mode {
    fn prim(self) -> Option<Prim> {
        const LINES: [f32; 8] = [cx(0.0), cx(20.5), cx(64.0), cx(20.5), cx(40.5), cx(0.0), cx(40.5), cx(64.0)];
        const LINE_STRIP: [f32; 6] = [cx(0.0), cx(20.5), cx(40.5), cx(20.5), cx(40.5), cx(64.0)];
        const LINE_LOOP: [f32; 6] = [cx(8.5), cx(8.5), cx(56.5), cx(8.5), cx(32.5), cx(56.5)];
        const TRI_STRIP: [f32; 8] = [-1.0, -1.0, 0.0, -1.0, -1.0, 1.0, 0.0, 1.0];
        const TRI_FAN: [f32; 8] = [0.0, -1.0, 1.0, -1.0, 1.0, 1.0, 0.0, 1.0];
        let (gl, vertices, red, fnv1a): (GLenum, &'static [f32], usize, u32) = match self {
            Mode::Lines => (GL_LINES, &LINES, 127, 0xda93439d),
            Mode::LineStrip => (GL_LINE_STRIP, &LINE_STRIP, 84, 0x793220e5),
            Mode::LineLoop => (GL_LINE_LOOP, &LINE_LOOP, 144, 0x10261645),
            Mode::TriStrip => (GL_TRIANGLE_STRIP, &TRI_STRIP, 2048, 0xf350fdc5),
            Mode::TriFan => (GL_TRIANGLE_FAN, &TRI_FAN, 2048, 0x8eb2fdc5),
            _ => return None,
        };
        Some(Prim { gl, vertices, vs: TRI_VS, size: PointSize::Shader, red, fnv1a })
    }
}

impl Mode {
    /// The modes of zs.c and the scissored clear: a D24S8 on the FBO.
    fn zs(self) -> bool {
        matches!(
            self,
            Mode::Depth
                | Mode::NoDepth
                | Mode::ClearHide
                | Mode::Mask
                | Mode::NoColour
                | Mode::ScissorClear
                | Mode::PreloadZs(_)
                | Mode::Poff(_)
        )
    }
}

const RED: [u8; 4] = [0xff, 0, 0, 0xff];
const GREEN: [u8; 4] = [0, 0xff, 0, 0xff];
const BLUE: [u8; 4] = [0, 0, 0xff, 0xff];
const MAGENTA: [u8; 4] = [0xff, 0, 0xff, 0xff];

/// What a zs mode predicts at (x, y) (row 0 is GL's bottom).
fn zs_want(mode: Mode, x: usize, y: usize) -> [u8; 4] {
    match mode {
        Mode::Depth if x < 16 => RED,
        Mode::Depth => GREEN,
        Mode::NoDepth if x < 48 => RED,
        Mode::NoDepth => GREEN,
        Mode::NoColour if x < 16 => RED,
        Mode::Mask => MAGENTA,
        Mode::ScissorClear if (16..48).contains(&x) && (16..48).contains(&y) => RED,
        Mode::PreloadZs(_) if x < 32 => RED,
        Mode::PreloadZs(_) => GREEN,
        _ => BLUE,
    }
}

/// zs.c's vertex shader: the position at its own z.
const ZS_VS: &str = "attribute vec3 pos; void main(){ gl_Position = vec4(pos, 1.0); }";

/// Draw zs.c's quad with the colour uniform `loc`. The client array may go
/// after the draw: the backend copies it into the open frame.
/// The quad covers x [x0, x1], the full height, at clip z `z`.
unsafe fn zs_quad(loc: GLint, x0: f32, x1: f32, z: f32, rgb: [f32; 3]) {
    let v = [x0, -1.0, z, x1, -1.0, z, x0, 1.0, z, x1, -1.0, z, x1, 1.0, z, x0, 1.0, z];
    // SAFETY: the caller's GL context; the array outlives the draw.
    unsafe {
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, v.as_ptr().cast());
        glEnableVertexAttribArray(0);
        glUniform4f(loc, rgb[0], rgb[1], rgb[2], 1.0);
        glDrawArrays(GL_TRIANGLES, 0, 6);
    }
}

/// The GL calls of a zs mode after the FBO is bound (zs.c's `main` from its
/// clear on).
unsafe fn zs_frame(mode: Mode) -> Result<(), String> {
    // SAFETY: the caller's GL context.
    unsafe {
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClearDepthf(1.0);
        glClearStencil(0x5a);
        if matches!(mode, Mode::Mask | Mode::ScissorClear) {
            glClear(GL_COLOR_BUFFER_BIT);
            if mode == Mode::Mask {
                glColorMask(1, 0, 0, 0);
                glClearColor(1.0, 1.0, 1.0, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
                glColorMask(1, 1, 1, 1);
            } else {
                glEnable(GL_SCISSOR_TEST);
                glScissor(16, 16, 32, 32);
                glClearColor(1.0, 0.0, 0.0, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
                glDisable(GL_SCISSOR_TEST);
            }
            return gl_error("the clears");
        }
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, ZS_VS)?);
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, COLOR_FS)?);
        glBindAttribLocation(p, 0, c"pos".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
        }
        glUseProgram(p);
        let col = glGetUniformLocation(p, c"u_color".as_ptr());
        if col < 0 {
            return Err("u_color is not an active uniform".into());
        }
        if let Mode::PreloadZs(k) = mode {
            /* preload_zs.c: frame 1 leaves columns 0-31 at depth 0.5 and
             * stencil 1; frame 2 starts from them */
            glClearStencil(0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
            glDepthMask(1);
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_ALWAYS, 1, 0xff);
            glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
            zs_quad(col, -1.0, 0.0, 0.0, [1.0, 0.0, 0.0]);
            glFinish();
            let (stencil, part) = PRELOAD_ZS_MODES[k].2;
            if part {
                glClear(if stencil { GL_DEPTH_BUFFER_BIT } else { GL_STENCIL_BUFFER_BIT });
            }
            if stencil {
                glDisable(GL_DEPTH_TEST);
                glStencilFunc(GL_EQUAL, 0, 0xff);
                glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
            } else {
                glDisable(GL_STENCIL_TEST);
            }
            zs_quad(col, -1.0, 1.0, 0.5, [0.0, 1.0, 0.0]);
        } else if mode == Mode::ClearHide {
            glClearDepthf(0.25);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
            zs_quad(col, -1.0, 1.0, 0.0, [1.0, 0.0, 0.0]);
        } else if mode == Mode::NoColour {
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
            glColorMask(0, 0, 0, 0);
            zs_quad(col, -0.5, 1.0, -0.5, [0.0, 1.0, 0.0]);
            glColorMask(1, 1, 1, 1);
            zs_quad(col, -1.0, 1.0, 0.5, [1.0, 0.0, 0.0]);
        } else {
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            if mode == Mode::Depth {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LESS);
            }
            zs_quad(col, -0.5, 1.0, -0.5, [0.0, 1.0, 0.0]);
            zs_quad(col, -1.0, 0.5, 0.5, [1.0, 0.0, 0.0]);
        }
        gl_error("the draws")
    }
}

/// `--color-uv`'s attributes, per vertex of [`l1::VERTICES`]: a_color
/// (vec4) and a_uv (vec2).
fn color_uv_vertices() -> (Vec<f32>, Vec<f32>) {
    let (mut color, mut uv) = (Vec::new(), Vec::new());
    for v in l1::VERTICES.chunks(2) {
        let (x, y) = (v[0], v[1]);
        color.extend([1.0, 0.5, (y + 1.0) / 2.0, 1.0]);
        uv.extend([x + 1.0, (y + 1.0) / 2.0]);
    }
    (color, uv)
}

/// `--color-uv`'s prediction of pixel (x, y) (row 0 is GL's bottom).
fn color_uv_want(x: usize, y: usize) -> [u8; 4] {
    if x >= W / 2 {
        return [0, 0, 0xff, 0xff];
    }
    /* round(n 255 / d) for an odd n: never a tie */
    let unorm = |n: usize, d: usize| ((2 * n * 255 + d) / (2 * d)) as u8;
    [unorm(2 * x + 1, 64), unorm(2 * y + 1, 256), unorm(2 * y + 1, 128), 0xff]
}

/// A pixel that differs from the prediction: x, y, read, predicted.
type Difference = (usize, usize, [u8; 4], [u8; 4]);

/// `--color-uv`'s comparison: (pixels equal to the prediction, the largest
/// channel difference, the first differing pixel).
fn color_uv_check(px: &[u8]) -> (usize, u8, Option<Difference>) {
    let (mut equal, mut worst, mut first) = (0, 0u8, None);
    for y in 0..H {
        for x in 0..W {
            let got: [u8; 4] = px[(y * W + x) * 4..][..4].try_into().unwrap();
            let want = color_uv_want(x, y);
            let d = got.iter().zip(want).map(|(a, b)| a.abs_diff(b)).max().unwrap_or(0);
            if d == 0 {
                equal += 1;
            } else if first.is_none() {
                first = Some((x, y, got, want));
            }
            worst = worst.max(d);
        }
    }
    (equal, worst, first)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Tally {
    red: usize,
    green: usize,
    blue: usize,
    other: usize,
    fill: usize,
    misplaced: usize,
}

/// Count the read-back (row 0 is GL's bottom): what the frame should put
/// where is `ink` over the columns `cols` with a draw, blue elsewhere.
fn tally(px: &[u8], ink: Option<[u8; 4]>, cols: core::ops::Range<usize>) -> Tally {
    let mut t = Tally::default();
    for y in 0..H {
        for x in 0..W {
            let p: [u8; 4] = px[(y * W + x) * 4..][..4].try_into().unwrap();
            match p {
                [0xff, 0, 0, 0xff] => t.red += 1,
                [0, 0xff, 0, 0xff] => t.green += 1,
                [0, 0, 0xff, 0xff] => t.blue += 1,
                _ => t.other += 1,
            }
            if p == l1::FILL {
                t.fill += 1;
            }
            let want = match ink {
                Some(c) if cols.contains(&x) => c,
                _ => [0, 0, 0xff, 0xff],
            };
            if p != want {
                t.misplaced += 1;
            }
        }
    }
    t
}

fn info_log(get: unsafe extern "C" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar), o: GLuint) -> String {
    let mut buf = [0u8; 1024];
    let mut n: GLsizei = 0;
    // SAFETY: a buffer of the size passed.
    unsafe { get(o, buf.len() as GLsizei, &mut n, buf.as_mut_ptr().cast()) };
    String::from_utf8_lossy(&buf[..n.max(0) as usize]).into_owned()
}

fn shader(ty: GLenum, src: &str) -> Result<GLuint, String> {
    let text = CString::new(src).map_err(|e| e.to_string())?;
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            return Err(format!("compile: {}", info_log(glGetShaderInfoLog, s)));
        }
        Ok(s)
    }
}

fn gl_error(at: &str) -> Result<(), String> {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    if e != GL_NO_ERROR {
        return Err(format!("{at}: GL error {e:#x}"));
    }
    Ok(())
}

fn run(mode: Mode, fb: Option<&str>, scale: u32) -> Result<bool, String> {
    osal::install();
    let lost0 = yakogl::backend::yakogl_gpu_work_lost();
    let refused0 = yakogl::backend::yakogl_draws_refused();
    if let Mode::Msaa(samples) = mode {
        // SAFETY: plain EGL and GL calls on this thread, in order.
        return unsafe { msaa_frame(samples, None, lost0, refused0) };
    }
    if mode == Mode::Pcoord(true) {
        // SAFETY: plain EGL and GL calls on this thread, in order.
        return unsafe { pcoord_pbuffer(lost0, refused0) };
    }
    if let Mode::MsaaCov(invert) = mode {
        // SAFETY: as above.
        return unsafe { msaa_frame(4, Some(invert), lost0, refused0) };
    }
    if let Some(w) = mode.window() {
        // SAFETY: plain EGL and GL calls on this thread, in order.
        return unsafe { window_frame(&w, lost0, refused0) };
    }
    if matches!(mode, Mode::FlipManyLines | Mode::FlipManyLinesColour1) {
        // SAFETY: plain GL calls on this thread, in order.
        return unsafe { manylines_frame(mode == Mode::FlipManyLinesColour1, lost0, refused0) };
    }
    if let Some(want) = mode.flip() {
        // SAFETY: plain GL calls on this thread, in order.
        return unsafe { flip_frame(mode, want, lost0, refused0) };
    }
    if let Mode::FragCoord(flip) = mode {
        // SAFETY: plain GL calls on this thread, in order.
        return unsafe { fragcoord_frame(flip, lost0, refused0) };
    }
    // SAFETY: plain EGL and GL calls on this thread, in order.
    unsafe {
        let dpy = eglGetDisplay(core::ptr::null_mut());
        if eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()) == 0 {
            return Err("eglInitialize failed (the OSAL or the backend did not come up)".into());
        }
        let cfg_attrs = [EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE];
        let mut cfg: EGLConfig = core::ptr::null_mut();
        let mut n = 0;
        if eglChooseConfig(dpy, cfg_attrs.as_ptr(), &mut cfg, 1, &mut n) == 0 || n < 1 {
            return Err("eglChooseConfig: no pbuffer ES 2 config".into());
        }
        let surf_attrs = [EGL_WIDTH, W as EGLint, EGL_HEIGHT, H as EGLint, EGL_NONE];
        let surf = eglCreatePbufferSurface(dpy, cfg, surf_attrs.as_ptr());
        let ctx_attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let ctx = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_attrs.as_ptr());
        if surf.is_null() || ctx.is_null() || eglMakeCurrent(dpy, surf, surf, ctx) == 0 {
            return Err("pbuffer, context or eglMakeCurrent failed".into());
        }

        let fill: Vec<u8> = (0..W * H).flat_map(|_| l1::FILL).collect();
        let (mut tex, mut fbo) = (0, 0);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            W as GLsizei,
            H as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            fill.as_ptr().cast(),
        );
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        if mode.zs() {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8_OES, W as GLsizei, H as GLsizei);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
        }
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        if status != GL_FRAMEBUFFER_COMPLETE {
            return Err(format!("FBO status {status:#x}"));
        }
        gl_error("FBO")?;
        glViewport(0, 0, W as GLsizei, H as GLsizei);
        if let Mode::Poff(i) = mode {
            return poff_frame(i, lost0, refused0);
        }
        if mode.zs() {
            zs_frame(mode)?;
            return finish_zs(mode, fb, scale, lost0, refused0);
        }
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        gl_error("glClear")?;
        if mode == Mode::DeleteHeld {
            return delete_held(tex, lost0, refused0);
        }
        if let Mode::Tex(i) = mode {
            return tex_frame(&TEX_MODES[i], lost0, refused0);
        }
        if let Mode::TexSolid(w) = mode {
            return tex_solid_frame(w, lost0, refused0);
        }
        if let Mode::Mip(i) = mode {
            return mip_frame(&MIP_MODES[i], lost0, refused0);
        }
        if let Mode::Fmt(i) = mode {
            return fmt_frame(i, lost0, refused0);
        }
        if let Mode::Preload(strip) = mode {
            return preload_frame(strip, lost0, refused0);
        }
        if let Mode::Blend(i) = mode {
            return blend_frame(i, lost0, refused0);
        }
        if matches!(
            mode,
            Mode::IdxU16Client | Mode::IdxU16Ibo | Mode::IdxU16Offset | Mode::IdxStripIbo | Mode::IdxU8Ibo
        ) {
            return idx_frame(mode, lost0, refused0);
        }
        if let Some(prim) = mode.prim() {
            return prim_frame(&prim, lost0, refused0);
        }
        if let Mode::Points(i) = mode {
            return prim_frame(&POINT_MODES[i].1, lost0, refused0);
        }
        if mode == Mode::Pcoord(false) {
            return pcoord_frame(lost0, refused0);
        }
        if let Mode::Vcomp(i) = mode {
            return vcomp_frame(&VCOMP_MODES[i], VCOMP_COLOR, lost0, refused0);
        }
        if let Mode::Fsconst(i) = mode {
            return vcomp_frame(&FSCONST_MODES[i], VCOMP_COLOR, lost0, refused0);
        }
        if let Mode::Fneg(i) = mode {
            return vcomp_frame(&FNEG_MODES[i], VCOMP_COLOR, lost0, refused0);
        }
        if let Mode::Fabs(i) = mode {
            return vcomp_frame(&FABS_MODES[i], FABS_COLOR, lost0, refused0);
        }
        if mode == Mode::FvgFillColor {
            return fvg_fill_color_frame(lost0, refused0);
        }
        if mode == Mode::FvgFillColorScissor {
            return fvg_fill_color_scissor_frames(lost0, refused0);
        }
        let ink = match mode {
            Mode::ClearOnly | Mode::DeleteHeld => None,
            Mode::Red => Some([0xff, 0, 0, 0xff]),
            Mode::Green | Mode::Varying | Mode::UniformGreen => Some([0, 0xff, 0, 0xff]),
            Mode::UniformRed | Mode::Mat4 => Some([0xff, 0, 0, 0xff]),
            Mode::ColorUv => None,
            _ => return Err("a zs mode reached the triangle path".into()),
        };
        let cols = if mode == Mode::Mat4 { 16..48 } else { 0..W / 2 };
        /* client arrays: they live until the read-back, where the frame that
         * reads them is built */
        let (color, uv) = color_uv_vertices();
        if mode != Mode::ClearOnly {
            let vs = match mode {
                Mode::Varying => TV_VS,
                Mode::Mat4 => MAT4_VS,
                Mode::ColorUv => CUV_VS,
                _ => TRI_VS,
            };
            let fs = if mode == Mode::ColorUv {
                CUV_FS.to_string()
            } else if mode == Mode::Varying {
                TV_FS.to_string()
            } else if matches!(mode, Mode::UniformGreen | Mode::UniformRed) {
                COLOR_FS.to_string()
            } else if mode == Mode::Green {
                let g = TRI_FS.replace("vec4(1.0, 0.0, 0.0, 1.0)", "vec4(0.0, 1.0, 0.0, 1.0)");
                if g == TRI_FS {
                    return Err("tri.frag has no red constant to change".into());
                }
                g
            } else {
                TRI_FS.to_string()
            };
            println!("panthor-gl: fragment shader: {}", fs.lines().last().unwrap_or(""));
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, vs)?);
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs)?);
            /* tri_varying's two attributes at the locations its board program
             * (tri_varying.c) gives them */
            let pos = if matches!(mode, Mode::Mat4 | Mode::ColorUv) { c"a_pos" } else { c"pos" };
            glBindAttribLocation(p, 0, pos.as_ptr());
            if mode == Mode::Varying {
                glBindAttribLocation(p, 1, c"col".as_ptr());
            }
            /* color_uv's declaration order, as gl_test binds them */
            if mode == Mode::ColorUv {
                glBindAttribLocation(p, 1, c"a_color".as_ptr());
                glBindAttribLocation(p, 2, c"a_uv".as_ptr());
            }
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            if ok != 1 {
                return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
            }
            glUseProgram(p);
            match mode {
                Mode::UniformGreen | Mode::UniformRed => {
                    let l = glGetUniformLocation(p, c"u_color".as_ptr());
                    if l < 0 {
                        return Err("u_color is not an active uniform".into());
                    }
                    let (r, g) = if mode == Mode::UniformRed { (1.0, 0.0) } else { (0.0, 1.0) };
                    glUniform4f(l, r, g, 0.0, 1.0);
                    println!("panthor-gl: glUniform4f(u_color, {r}, {g}, 0, 1)");
                }
                Mode::Mat4 => {
                    let l = glGetUniformLocation(p, c"u_mvp".as_ptr());
                    if l < 0 {
                        return Err("u_mvp is not an active uniform".into());
                    }
                    glUniformMatrix4fv(l, 1, 0, MAT4_SHIFT.as_ptr());
                    println!("panthor-gl: glUniformMatrix4fv(u_mvp, translate +0.5 x)");
                }
                _ => {}
            }
            gl_error("glUniform")?;
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, l1::VERTICES.as_ptr().cast());
            if mode == Mode::Varying {
                glEnableVertexAttribArray(1);
                glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, GREEN_VERTICES.as_ptr().cast());
            }
            if mode == Mode::ColorUv {
                glEnableVertexAttribArray(1);
                glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, color.as_ptr().cast());
                glEnableVertexAttribArray(2);
                glVertexAttribPointer(2, 2, GL_FLOAT, 0, 0, uv.as_ptr().cast());
            }
            glDrawArrays(GL_TRIANGLES, 0, 6);
            gl_error("glDrawArrays")?;
        }
        let mut px = vec![0u8; W * H * 4];
        glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        gl_error("glReadPixels")?;
        glFinish();

        let t = tally(&px, ink, cols);
        let cuv = (mode == Mode::ColorUv).then(|| color_uv_check(&px));
        if let Some((equal, worst, first)) = cuv {
            println!("panthor-gl: color_uv: {equal} of {} pixels equal to the prediction, largest channel difference {worst}", W * H);
            for (x, y) in [(0, 0), (31, 0), (0, 63), (31, 63), (15, 32), (32, 32)] {
                let got = &px[(y * W + x) * 4..][..4];
                println!("panthor-gl: color_uv: pixel ({x}, {y}) = {got:?}, predicted {:?}", color_uv_want(x, y));
            }
            if let Some((x, y, got, want)) = first {
                println!("panthor-gl: color_uv: first difference at ({x}, {y}): {got:?}, predicted {want:?}");
            }
        }
        let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
        let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
        let (subs, open, live, live_hw, bytes, bytes_hw) = osal::counts().ok_or("no OSAL state")?;
        let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
        let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
        /* the two-colour tally is the other modes' check; --color-uv's is
         * the per-pixel comparison above */
        if cuv.is_none() {
            println!(
                "panthor-gl: read-back: red {} green {} blue {} other {} (fill {}) misplaced {}",
                t.red, t.green, t.blue, t.other, t.fill, t.misplaced
            );
        }
        println!(
            "panthor-gl: gpu work lost {lost}, draws refused {refused}; osal submits {subs}, unwaited fences {open}, BOs live {live} (high-water {live_hw}), bytes live {bytes} (high-water {bytes_hw}); group state {:#x} fatal queues {:#x}; vm usable {usable}",
            gs.0, gs.1
        );
        if let Some(path) = fb {
            blit(path, &px, scale)?;
        }
        let want = match mode {
            Mode::Red | Mode::UniformRed | Mode::Mat4 => Tally { red: 2048, blue: 2048, ..Tally::default() },
            Mode::Green | Mode::Varying | Mode::UniformGreen => Tally { green: 2048, blue: 2048, ..Tally::default() },
            Mode::ClearOnly | Mode::DeleteHeld => Tally { blue: 4096, ..Tally::default() },
            Mode::ColorUv => {
                let equal = cuv.map_or(0, |c| c.0);
                return Ok(equal == W * H && lost == 0 && refused == 0 && gs == (0, 0) && usable);
            }
            _ => return Err("a zs mode reached the triangle path".into()),
        };
        Ok(t == want && lost == 0 && refused == 0 && gs == (0, 0) && usable)
    }
}

/// fragcoord.c's fragment shader (0dce654a8:tools/mali/board-ref/2026-10-22-s4-fragcoord).
const FRAGCOORD_FS: &str =
    "precision highp float; void main(){ vec2 q = gl_FragCoord.xy; gl_FragColor = vec4(q * 0.015625, 0.0, 1.0); }";

/// `--fragcoord`, `--fragcoord-flip`: fragcoord.c's draw (module docs) into
/// a 64x48 FBO or the top-down default surface; every pixel against the
/// prediction, the FNV-1a of the read-back against Mesa's, and for the
/// top-down surface its rows in memory against the read-back's mirrored.
unsafe fn fragcoord_frame(flip: bool, lost0: u32, refused0: u32) -> Result<bool, String> {
    const MESA_FNV1A: u32 = 0x99cb_74c5;
    top_down_context(FLIP_W as u32, FLIP_H as u32)?;
    if !flip {
        let (mut tex, mut fbo) = (0, 0);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            FLIP_W as GLsizei,
            FLIP_H as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            core::ptr::null(),
        );
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        if glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE {
            return Err("the 64x48 FBO is not complete".into());
        }
    }
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, TRI_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, FRAGCOORD_FS)?);
    glBindAttribLocation(p, 0, c"pos".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    glViewport(0, 0, FLIP_W as GLsizei, FLIP_H as GLsizei);
    glClearColor(0.0, 0.0, 1.0, 1.0);
    glClear(GL_COLOR_BUFFER_BIT);
    let quad: [f32; 12] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0];
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, quad.as_ptr().cast());
    glDrawArrays(GL_TRIANGLES, 0, 6);
    gl_error("the draw")?;
    let mut px = vec![0u8; FLIP_W * FLIP_H * 4];
    glReadPixels(0, 0, FLIP_W as GLsizei, FLIP_H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    /* fragcoord.c's prediction: (2k + 1) / 128 is exact in fp16 and never
     * half-way between two bytes */
    let unorm = |k: usize| (((2 * k + 1) * 255 + 64) / 128) as u8;
    let mut differ = 0usize;
    for (i, p) in px.chunks(4).enumerate() {
        let (x, y) = (i % FLIP_W, i / FLIP_W);
        let want = [unorm(x), unorm(y), 0, 255];
        if p != want {
            if differ < 8 {
                println!("panthor-gl: fragcoord: pixel ({x}, {y}) {p:?}, want {want:?}");
            }
            differ += 1;
        }
    }
    let word = |i: usize| u32::from_le_bytes(px[i * 4..][..4].try_into().unwrap());
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    println!(
        "panthor-gl: fragcoord: corners (0,0) {:08x} (63,0) {:08x} (0,47) {:08x} (63,47) {:08x}",
        word(0),
        word(FLIP_W - 1),
        word((FLIP_H - 1) * FLIP_W),
        word(FLIP_H * FLIP_W - 1)
    );
    println!(
        "panthor-gl: fragcoord: pixels {}: differ {differ} fnv1a {fnv1a:08x} (Mesa's: differ 0 fnv1a {MESA_FNV1A:08x})",
        FLIP_W * FLIP_H
    );
    /* the top-down surface holds the picture mirrored row for row */
    let mut stored = true;
    if flip {
        let surf = yakogl::gl::context::gl_default_surface();
        let cpu = (*surf).cpu as *const u8;
        if cpu.is_null() || (*surf).size < px.len() {
            return Err("the default surface has no CPU view of 64x48 pixels".into());
        }
        let raw = core::slice::from_raw_parts(cpu, px.len());
        let row = FLIP_W * 4;
        let mirrored = (0..FLIP_H).filter(|&y| raw[(FLIP_H - 1 - y) * row..][..row] == px[y * row..][..row]).count();
        println!(
            "panthor-gl: fragcoord: surface rows: {mirrored} of {FLIP_H} are the read-back's rows mirrored (top-down)"
        );
        stored = mirrored == FLIP_H;
    }
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    Ok(differ == 0 && fnv1a == MESA_FNV1A && stored && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// `--fvg-fill-color`: femtovg's solid fill, main.vert with
/// fill_color.nodiscard.noscissor.frag (tools/mali/testdata; #1384 S4), as
/// 0dce654a8:tools/mali/board-ref/2026-10-21-s4-alu/fvgfc.c draws it through the
/// board's Mesa: one full-screen triangle into the 64x64 FBO, `vertex` in
/// pixels and `tcoord` = vertex / 64, so u is the column and v the row in
/// [0, 1); viewSize (64, 64); frag[6] innerCol = (0, 1, 0, 1), frag[10].x
/// strokeMult = 1. The stroke mask `min(1, 1 - |2u - 1|) * min(1, v)`
/// scales innerCol: a tent along x times a ramp along y, in G and A; R and
/// B are zero. The read-back is hashed as gl_test's `read_topdown` does
/// (FNV-1a over the top-down RGBA bytes) and written whole to
/// fvgfc-ours.rgba in the working directory, so the picture is compared
/// with Mesa's byte for byte.
unsafe fn fvg_fill_color_frame(lost0: u32, refused0: u32) -> Result<bool, String> {
    glClearColor(0.1, 0.1, 0.25, 1.0);
    glClear(GL_COLOR_BUFFER_BIT);
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, FVG_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, FVG_FC_FS)?);
    glBindAttribLocation(p, 0, c"vertex".as_ptr());
    glBindAttribLocation(p, 1, c"tcoord".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    let view = glGetUniformLocation(p, c"viewSize".as_ptr());
    let frag = glGetUniformLocation(p, c"frag".as_ptr());
    if view < 0 || frag < 0 {
        return Err(format!("viewSize ({view}) or frag ({frag}) is not an active uniform"));
    }
    let mut block = [0f32; 56];
    block[6 * 4..6 * 4 + 4].copy_from_slice(&[0.0, 1.0, 0.0, 1.0]);
    block[10 * 4] = 1.0;
    glUniform2f(view, W as f32, H as f32);
    glUniform4fv(frag, 14, block.as_ptr());
    gl_error("glUniform")?;
    println!("panthor-gl: fvg-fill-color: viewSize loc={view} frag loc={frag}; innerCol (0, 1, 0, 1), strokeMult 1");
    /* fvgfc.c's vertices: (x, y, u, v) with u = x / 64, v = y / 64 */
    let v: [f32; 12] = [-64.0, -64.0, -1.0, -1.0, 192.0, -64.0, 3.0, -1.0, 64.0, 192.0, 1.0, 3.0];
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, 0, 16, v.as_ptr().cast());
    glEnableVertexAttribArray(1);
    glVertexAttribPointer(1, 2, GL_FLOAT, 0, 16, v.as_ptr().cast::<u8>().add(8).cast());
    glDrawArrays(GL_TRIANGLES, 0, 3);
    gl_error("glDrawArrays")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    /* gl_test's read_topdown: the rows top first, FNV-1a over the bytes */
    let mut top = vec![0u8; W * H * 4];
    for y in 0..H {
        top[y * W * 4..(y + 1) * W * 4].copy_from_slice(&px[(H - 1 - y) * W * 4..(H - y) * W * 4]);
    }
    let mut h = 2166136261u32;
    for b in &top {
        h = (h ^ u32::from(*b)).wrapping_mul(16777619);
    }
    let (mut shape, mut greens) = (0usize, std::collections::BTreeSet::new());
    for p in top.chunks(4) {
        if p[0] == 0 && p[2] == 0 && p[1] == p[3] {
            shape += 1;
        }
        greens.insert(p[1]);
    }
    let centre = &top[(H / 2 * W + W / 2) * 4..][..4];
    println!("panthor-gl: fvg-fill-color: readback fnv1a {h:08x}, centre (32, 32 top-down) {centre:?}");
    println!(
        "panthor-gl: fvg-fill-color: {shape} of {} pixels have R = B = 0 and G = A, {} distinct G values",
        W * H,
        greens.len()
    );
    for y in (0..H).step_by(8) {
        let row: String = (0..W)
            .step_by(2)
            .map(|x| top[(y * W + x) * 4 + 1])
            .map(|g| b" .:-=+*#%@"[g as usize / 26] as char)
            .collect();
        println!("panthor-gl: fvg-fill-color: G row {y:2} {row}");
    }
    match std::fs::write("fvgfc-ours.rgba", &top) {
        Ok(()) => println!("panthor-gl: fvg-fill-color: wrote fvgfc-ours.rgba ({} bytes, top-down RGBA)", top.len()),
        Err(e) => println!("panthor-gl: fvg-fill-color: fvgfc-ours.rgba not written: {e}"),
    }
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let (subs, open, live, live_hw, bytes, bytes_hw) = osal::counts().ok_or("no OSAL state")?;
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; osal submits {subs}, unwaited fences {open}, BOs live {live} (high-water {live_hw}), bytes live {bytes} (high-water {bytes_hw}); group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    Ok(shape == W * H && greens.len() > 16 && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// `--fvg-fill-color-scissor`: femtovg's solid fill under a scissor,
/// main.vert with fill_color.nodiscard.frag (tools/mali/testdata; #1384
/// S5, the `if` of scissorMask), as 0dce654a8:tools/mali/board-ref/2026-10-22-s5-if/
/// fvgfcs.c draws it through the board's Mesa: `--fvg-fill-color`'s
/// triangle and uniforms, plus the scissor: scissorMat the translation by
/// (-32, -32) (the scissor's space centred), scissorExt (20, 12),
/// scissorScale (1, 1). Two frames: scissorRadius 0 takes the else side of
/// the `if` (the rectangle), 6 the then side (the rounded rectangle). The
/// stroke mask times the scissor scales innerCol: G and A, R and B zero,
/// zero outside the 40x24 window; the two pictures differ within 8 pixels
/// of a window corner only. Each read-back is hashed as gl_test's
/// `read_topdown` does and written whole (fvgfcs-rect-ours.rgba,
/// fvgfcs-round-ours.rgba), so the pictures are compared with Mesa's byte
/// for byte.
unsafe fn fvg_fill_color_scissor_frames(lost0: u32, refused0: u32) -> Result<bool, String> {
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, FVG_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, FVG_FCS_FS)?);
    glBindAttribLocation(p, 0, c"vertex".as_ptr());
    glBindAttribLocation(p, 1, c"tcoord".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    let view = glGetUniformLocation(p, c"viewSize".as_ptr());
    let frag = glGetUniformLocation(p, c"frag".as_ptr());
    if view < 0 || frag < 0 {
        return Err(format!("viewSize ({view}) or frag ({frag}) is not an active uniform"));
    }
    let mut block = [0f32; 56];
    /* scissorMat: the identity translated by (-32, -32) */
    block[0] = 1.0;
    block[4 + 1] = 1.0;
    block[2 * 4..2 * 4 + 2].copy_from_slice(&[-32.0, -32.0]);
    block[6 * 4..6 * 4 + 4].copy_from_slice(&[0.0, 1.0, 0.0, 1.0]);
    /* scissorExt (20, 12), scissorScale (1, 1) */
    block[8 * 4..8 * 4 + 4].copy_from_slice(&[20.0, 12.0, 1.0, 1.0]);
    block[10 * 4] = 1.0;
    glUniform2f(view, W as f32, H as f32);
    println!(
        "panthor-gl: fvg-fill-color-scissor: viewSize loc={view} frag loc={frag}; innerCol (0, 1, 0, 1), strokeMult 1, scissor centred, ext (20, 12), scale (1, 1)"
    );
    /* fvgfcs.c's vertices: (x, y, u, v) with u = x / 64, v = y / 64 */
    let v: [f32; 12] = [-64.0, -64.0, -1.0, -1.0, 192.0, -64.0, 3.0, -1.0, 64.0, 192.0, 1.0, 3.0];
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, 0, 16, v.as_ptr().cast());
    glEnableVertexAttribArray(1);
    glVertexAttribPointer(1, 2, GL_FLOAT, 0, 16, v.as_ptr().cast::<u8>().add(8).cast());
    let mut pictures: Vec<Vec<u8>> = Vec::new();
    let mut all = true;
    for (radius, tag, file) in [
        (0.0f32, "rect (radius 0, the else side)", "fvgfcs-rect-ours.rgba"),
        (6.0, "round (radius 6, the then side)", "fvgfcs-round-ours.rgba"),
    ] {
        block[12 * 4 + 3] = radius;
        glUniform4fv(frag, 14, block.as_ptr());
        gl_error("glUniform")?;
        glClearColor(0.1, 0.1, 0.25, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        gl_error("glDrawArrays")?;
        let mut px = vec![0u8; W * H * 4];
        glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        gl_error("glReadPixels")?;
        glFinish();
        /* gl_test's read_topdown: the rows top first, FNV-1a over the bytes */
        let mut top = vec![0u8; W * H * 4];
        for y in 0..H {
            top[y * W * 4..(y + 1) * W * 4].copy_from_slice(&px[(H - 1 - y) * W * 4..(H - y) * W * 4]);
        }
        let mut h = 2166136261u32;
        for b in &top {
            h = (h ^ u32::from(*b)).wrapping_mul(16777619);
        }
        let (mut shape, mut greens) = (0usize, std::collections::BTreeSet::new());
        for p in top.chunks(4) {
            if p[0] == 0 && p[2] == 0 && p[1] == p[3] {
                shape += 1;
            }
            greens.insert(p[1]);
        }
        let at = |x: usize, y: usize| &top[(y * W + x) * 4..][..4];
        let (centre, corner, above) = (at(W / 2, H / 2), at(2, 2), at(W / 2, 2));
        println!("panthor-gl: fvg-fill-color-scissor {tag}: readback fnv1a {h:08x}, centre (32, 32 top-down) {centre:?}, corner (2, 2) {corner:?}, above (32, 2) {above:?}");
        println!(
            "panthor-gl: fvg-fill-color-scissor {tag}: {shape} of {} pixels have R = B = 0 and G = A, {} distinct G values",
            W * H,
            greens.len()
        );
        for y in (0..H).step_by(4) {
            let row: String = (0..W)
                .step_by(2)
                .map(|x| top[(y * W + x) * 4 + 1])
                .map(|g| b" .:-=+*#%@"[g as usize / 26] as char)
                .collect();
            println!("panthor-gl: fvg-fill-color-scissor {tag}: G row {y:2} {row}");
        }
        match std::fs::write(file, &top) {
            Ok(()) => println!("panthor-gl: fvg-fill-color-scissor: wrote {file} ({} bytes, top-down RGBA)", top.len()),
            Err(e) => println!("panthor-gl: fvg-fill-color-scissor: {file} not written: {e}"),
        }
        let g = centre[1] as u32;
        all &= shape == W * H
            && greens.len() > 16
            && corner == [0, 0, 0, 0]
            && above == [0, 0, 0, 0]
            && (100..=140).contains(&g);
        pictures.push(top);
    }
    /* the rounded corners are the only difference */
    let (mut differ, mut far) = (0usize, 0usize);
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) * 4;
            if pictures[0][i..i + 4] != pictures[1][i..i + 4] {
                differ += 1;
                let (dx, dy) = ((x as i32 - 32).abs(), (y as i32 - 32).abs());
                if !(dx > 20 - 8 && dy > 12 - 8) {
                    far += 1;
                }
            }
        }
    }
    println!("panthor-gl: fvg-fill-color-scissor: the two pictures differ in {differ} pixel(s), {far} of them away from a corner");
    all &= differ > 0 && far == 0;
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let (subs, open, live, live_hw, bytes, bytes_hw) = osal::counts().ok_or("no OSAL state")?;
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; osal submits {subs}, unwaited fences {open}, BOs live {live} (high-water {live_hw}), bytes live {bytes} (high-water {bytes_hw}); group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    Ok(all && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// The OSAL's init and the backend's, then a context whose default surface
/// is top-down, `w` x `h`: gl_test's order (its main), the two calls
/// eglInitialize makes (src/gl/egl.rs).
unsafe fn top_down_context(w: u32, h: u32) -> Result<(), String> {
    let init = yakogl::osal::ops().and_then(|o| o.init).ok_or("the OSAL has no init")?;
    let rc = init(core::ptr::null_mut());
    if rc != 0 {
        return Err(format!("OSAL init failed ({rc})"));
    }
    let rc = yakogl::backend::init();
    if rc != 0 {
        return Err(format!("backend init failed ({rc})"));
    }
    let ctx = yakogl::gl::context::gl_context_create(w, h);
    if ctx.is_null() || yakogl::gl::context::gl_default_surface().is_null() {
        return Err("gl_context_create: no context or no default surface".into());
    }
    Ok(())
}

/// A `--window-*` mode (module docs): the window's native format and the
/// config sizes it needs, the triangle's colour, and Mesa's read-back of
/// fmt.c in that config and colour (its FNV-1a).
struct WindowSpec {
    native: u32,
    sizes: [EGLint; 4],
    mid: bool,
    fnv1a: u32,
    /// preload_fmt.c's two frames instead of fmt.c's triangle.
    preload: bool,
}

impl Mode {
    fn window(self) -> Option<WindowSpec> {
        use yakogl::gl::egl::{GLES_NATIVE_FORMAT_RGB565, GLES_NATIVE_FORMAT_RGBA8888, GLES_NATIVE_FORMAT_XRGB8888};
        let (native, sizes, mid, fnv1a) = match self {
            Mode::WindowTri => (GLES_NATIVE_FORMAT_RGBA8888, [8, 8, 8, 8], false, 0x890abd45),
            Mode::WindowMid => (GLES_NATIVE_FORMAT_RGBA8888, [8, 8, 8, 8], true, 0xb343e245),
            /* our EGL offers no 8/8/8/0 config: an XRGB8888 window takes
             * the RGBA8888 one, as gl_test's XRGB windows (cases 172, 173)
             * do; EGL matches only RGB565 to its config (src/gl/egl.rs) */
            Mode::WindowXrgbRed => (GLES_NATIVE_FORMAT_XRGB8888, [8, 8, 8, 8], false, 0x890abd45),
            Mode::WindowXrgbMid => (GLES_NATIVE_FORMAT_XRGB8888, [8, 8, 8, 8], true, 0xb343e245),
            Mode::Window565Red => (GLES_NATIVE_FORMAT_RGB565, [5, 6, 5, 0], false, 0x890abd45),
            Mode::Window565Mid => (GLES_NATIVE_FORMAT_RGB565, [5, 6, 5, 0], true, 0x04c57ea5),
            /* preload_fmt.c (0dce654a8:tools/mali/board-ref/2026-10-19-preload-formats):
             * one hash for every config */
            Mode::WindowPreload(f) => {
                let sizes = if f == GLES_NATIVE_FORMAT_RGB565 { [5, 6, 5, 0] } else { [8, 8, 8, 8] };
                return Some(WindowSpec { native: f, sizes, mid: false, fnv1a: 0x9a3071c5, preload: true });
            }
            _ => return None,
        };
        Some(WindowSpec { native, sizes, mid, fnv1a, preload: false })
    }
}

/// A `--window-*` mode: an EGL window over the program's own `gpu_alloc`,
/// flip.c's triangle, the read-back against Mesa's, and the program's own
/// allocation, read in the window's layout, against the read-back
/// mirrored.
unsafe fn window_frame(w: &WindowSpec, lost0: u32, refused0: u32) -> Result<bool, String> {
    use yakogl::gl::egl::{
        eglCreateWindowSurface, eglGetConfigAttrib, GlesNativeWindow, GLES_NATIVE_FORMAT_RGB565,
        GLES_NATIVE_FORMAT_XRGB8888,
    };
    let dpy = eglGetDisplay(core::ptr::null_mut());
    if eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()) == 0 {
        return Err("eglInitialize failed".into());
    }
    let [r, g, b, a] = w.sizes;
    let cfg_attrs = [
        EGL_SURFACE_TYPE,
        EGL_WINDOW_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE,
        r,
        EGL_GREEN_SIZE,
        g,
        EGL_BLUE_SIZE,
        b,
        EGL_ALPHA_SIZE,
        a,
        EGL_NONE,
    ];
    /* eglChooseConfig sorts deeper colour first (EGL 1.5 3.4.1.2): the
     * config whose sizes are exactly these */
    let mut cfgs: [EGLConfig; 8] = [core::ptr::null_mut(); 8];
    let mut n = 0;
    eglChooseConfig(dpy, cfg_attrs.as_ptr(), cfgs.as_mut_ptr(), cfgs.len() as EGLint, &mut n);
    let exact = |c: EGLConfig| {
        [EGL_RED_SIZE, EGL_GREEN_SIZE, EGL_BLUE_SIZE, EGL_ALPHA_SIZE].iter().zip(w.sizes).all(|(&k, want)| {
            let mut v = -1;
            eglGetConfigAttrib(dpy, c, k, &mut v) != 0 && v == want
        })
    };
    let cfg =
        *cfgs[..n.max(0) as usize].iter().find(|&&c| exact(c)).ok_or("eglChooseConfig: no exact window config")?;
    let bpp = if w.native == GLES_NATIVE_FORMAT_RGB565 { 2 } else { 4 };
    let mut mem = yakogl::osal::OsalGpuMem::ZERO;
    let bytes = FLIP_W * FLIP_H * bpp;
    let heap = yakogl::backend::heap_base(yakogl::backend::GLES_HEAP_DATA);
    if yakogl::osal::gpu_alloc(heap, bytes, yakogl::osal::OSAL_MEM_DATA, &mut mem) != 0 || mem.cpu.is_null() {
        return Err("gpu_alloc of the window's memory".into());
    }
    for p in core::slice::from_raw_parts_mut(mem.cpu as *mut u8, bytes).chunks_mut(bpp) {
        p.copy_from_slice(&l1::FILL[..bpp]);
    }
    let mut win = GlesNativeWindow {
        phys: mem.phys,
        width: FLIP_W as u32,
        height: FLIP_H as u32,
        stride_bytes: (FLIP_W * bpp) as u32,
        format: w.native,
    };
    let single = [EGL_RENDER_BUFFER, EGL_SINGLE_BUFFER, EGL_NONE];
    let surf = eglCreateWindowSurface(dpy, cfg, &mut win, single.as_ptr());
    let (windows, _) = osal::windows().ok_or("no OSAL state")?;
    println!(
        "panthor-gl: window over gpu_alloc gpu {:#x} phys {:#x}, native format {}: map_phys windows live {windows}",
        mem.gpu, mem.phys, w.native
    );
    let ctx_attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    let ctx = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_attrs.as_ptr());
    if surf.is_null() || ctx.is_null() || eglMakeCurrent(dpy, surf, surf, ctx) == 0 {
        return Err("window surface, context or eglMakeCurrent failed".into());
    }
    let fs = if w.mid {
        let m = TRI_FS.replace("vec4(1.0, 0.0, 0.0, 1.0)", "vec4(0.5, 0.25, 0.75, 1.0)");
        if m == TRI_FS {
            return Err("tri.frag has no red constant to change".into());
        }
        m
    } else {
        TRI_FS.to_string()
    };
    let p = glCreateProgram();
    let (vs, fs) = if w.preload { (PRELOAD_VS, COLOR_FS.to_string()) } else { (TRI_VS, fs) };
    glAttachShader(p, shader(GL_VERTEX_SHADER, vs)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs)?);
    glBindAttribLocation(p, 0, c"pos".as_ptr());
    glLinkProgram(p);
    glUseProgram(p);
    glViewport(0, 0, FLIP_W as GLsizei, FLIP_H as GLsizei);
    glClearColor(0.0, 0.0, 1.0, 1.0);
    glClear(GL_COLOR_BUFFER_BIT);
    if w.preload {
        /* preload_fmt.c: red over columns 0-31, glFinish, green over 16-47
         * in a frame without a clear */
        let col = glGetUniformLocation(p, c"u_color".as_ptr());
        let quad = |x0: f32, x1: f32, rgb: [f32; 3]| {
            let v = [x0, -1.0, x1, -1.0, x0, 1.0, x1, -1.0, x1, 1.0, x0, 1.0];
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, v.as_ptr().cast());
            glEnableVertexAttribArray(0);
            glUniform4f(col, rgb[0], rgb[1], rgb[2], 1.0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
        };
        quad(-1.0, 0.0, [1.0, 0.0, 0.0]);
        glFinish();
        quad(-0.5, 0.5, [0.0, 1.0, 0.0]);
    } else {
        let (cx, cy) = (|x: f32| x / (FLIP_W as f32 / 2.0) - 1.0, |y: f32| y / (FLIP_H as f32 / 2.0) - 1.0);
        let tri = [cx(8.0), cy(8.0), cx(40.0), cy(8.0), cx(8.0), cy(40.0)];
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, tri.as_ptr().cast());
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    gl_error("the draw")?;
    let mut px = vec![0u8; FLIP_W * FLIP_H * 4];
    glReadPixels(0, 0, FLIP_W as GLsizei, FLIP_H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    /* the GPU is done with the pages (glFinish): the program's own view,
     * memory row H - 1 - y against GL row y, in the window's layout */
    let own = core::slice::from_raw_parts(mem.cpu as *const u8, bytes);
    let pixel_matches = |q: &[u8], rgba: &[u8]| match w.native {
        GLES_NATIVE_FORMAT_XRGB8888 => q[..3] == [rgba[2], rgba[1], rgba[0]],
        GLES_NATIVE_FORMAT_RGB565 => {
            let v = u16::from_le_bytes([q[0], q[1]]);
            [(v >> 11) as u8, (v >> 5 & 63) as u8, (v & 31) as u8] == [rgba[0] >> 3, rgba[1] >> 2, rgba[2] >> 3]
        }
        _ => q == rgba,
    };
    let mirrored = (0..FLIP_H)
        .filter(|&y| {
            let raw = &own[(FLIP_H - 1 - y) * FLIP_W * bpp..][..FLIP_W * bpp];
            raw.chunks(bpp).zip(px[y * FLIP_W * 4..][..FLIP_W * 4].chunks(4)).all(|(q, rgba)| pixel_matches(q, rgba))
        })
        .count();
    let mut colours: Vec<([u8; 4], usize)> = Vec::new();
    for q in px.chunks(4) {
        let q: [u8; 4] = q.try_into().unwrap();
        match colours.iter_mut().find(|(c, _)| *c == q) {
            Some((_, n)) => *n += 1,
            None => colours.push((q, 1)),
        }
    }
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    for (c, n) in &colours {
        println!("panthor-gl: colour {:08x}: {n} pixels", u32::from_le_bytes(*c));
    }
    println!(
        "panthor-gl: pixels {}: colours {} fnv1a {fnv1a:08x} (Mesa's pbuffer: fnv1a {:08x})",
        FLIP_W * FLIP_H,
        colours.len(),
        w.fnv1a
    );
    println!("panthor-gl: the program's own allocation: {mirrored} of {FLIP_H} rows are the read-back's rows mirrored");
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(windows == 1 && fnv1a == w.fnv1a && mirrored == FLIP_H && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// gl_test case 56's calls on a 64x64 top-down surface, against the board's
/// Mesa on the same calls (module docs): per line count, the rows lit at
/// x = 32 and the first line's pixel.
unsafe fn manylines_frame(colour1: bool, lost0: u32, refused0: u32) -> Result<bool, String> {
    top_down_context(W as u32, H as u32)?;
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, CUV_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, CUV_FS)?);
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    let loc = |n: &core::ffi::CStr| glGetAttribLocation(p, n.as_ptr());
    let locs = (loc(c"a_pos"), loc(c"a_color"), loc(c"a_uv"));
    println!("panthor-gl: locations: a_pos {} a_color {} a_uv {}", locs.0, locs.1, locs.2);
    if locs != (0, 1, 2) {
        return Err("the linker's attribute order is not the case's (0, 1, 2)".into());
    }
    /* gl_test's cuv_vertex: pos 4, colour 4, uv 2 floats */
    let mut v = [[0f32; 10]; 40];
    let stride = core::mem::size_of::<[f32; 10]>() as GLsizei;
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, stride, v[0].as_ptr().cast());
    glVertexAttribPointer(2, 2, GL_FLOAT, 0, stride, v[0][8..].as_ptr().cast());
    glEnableVertexAttribArray(0);
    glEnableVertexAttribArray(2);
    if colour1 {
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, stride, v[0][4..].as_ptr().cast());
        glEnableVertexAttribArray(1);
    }
    let ndc_x = |x: u32| (x as f32 + 0.5) / W as f32 * 2.0 - 1.0;
    let ndc_y = |y: u32| 1.0 - (y as f32 + 0.5) / H as f32 * 2.0;
    let mut px = vec![0u8; W * H * 4];
    let mut all = true;
    for n in [2u32, 4, 8, 16, 20] {
        for i in 0..n {
            let row = 2 + 3 * i;
            for (k, x) in [4, 59].into_iter().enumerate() {
                v[(2 * i) as usize + k] = [ndc_x(x), ndc_y(row), 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
            }
        }
        glClear(GL_COLOR_BUFFER_BIT);
        glDrawArrays(GL_LINES, 0, (2 * n) as GLsizei);
        glFinish();
        gl_error("many lines")?;
        glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        gl_error("glReadPixels")?;
        /* the case's read_topdown: GL row H - 1 - y is top-down row y */
        let word = |y: u32| {
            let i = ((H - 1 - y as usize) * W + 32) * 4;
            u32::from_le_bytes(px[i..i + 4].try_into().unwrap())
        };
        let lit = (0..n).filter(|i| word(2 + 3 * i) & 0x00ff_ffff != 0).count() as u32;
        println!("panthor-gl: {n:2} lines in one draw -> {lit:2} rows lit; row 2 pixel {:08x}", word(2));
        let want = if colour1 { (n, 0xffff_ffff) } else { (0, 0xff00_0000) };
        all &= (lit, word(2)) == want;
    }
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    Ok(all && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// A flip.c mode into a top-down default surface, the read-back compared
/// with Mesa's pbuffer read-back (`want`: its red count and FNV-1a hash),
/// and the same failure conditions as the triangle modes.
unsafe fn flip_frame(mode: Mode, want: (usize, u32), lost0: u32, refused0: u32) -> Result<bool, String> {
    top_down_context(FLIP_W as u32, FLIP_H as u32)?;
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, TRI_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, TRI_FS)?);
    glBindAttribLocation(p, 0, c"pos".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    glViewport(0, 0, FLIP_W as GLsizei, FLIP_H as GLsizei);
    glClearColor(0.0, 0.0, 1.0, 1.0);
    glClear(GL_COLOR_BUFFER_BIT);
    /* flip.c's window (x, y) to clip */
    let (cx, cy) = (|x: f32| x / (FLIP_W as f32 / 2.0) - 1.0, |y: f32| y / (FLIP_H as f32 / 2.0) - 1.0);
    let tri = [cx(8.0), cy(8.0), cx(40.0), cy(8.0), cx(8.0), cy(40.0)];
    let quad: [f32; 12] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0];
    let v: &[f32] = match mode {
        Mode::FlipFrontCw => {
            glFrontFace(GL_CW);
            &tri
        }
        Mode::FlipTri => &tri,
        Mode::FlipViewport => {
            glViewport(8, 4, 32, 16);
            &quad
        }
        Mode::FlipScissorDraw | Mode::FlipScissorClear => {
            glEnable(GL_SCISSOR_TEST);
            glScissor(8, 4, 16, 20);
            &quad
        }
        _ => return Err("not a flip mode".into()),
    };
    if mode == Mode::FlipScissorClear {
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    } else {
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, v.as_ptr().cast());
        glDrawArrays(GL_TRIANGLES, 0, (v.len() / 2) as GLsizei);
    }
    gl_error("the draw")?;
    let mut px = vec![0u8; FLIP_W * FLIP_H * 4];
    glReadPixels(0, 0, FLIP_W as GLsizei, FLIP_H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    /* the read-back is GL's row order whichever way the surface is stored;
     * the surface's own rows tell the two apart: top-down, memory row
     * H - 1 - y holds GL row y (64 pixels a row: tex_pitch(64) = 64) */
    let surf = yakogl::gl::context::gl_default_surface();
    let cpu = (*surf).cpu as *const u8;
    if cpu.is_null() || (*surf).size < px.len() {
        return Err("the default surface has no CPU view of 64x48 pixels".into());
    }
    let raw = core::slice::from_raw_parts(cpu, px.len());
    let row = FLIP_W * 4;
    let mirrored = (0..FLIP_H).filter(|&y| raw[(FLIP_H - 1 - y) * row..][..row] == px[y * row..][..row]).count();
    let same = (0..FLIP_H).filter(|&y| raw[y * row..][..row] == px[y * row..][..row]).count();
    println!("panthor-gl: surface rows: {mirrored} of {FLIP_H} are the read-back's rows mirrored (top-down), {same} the same rows");
    let at = |i: usize| -> [u8; 4] { px[i * 4..][..4].try_into().unwrap() };
    let reds: Vec<usize> = (0..FLIP_W * FLIP_H).filter(|&i| at(i) == RED).collect();
    let other = (0..FLIP_W * FLIP_H).filter(|&i| ![RED, BLUE].contains(&at(i))).count();
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    let span = |f: fn(usize) -> usize| {
        let v: Vec<usize> = reds.iter().map(|&i| f(i)).collect();
        (v.iter().min().copied().unwrap_or(0), v.iter().max().copied().unwrap_or(0))
    };
    let ((x0, x1), (y0, y1)) = (span(|i| i % FLIP_W), span(|i| i / FLIP_W));
    println!("panthor-gl: red box x [{x0}, {x1}] y [{y0}, {y1}]");
    println!(
        "panthor-gl: pixels {}: red {} other {other} fnv1a {fnv1a:08x} (Mesa's pbuffer: red {} other 0 fnv1a {:08x})",
        FLIP_W * FLIP_H,
        reds.len(),
        want.0,
        want.1
    );
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(reds.len() == want.0
        && other == 0
        && fnv1a == want.1
        && mirrored == FLIP_H
        && lost == 0
        && refused == 0
        && gs == (0, 0)
        && usable)
}

/// `--msaa-4x` / `--msaa-1x` (module docs): case 16's draw into a pbuffer
/// of `samples`, the read-back against Mesa's.
unsafe fn msaa_frame(samples: u32, coverage: Option<bool>, lost0: u32, refused0: u32) -> Result<bool, String> {
    use yakogl::gl::egl::eglGetConfigAttrib;
    let dpy = eglGetDisplay(core::ptr::null_mut());
    if eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()) == 0 {
        return Err("eglInitialize failed".into());
    }
    let want = if samples > 1 { samples as EGLint } else { 0 };
    let cfg_attrs = [EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_SAMPLES, want, EGL_NONE];
    let mut cfgs: [EGLConfig; 8] = [core::ptr::null_mut(); 8];
    let mut n = 0;
    eglChooseConfig(dpy, cfg_attrs.as_ptr(), cfgs.as_mut_ptr(), cfgs.len() as EGLint, &mut n);
    let cfg = *cfgs[..n.max(0) as usize]
        .iter()
        .find(|&&c| {
            let mut v = -1;
            eglGetConfigAttrib(dpy, c, EGL_SAMPLES, &mut v) != 0 && v == want
        })
        .ok_or_else(|| format!("eglChooseConfig: no config of EGL_SAMPLES {want}"))?;
    let surf_attrs = [EGL_WIDTH, W as EGLint, EGL_HEIGHT, H as EGLint, EGL_NONE];
    let surf = eglCreatePbufferSurface(dpy, cfg, surf_attrs.as_ptr());
    let ctx_attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    let ctx = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_attrs.as_ptr());
    if surf.is_null() || ctx.is_null() || eglMakeCurrent(dpy, surf, surf, ctx) == 0 {
        return Err("pbuffer, context or eglMakeCurrent failed".into());
    }
    let fs = TRI_FS.replace("vec4(1.0, 0.0, 0.0, 1.0)", "vec4(1.0, 0.0, 1.0, 1.0)");
    if fs == TRI_FS {
        return Err("tri.frag has no red constant to change".into());
    }
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, TRI_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs)?);
    glBindAttribLocation(p, 0, c"pos".as_ptr());
    glLinkProgram(p);
    glUseProgram(p);
    glViewport(0, 0, W as GLsizei, H as GLsizei);
    glClearColor(0.0, 0.0, 0.0, 1.0);
    glClear(GL_COLOR_BUFFER_BIT);
    const TRI: [f32; 6] = [-1.0, -1.0, 1.0, -1.0, 1.0, 1.0];
    if let Some(invert) = coverage {
        glEnable(GL_SAMPLE_COVERAGE);
        glSampleCoverage(0.5, GLboolean::from(invert));
    }
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, TRI.as_ptr().cast());
    glDrawArrays(GL_TRIANGLES, 0, 3);
    gl_error("the draw")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let at = |x: usize, y: usize| u32::from_le_bytes(px[(y * W + x) * 4..][..4].try_into().unwrap());
    let half = px.chunks(4).filter(|q| *q == [0x80, 0, 0x80, 0xff]).count();
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    let mesa = match (samples, coverage) {
        (1, _) => 0x570010c5,
        (_, None) => 0x3c0190c5,
        (_, Some(false)) => 0xd8c31dc5,
        (_, Some(true)) => 0xa8c31dc5,
    };
    println!(
        "panthor-gl: inside {:08x} outside {:08x} edge {:08x}; half-covered pixels {half}; pixels 4096: fnv1a {fnv1a:08x} (Mesa: fnv1a {mesa:08x})",
        at(40, 20),
        at(20, 40),
        at(32, 32)
    );
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(fnv1a == mesa && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// vcomp.c's draw into the bound FBO and its read-back against Mesa's, for
/// a mode of [`VCOMP_MODES`], [`FSCONST_MODES`], [`FNEG_MODES`] or
/// [`FABS_MODES`], with `color` as v_color.
unsafe fn vcomp_frame(
    m: &(&str, &str, bool, [u32; 4], u32),
    color: [f32; 4],
    lost0: u32,
    refused0: u32,
) -> Result<bool, String> {
    let (_, fs, two, want_px, want) = *m;
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, if two { CMP_VS2X4 } else { CMP_VS4 })?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs)?);
    glBindAttribLocation(p, 0, c"a_pos".as_ptr());
    glBindAttribLocation(p, 1, c"a_color".as_ptr());
    glBindAttribLocation(p, 2, c"a_uv".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    /* vcomp.c's vertices: the full target as two triangles */
    #[rustfmt::skip]
    const POS: [f32; 24] = [
        -1.0, -1.0, 0.0, 1.0,  1.0, -1.0, 0.0, 1.0,  -1.0, 1.0, 0.0, 1.0,
        1.0, -1.0, 0.0, 1.0,  1.0, 1.0, 0.0, 1.0,  -1.0, 1.0, 0.0, 1.0,
    ];
    /* the mode's values: one per component, the same at every vertex,
     * each one fp16 holds exactly (see the record's README) */
    let mut col = [0.0f32; 24];
    for (i, c) in col.iter_mut().enumerate() {
        *c = color[i % 4];
    }
    const UV: [f32; 24] = {
        let mut v = [0.0; 24];
        let mut i = 0;
        while i < 24 {
            v[i] = [0.625, 0.875, 0.0625, 0.9375][i % 4];
            i += 1;
        }
        v
    };
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr().cast());
    glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, col.as_ptr().cast());
    glEnableVertexAttribArray(0);
    glEnableVertexAttribArray(1);
    if two {
        glVertexAttribPointer(2, 4, GL_FLOAT, 0, 0, UV.as_ptr().cast());
        glEnableVertexAttribArray(2);
    }
    glViewport(0, 0, W as GLsizei, H as GLsizei);
    glClearColor(0.0, 0.0, 0.0, 1.0);
    glClear(GL_COLOR_BUFFER_BIT);
    glDrawArrays(GL_TRIANGLES, 0, 6);
    gl_error("glDrawArrays")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let at = |x: usize, y: usize| u32::from_le_bytes(px[(y * W + x) * 4..][..4].try_into().unwrap());
    let mut pixels = true;
    for ((x, y), w) in [(8, 8), (56, 8), (8, 56), (56, 56)].into_iter().zip(want_px) {
        println!("panthor-gl: GL ({x},{y}) {:08x} (Mesa: {w:08x})", at(x, y));
        pixels &= at(x, y) == w;
    }
    /* each row's hash, as vcomp.c prints them, to find a difference */
    for (y, row) in px.chunks(W * 4).enumerate() {
        let r = row.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
        println!("panthor-gl: row {y} fnv1a {r:08x}");
    }
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    println!("panthor-gl: pixels 4096: fnv1a {fnv1a:08x} (Mesa: fnv1a {want:08x})");
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(pixels && fnv1a == want && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// pcoord.c's pbuffer: an EGL pbuffer of 64x64 RGBA8888, then the draw.
unsafe fn pcoord_pbuffer(lost0: u32, refused0: u32) -> Result<bool, String> {
    let dpy = eglGetDisplay(core::ptr::null_mut());
    if eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()) == 0 {
        return Err("eglInitialize failed".into());
    }
    /* pcoord.c's request: RGBA8888 (with no colour size asked, EGL 1.5
     * 3.4.1 sorts the smaller RGB565 first) */
    let cfg_attrs = [
        EGL_SURFACE_TYPE,
        EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE,
        8,
        EGL_GREEN_SIZE,
        8,
        EGL_BLUE_SIZE,
        8,
        EGL_ALPHA_SIZE,
        8,
        EGL_SAMPLES,
        0,
        EGL_NONE,
    ];
    let mut cfg: EGLConfig = core::ptr::null_mut();
    let mut n = 0;
    if eglChooseConfig(dpy, cfg_attrs.as_ptr(), &mut cfg, 1, &mut n) == 0 || n < 1 {
        return Err("eglChooseConfig: no pbuffer config".into());
    }
    let size = |a: EGLint| {
        let mut v = -1;
        yakogl::gl::egl::eglGetConfigAttrib(dpy, cfg, a, &mut v);
        v
    };
    let rgba = [EGL_RED_SIZE, EGL_GREEN_SIZE, EGL_BLUE_SIZE, EGL_ALPHA_SIZE].map(size);
    println!("panthor-gl: pbuffer config RGBA sizes {rgba:?}");
    if rgba != [8; 4] {
        return Err(format!("eglChooseConfig gave RGBA sizes {rgba:?}, not 8888"));
    }
    let surf_attrs = [EGL_WIDTH, W as EGLint, EGL_HEIGHT, H as EGLint, EGL_NONE];
    let surf = eglCreatePbufferSurface(dpy, cfg, surf_attrs.as_ptr());
    let ctx_attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    let ctx = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_attrs.as_ptr());
    if surf.is_null() || ctx.is_null() || eglMakeCurrent(dpy, surf, surf, ctx) == 0 {
        return Err("pbuffer, context or eglMakeCurrent failed".into());
    }
    pcoord_frame(lost0, refused0)
}

/// pcoord.c's draw into the bound target and its read-back against Mesa's.
unsafe fn pcoord_frame(lost0: u32, refused0: u32) -> Result<bool, String> {
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, POINTS_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, POINTCOORD_FS)?);
    glBindAttribLocation(p, 0, c"a_pos".as_ptr());
    glBindAttribLocation(p, 1, c"a_color".as_ptr());
    glBindAttribLocation(p, 2, c"a_size".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    /* gl_test's struct pv: position, colour, size */
    let v: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.25, 0.75, 0.5, 1.0, 16.0];
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, 36, v.as_ptr().cast());
    glVertexAttribPointer(1, 4, GL_FLOAT, 0, 36, v[4..].as_ptr().cast());
    glVertexAttribPointer(2, 1, GL_FLOAT, 0, 36, v[8..].as_ptr().cast());
    for a in 0..3 {
        glEnableVertexAttribArray(a);
    }
    glViewport(0, 0, W as GLsizei, H as GLsizei);
    glClearColor(0.0, 0.0, 0.0, 1.0);
    glClear(GL_COLOR_BUFFER_BIT);
    glDrawArrays(GL_POINTS, 0, 1);
    gl_error("glDrawArrays")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let at = |x: usize, y: usize| u32::from_le_bytes(px[(y * W + x) * 4..][..4].try_into().unwrap());
    /* Mesa's read-back (0dce654a8:tools/mali/board-ref/2026-10-19-pointcoord/mesa) */
    const WANT: [((usize, usize), u32); 5] = [
        ((24, 39), 0xff000808),
        ((39, 39), 0xff0008f7),
        ((24, 24), 0xff00f708),
        ((39, 24), 0xff00f7f7),
        ((23, 32), 0xff000000),
    ];
    let mut corners = true;
    for ((x, y), want) in WANT {
        println!("panthor-gl: GL ({x},{y}) {:08x} (Mesa: {want:08x})", at(x, y));
        corners &= at(x, y) == want;
    }
    let lit = (0..W * H).filter(|&i| at(i % W, i / W) != 0xff000000).count();
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    println!("panthor-gl: pixels 4096: not black {lit} fnv1a {fnv1a:08x} (Mesa: not black 256 fnv1a fb970d45)");
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(corners && lit == 256 && fnv1a == 0xfb970d45 && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// A poff.c mode (module docs) on the bound FBO with its D24S8: the two
/// quads, the read-back compared with Mesa's.
unsafe fn poff_frame(i: usize, lost0: u32, refused0: u32) -> Result<bool, String> {
    let (name, tilt, factor, units, want) = POFFS[i];
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, ZS_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, COLOR_FS)?);
    glBindAttribLocation(p, 0, c"pos".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    let col = glGetUniformLocation(p, c"u_color".as_ptr());
    if col < 0 {
        return Err("u_color is not an active uniform".into());
    }
    glClearColor(0.0, 0.0, 0.0, 1.0);
    glClearDepthf(1.0);
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    glEnable(GL_DEPTH_TEST);
    glDepthFunc(GL_LESS);
    let (zl, zr) = if tilt { (-0.5, 0.5) } else { (0.0, 0.0) };
    let quad: [f32; 18] = [-1.0, -1.0, zl, 1.0, -1.0, zr, -1.0, 1.0, zl, -1.0, 1.0, zl, 1.0, -1.0, zr, 1.0, 1.0, zr];
    glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, quad.as_ptr().cast());
    glEnableVertexAttribArray(0);
    glUniform4f(col, 0.0, 0.0, 1.0, 1.0);
    glDrawArrays(GL_TRIANGLES, 0, 6);
    glEnable(GL_POLYGON_OFFSET_FILL);
    glPolygonOffset(factor, units);
    glUniform4f(col, 0.0, 1.0, 0.0, 1.0);
    glDrawArrays(GL_TRIANGLES, 0, 6);
    gl_error("the draws")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let (green, blue) = (px.chunks(4).filter(|q| *q == GREEN).count(), px.chunks(4).filter(|q| *q == BLUE).count());
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    println!("panthor-gl: {name}: pixels 4096: green {green} blue {blue} fnv1a {fnv1a:08x} (Mesa: fnv1a {want:08x})");
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(fnv1a == want && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// A blend.c mode (module docs): the blended quad over the clear, the
/// read-back compared with Mesa's, and the same failure conditions as the
/// triangle modes.
unsafe fn blend_frame(i: usize, lost0: u32, refused0: u32) -> Result<bool, String> {
    let (name, want) = BLENDS[i];
    let fs = TRI_FS.replace("vec4(1.0, 0.0, 0.0, 1.0)", "vec4(1.0, 0.5, 0.25, 0.75)");
    if fs == TRI_FS {
        return Err("tri.frag has no red constant to change".into());
    }
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, TRI_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs)?);
    glBindAttribLocation(p, 0, c"pos".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    glClearColor(0.25, 0.5, 0.75, 0.5);
    glClear(GL_COLOR_BUFFER_BIT);
    glEnable(GL_BLEND);
    match name {
        "over" => glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA),
        "add" => glBlendFunc(GL_ONE, GL_ONE),
        "premul" => glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA),
        "replace" => glBlendFunc(GL_ONE, GL_ZERO),
        "sub" => {
            glBlendEquation(GL_FUNC_SUBTRACT);
            glBlendFunc(GL_ONE, GL_ONE);
        }
        "revsub" => {
            glBlendEquation(GL_FUNC_REVERSE_SUBTRACT);
            glBlendFunc(GL_ONE, GL_ONE);
        }
        "mul" => glBlendFunc(GL_DST_COLOR, GL_ZERO),
        "twosd" => glBlendFunc(GL_DST_COLOR, GL_SRC_COLOR),
        "const" => {
            glBlendColor(0.5, 0.5, 0.5, 0.5);
            glBlendFunc(GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_COLOR);
        }
        _ => glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ZERO),
    }
    gl_error("the blend state")?;
    const QUAD: [f32; 12] = [-1.0, -1.0, 0.0, -1.0, -1.0, 1.0, -1.0, 1.0, 0.0, -1.0, 0.0, 1.0];
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, QUAD.as_ptr().cast());
    glDrawArrays(GL_TRIANGLES, 0, 6);
    gl_error("glDrawArrays")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let at = |x: usize, y: usize| u32::from_le_bytes(px[(y * W + x) * 4..][..4].try_into().unwrap());
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    println!(
        "panthor-gl: blended {:08x} kept {:08x}; pixels 4096: fnv1a {fnv1a:08x} (Mesa: fnv1a {want:08x})",
        at(8, 8),
        at(56, 8)
    );
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(fnv1a == want && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// An idx.c mode (module docs): one red glDrawElements over the blue
/// clear, the read-back compared with Mesa's, and the same failure
/// conditions as the triangle modes.
unsafe fn idx_frame(mode: Mode, lost0: u32, refused0: u32) -> Result<bool, String> {
    const V: [f32; 16] = [-1.0, -1.0, 0.0, -1.0, -1.0, 1.0, 0.0, 1.0, 0.0, -1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0];
    const RIGHT16: [u16; 6] = [4, 5, 6, 6, 5, 7];
    const OFFSET16: [u16; 9] = [0, 0, 0, 4, 5, 6, 6, 5, 7];
    const STRIP16: [u16; 4] = [0, 1, 2, 3];
    const RIGHT8: [u8; 6] = [4, 5, 6, 6, 5, 7];
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, TRI_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, TRI_FS)?);
    glBindAttribLocation(p, 0, c"pos".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    let (mut vbo, mut ibo) = (0, 0);
    if mode == Mode::IdxU16Client {
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, V.as_ptr().cast());
    } else {
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(GL_ARRAY_BUFFER, core::mem::size_of_val(&V) as GLsizeiptr, V.as_ptr().cast(), GL_STATIC_DRAW);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, core::ptr::null());
        glGenBuffers(1, &mut ibo);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
    }
    glEnableVertexAttribArray(0);
    let element = |data: *const core::ffi::c_void, bytes: usize| {
        glBufferData(GL_ELEMENT_ARRAY_BUFFER, bytes as GLsizeiptr, data, GL_STATIC_DRAW)
    };
    let (want_red, want_fnv) = match mode {
        Mode::IdxU16Client => {
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, RIGHT16.as_ptr().cast());
            (2048, 0x8eb2fdc5)
        }
        Mode::IdxU16Ibo => {
            element(RIGHT16.as_ptr().cast(), core::mem::size_of_val(&RIGHT16));
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, core::ptr::null());
            (2048, 0x8eb2fdc5)
        }
        Mode::IdxU16Offset => {
            element(OFFSET16.as_ptr().cast(), core::mem::size_of_val(&OFFSET16));
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, 6 as *const core::ffi::c_void);
            (2048, 0x8eb2fdc5)
        }
        Mode::IdxStripIbo => {
            element(STRIP16.as_ptr().cast(), core::mem::size_of_val(&STRIP16));
            glDrawElements(GL_TRIANGLE_STRIP, 4, GL_UNSIGNED_SHORT, core::ptr::null());
            (2048, 0xf350fdc5)
        }
        _ => {
            element(RIGHT8.as_ptr().cast(), core::mem::size_of_val(&RIGHT8));
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, core::ptr::null());
            (2048, 0x8eb2fdc5)
        }
    };
    gl_error("glDrawElements")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let red = px.chunks(4).filter(|q| *q == RED).count();
    let other = px.chunks(4).filter(|q| *q != RED && *q != BLUE).count();
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    println!(
        "panthor-gl: pixels 4096: red {red} other {other} fnv1a {fnv1a:08x} (Mesa: red {want_red} other 0 fnv1a {want_fnv:08x})"
    );
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(red == want_red && other == 0 && fnv1a == want_fnv && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// A prims.c mode: one red draw in `prim`'s GL mode over the blue clear,
/// the read-back compared with Mesa's (its red count and FNV-1a hash), and
/// the same failure conditions as the triangle modes.
unsafe fn prim_frame(prim: &Prim, lost0: u32, refused0: u32) -> Result<bool, String> {
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, prim.vs)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, TRI_FS)?);
    glBindAttribLocation(p, 0, c"pos".as_ptr());
    glBindAttribLocation(p, 1, c"size".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, prim.vertices.as_ptr().cast());
    match prim.size {
        PointSize::Shader => {}
        PointSize::Attribute(sizes) => {
            glVertexAttribPointer(1, 1, GL_FLOAT, 0, 0, sizes.as_ptr().cast());
            glEnableVertexAttribArray(1);
        }
        PointSize::Uniform(v) => {
            let l = glGetUniformLocation(p, c"u_size".as_ptr());
            if l < 0 {
                return Err("u_size is not an active uniform".into());
            }
            glUniform1f(l, v);
        }
    }
    glDrawArrays(prim.gl, 0, (prim.vertices.len() / 2) as GLsizei);
    gl_error("glDrawArrays")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let at = |x: usize, y: usize| -> [u8; 4] { px[(y * W + x) * 4..][..4].try_into().unwrap() };
    let red = (0..W * H).filter(|&i| at(i % W, i / W) == RED).count();
    let other = (0..W * H).filter(|&i| ![RED, BLUE].contains(&at(i % W, i / W))).count();
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    let list = |v: Vec<usize>| v.iter().map(|n| format!(" {n}")).collect::<String>();
    println!("panthor-gl: row 20 red x:{}", list((0..W).filter(|&x| at(x, 20) == RED).collect()));
    println!("panthor-gl: column 40 red y:{}", list((0..H).filter(|&y| at(40, y) == RED).collect()));
    println!(
        "panthor-gl: pixels 4096: red {red} other {other} fnv1a {fnv1a:08x} (Mesa: red {} other 0 fnv1a {:08x})",
        prim.red, prim.fnv1a
    );
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(red == prim.red && other == 0 && fnv1a == prim.fnv1a && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// A tex.c mode ([`TexMode`]): the textured quad over the blue clear,
/// every pixel against its prediction and the read-back's hash against
/// Mesa's, and the same failure conditions as the triangle modes.
unsafe fn tex_frame(t: &TexMode, lost0: u32, refused0: u32) -> Result<bool, String> {
    let n = t.side;
    let texels: Vec<u8> =
        (0..n * n).flat_map(|k| [TexMode::channel(k % n), TexMode::channel(k / n), 128, 255]).collect();
    let mut tex = 0;
    glGenTextures(1, &mut tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    glTexImage2D(
        GL_TEXTURE_2D,
        0,
        GL_RGBA as GLint,
        n as GLsizei,
        n as GLsizei,
        0,
        GL_RGBA,
        GL_UNSIGNED_BYTE,
        texels.as_ptr().cast(),
    );
    let filter = if t.linear { GL_LINEAR } else { GL_NEAREST } as GLint;
    let wrap = match t.wrap {
        Wrap::Repeat => GL_REPEAT,
        Wrap::Mirror => GL_MIRRORED_REPEAT,
        Wrap::Clamp => GL_CLAMP_TO_EDGE,
    } as GLint;
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filter);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filter);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, wrap);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, wrap);
    gl_error("the texture")?;
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, TEX_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, TEX_FS)?);
    glBindAttribLocation(p, 0, c"a_pos".as_ptr());
    glBindAttribLocation(p, 1, c"a_uv".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    const POS: [f32; 24] = [
        -1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, -1.0,
        1.0, 0.0, 1.0,
    ];
    const UV: [f32; 12] = [-0.5, -0.5, 1.5, -0.5, 1.5, 1.5, -0.5, -0.5, 1.5, 1.5, -0.5, 1.5];
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr().cast());
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr().cast());
    glEnableVertexAttribArray(1);
    glDrawArrays(GL_TRIANGLES, 0, 6);
    gl_error("glDrawArrays")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let mut bad = 0usize;
    for y in 0..H {
        for x in 0..W {
            let want = [t.predict(x), t.predict(y), 128, 255];
            let got: [u8; 4] = px[(y * W + x) * 4..][..4].try_into().unwrap();
            if got != want {
                if bad < 8 {
                    println!("panthor-gl: bad ({x}, {y}): got {got:02x?} want {want:02x?}");
                }
                bad += 1;
            }
        }
    }
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    let row: Vec<String> = (0..W)
        .step_by(8)
        .map(|x| format!("{:08x}", u32::from_le_bytes(px[(32 * W + x) * 4..][..4].try_into().unwrap())))
        .collect();
    println!("panthor-gl: row 32: {}", row.join(" "));
    println!("panthor-gl: pixels 4096: not as predicted {bad}; fnv1a {fnv1a:08x} (Mesa's {:08x})", t.fnv1a);
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(bad == 0 && fnv1a == t.fnv1a && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// A mip.c mode ([`MipMode`]): the four bands over the black clear, every
/// pixel its band's level's colour, the hash Mesa's, and the same failure
/// conditions as the triangle modes.
unsafe fn mip_frame(t: &MipMode, lost0: u32, refused0: u32) -> Result<bool, String> {
    let mut tex = 0;
    glGenTextures(1, &mut tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    let mut levels = 0;
    let mut s = t.side;
    while s >= 1 {
        let texels: Vec<u8> = (0..s * s).flat_map(|_| MIP_LEVEL[levels]).collect();
        let n = s as GLsizei;
        glTexImage2D(
            GL_TEXTURE_2D,
            levels as GLint,
            GL_RGBA as GLint,
            n,
            n,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            texels.as_ptr().cast(),
        );
        levels += 1;
        s /= 2;
    }
    for (pname, v) in [
        (GL_TEXTURE_MIN_FILTER, t.min),
        (GL_TEXTURE_MAG_FILTER, t.mag),
        (GL_TEXTURE_WRAP_S, GL_REPEAT),
        (GL_TEXTURE_WRAP_T, GL_REPEAT),
    ] {
        glTexParameteri(GL_TEXTURE_2D, pname, v as GLint);
    }
    gl_error("the chain")?;
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, TEX_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, TEX_FS)?);
    glBindAttribLocation(p, 0, c"a_pos".as_ptr());
    glBindAttribLocation(p, 1, c"a_uv".as_ptr());
    glLinkProgram(p);
    glUseProgram(p);
    glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    glClearColor(0.0, 0.0, 0.0, 1.0);
    glClear(GL_COLOR_BUFFER_BIT);
    /* band b: rows [16 b, 16 b + 16), 2^b texels of level 0 a pixel */
    let (mut pos, mut uv) = (Vec::new(), Vec::new());
    for b in 0..4 {
        let (y0, y1) = (-1.0 + b as f32 * 0.5, -0.5 + b as f32 * 0.5);
        let (u1, v1) = ((1 << b) as f32 * W as f32 / t.side as f32, (1 << b) as f32 * 16.0 / t.side as f32);
        for q in [
            [-1.0, y0, 0.0, 0.0],
            [1.0, y0, u1, 0.0],
            [1.0, y1, u1, v1],
            [-1.0, y0, 0.0, 0.0],
            [1.0, y1, u1, v1],
            [-1.0, y1, 0.0, v1],
        ] {
            pos.extend([q[0], q[1], 0.0, 1.0]);
            uv.extend([q[2], q[3]]);
        }
    }
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, pos.as_ptr().cast());
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, uv.as_ptr().cast());
    glEnableVertexAttribArray(1);
    glDrawArrays(GL_TRIANGLES, 0, 24);
    gl_error("glDrawArrays")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let mut bad = 0usize;
    for y in 0..H {
        for x in 0..W {
            let want = MIP_LEVEL[(y / 16).min(levels - 1)];
            let got: [u8; 4] = px[(y * W + x) * 4..][..4].try_into().unwrap();
            if got != want {
                if bad < 8 {
                    println!("panthor-gl: bad ({x}, {y}): got {got:02x?} want {want:02x?}");
                }
                bad += 1;
            }
        }
    }
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    let bands: Vec<String> = (0..4).map(|b| format!("{:02x?}", &px[((b * 16 + 8) * W + 32) * 4..][..4])).collect();
    println!("panthor-gl: bands: {}", bands.join(" "));
    println!("panthor-gl: pixels 4096: not as predicted {bad}; fnv1a {fnv1a:08x} (Mesa's {MIP_FNV1A:08x})");
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(bad == 0 && fnv1a == MIP_FNV1A && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// A fmt.c mode: the quad sampling the mode's texture over the blue clear,
/// every pixel its texel's predicted value, the hash Mesa's, and the same
/// failure conditions as the triangle modes.
unsafe fn fmt_frame(mode: usize, lost0: u32, refused0: u32) -> Result<bool, String> {
    let mut tex = 0;
    glGenTextures(1, &mut tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    let (tw, th, want): (usize, usize, Vec<[u8; 4]>) = match mode {
        0 => {
            let data: Vec<u8> = FMT_COLS.iter().flat_map(|c| etc1_flat(*c)).collect();
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, 0x8D64, 16, 8, 0, data.len() as GLsizei, data.as_ptr().cast());
            let want = (0..16 * 8)
                .map(|k| {
                    let c = FMT_COLS[(k / 16 / 4) * 4 + (k % 16) / 4];
                    [etc1_value(c[0]), etc1_value(c[1]), etc1_value(c[2]), 255]
                })
                .collect();
            (16, 8, want)
        }
        1 => {
            let mut data = [0u8; 64];
            for b in 0..4 {
                data[16 * b] = FMT_ALPHAS[b];
                data[16 * b + 8..16 * b + 16].copy_from_slice(&etc1_flat(FMT_COLS[b]));
            }
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, 0x9278, 8, 8, 0, 64, data.as_ptr().cast());
            let want = (0..64)
                .map(|k| {
                    let b = (k / 8 / 4) * 2 + (k % 8) / 4;
                    let c = FMT_COLS[b];
                    [etc1_value(c[0]), etc1_value(c[1]), etc1_value(c[2]), FMT_ALPHAS[b]]
                })
                .collect();
            (8, 8, want)
        }
        _ => {
            let data: Vec<u8> = FMT_SRGB.iter().flat_map(|&v| [v, v, v, 255]).collect();
            glTexImage2D(GL_TEXTURE_2D, 0, 0x8C43, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, data.as_ptr().cast());
            (4, 4, FMT_LINEAR.iter().map(|&v| [v, v, v, 255]).collect())
        }
    };
    for (pname, v) in [
        (GL_TEXTURE_MIN_FILTER, GL_NEAREST),
        (GL_TEXTURE_MAG_FILTER, GL_NEAREST),
        (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
        (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
    ] {
        glTexParameteri(GL_TEXTURE_2D, pname, v as GLint);
    }
    gl_error("the texture")?;
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, TEX_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, TEX_FS)?);
    glBindAttribLocation(p, 0, c"a_pos".as_ptr());
    glBindAttribLocation(p, 1, c"a_uv".as_ptr());
    glLinkProgram(p);
    glUseProgram(p);
    glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    const POS: [f32; 24] = [
        -1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, -1.0,
        1.0, 0.0, 1.0,
    ];
    const UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr().cast());
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr().cast());
    glEnableVertexAttribArray(1);
    glDrawArrays(GL_TRIANGLES, 0, 6);
    gl_error("glDrawArrays")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let mut bad = 0usize;
    for y in 0..H {
        for x in 0..W {
            let w = want[(y * th / H) * tw + x * tw / W];
            let got: [u8; 4] = px[(y * W + x) * 4..][..4].try_into().unwrap();
            if got != w {
                if bad < 8 {
                    println!("panthor-gl: bad ({x}, {y}): got {got:02x?} want {w:02x?}");
                }
                bad += 1;
            }
        }
    }
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    let mesa = FMT_MODES[mode].2;
    println!("panthor-gl: pixels 4096: not as predicted {bad}; fnv1a {fnv1a:08x} (Mesa's {mesa:08x})");
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(bad == 0 && fnv1a == mesa && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// preload_zs.c's modes (0dce654a8:tools/mali/board-ref/2026-10-19-preload-zs): the
/// flag, its line, and (frame 2 stencil-tested, frame 2 clears the other
/// plane first). Mesa's fnv1a is bffbedc5 for all four.
const PRELOAD_ZS_MODES: [(&str, &str, (bool, bool)); 4] = [
    ("--preload-zs-depth", "GL preload_zs.c depth: depth and stencil preloaded, depth-tested", (false, false)),
    ("--preload-zs-stencil", "GL preload_zs.c stencil: depth and stencil preloaded, stencil-tested", (true, false)),
    ("--preload-zs-depth-sclear", "GL preload_zs.c depth_sclear: the depth preloaded alone", (false, true)),
    ("--preload-zs-stencil-zclear", "GL preload_zs.c stencil_zclear: the stencil preloaded alone", (true, true)),
];

/// preload.c's and preload_zs.c's FNV-1a on the board's Mesa.
const PRELOAD_ZS_FNV1A: u32 = 0xbffbedc5;

/// preload.c's vertex shader.
const PRELOAD_VS: &str = "attribute vec2 pos; void main(){ gl_Position = vec4(pos, 0.0, 1.0); }";

/// A preload.c mode after the blue clear: red over columns 0-31, glFinish,
/// then green over columns 32-63 (`half`) or 16-47 (`strip`) in a frame
/// without a clear, which starts from the first frame's image. Every pixel
/// as predicted, the hash Mesa's, and the same failure conditions as the
/// triangle modes.
unsafe fn preload_frame(strip: bool, lost0: u32, refused0: u32) -> Result<bool, String> {
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, PRELOAD_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, COLOR_FS)?);
    glBindAttribLocation(p, 0, c"pos".as_ptr());
    glLinkProgram(p);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    if ok != 1 {
        return Err(format!("link: {}", info_log(glGetProgramInfoLog, p)));
    }
    glUseProgram(p);
    let col = glGetUniformLocation(p, c"u_color".as_ptr());
    if col < 0 {
        return Err("u_color is not an active uniform".into());
    }
    let quad = |x0: f32, x1: f32, rgb: [f32; 3]| {
        let v = [x0, -1.0, x1, -1.0, x0, 1.0, x1, -1.0, x1, 1.0, x0, 1.0];
        // SAFETY: the caller's GL context; the array outlives the draw.
        unsafe {
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, v.as_ptr().cast());
            glEnableVertexAttribArray(0);
            glUniform4f(col, rgb[0], rgb[1], rgb[2], 1.0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
        }
    };
    quad(-1.0, 0.0, [1.0, 0.0, 0.0]);
    glFinish();
    if strip {
        quad(-0.5, 0.5, [0.0, 1.0, 0.0]);
    } else {
        quad(0.0, 1.0, [0.0, 1.0, 0.0]);
    }
    gl_error("the draws")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let mut bad = 0usize;
    for y in 0..H {
        for x in 0..W {
            let w = match (strip, x) {
                (true, 0..16) | (false, 0..32) => RED,
                (true, 48..) => BLUE,
                _ => GREEN,
            };
            let got: [u8; 4] = px[(y * W + x) * 4..][..4].try_into().unwrap();
            if got != w {
                if bad < 8 {
                    println!("panthor-gl: bad ({x}, {y}): got {got:02x?} want {w:02x?}");
                }
                bad += 1;
            }
        }
    }
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    /* preload.c's runs (0dce654a8:tools/mali/board-ref/2026-10-19-preload/mesa) */
    let mesa = if strip { 0x4b370dc5 } else { 0xbffbedc5 };
    println!("panthor-gl: pixels 4096: not as predicted {bad}; fnv1a {fnv1a:08x} (Mesa's {mesa:08x})");
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(bad == 0 && fnv1a == mesa && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// `--tex-solid W`: the quad sampling a W x 4 all-green texture, every
/// pixel green, and the same failure conditions as the triangle modes.
unsafe fn tex_solid_frame(w: usize, lost0: u32, refused0: u32) -> Result<bool, String> {
    let texels: Vec<u8> = (0..w * 4).flat_map(|_| GREEN).collect();
    let mut tex = 0;
    glGenTextures(1, &mut tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    glTexImage2D(
        GL_TEXTURE_2D,
        0,
        GL_RGBA as GLint,
        w as GLsizei,
        4,
        0,
        GL_RGBA,
        GL_UNSIGNED_BYTE,
        texels.as_ptr().cast(),
    );
    for (pname, v) in [
        (GL_TEXTURE_MIN_FILTER, GL_NEAREST),
        (GL_TEXTURE_MAG_FILTER, GL_NEAREST),
        (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
        (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
    ] {
        glTexParameteri(GL_TEXTURE_2D, pname, v as GLint);
    }
    gl_error("the texture")?;
    let p = glCreateProgram();
    glAttachShader(p, shader(GL_VERTEX_SHADER, TEX_VS)?);
    glAttachShader(p, shader(GL_FRAGMENT_SHADER, TEX_FS)?);
    glBindAttribLocation(p, 0, c"a_pos".as_ptr());
    glBindAttribLocation(p, 1, c"a_uv".as_ptr());
    glLinkProgram(p);
    glUseProgram(p);
    glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    const POS: [f32; 24] = [
        -1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, -1.0,
        1.0, 0.0, 1.0,
    ];
    const UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr().cast());
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr().cast());
    glEnableVertexAttribArray(1);
    glDrawArrays(GL_TRIANGLES, 0, 6);
    gl_error("glDrawArrays")?;
    let mut px = vec![0u8; W * H * 4];
    glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
    gl_error("glReadPixels")?;
    glFinish();
    let mut counts = std::collections::BTreeMap::<[u8; 4], usize>::new();
    for c in px.chunks(4) {
        *counts.entry(c.try_into().unwrap()).or_default() += 1;
    }
    let listed: Vec<String> = counts.iter().map(|(c, n)| format!("{c:02x?} {n}")).collect();
    println!("panthor-gl: texture {w}x4, rows of {} bytes: read-back {}", 4 * w, listed.join(", "));
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if refused != 0 {
        println!("panthor-gl: first refusal: {}", yakogl::mali::backend::first_refusal().unwrap_or("(none)"));
    }
    Ok(counts.get(&GREEN) == Some(&(W * H)) && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// A zs mode's read-back, counted against its prediction ([`zs_want`]),
/// and the same failure conditions as the triangle modes.
unsafe fn finish_zs(mode: Mode, fb: Option<&str>, scale: u32, lost0: u32, refused0: u32) -> Result<bool, String> {
    let mut px = vec![0u8; W * H * 4];
    // SAFETY: the caller's GL context; a buffer of the size read.
    unsafe {
        glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        gl_error("glReadPixels")?;
        glFinish();
    }
    let (mut counts, mut misplaced) = (std::collections::BTreeMap::<[u8; 4], usize>::new(), 0usize);
    for y in 0..H {
        for x in 0..W {
            let p: [u8; 4] = px[(y * W + x) * 4..][..4].try_into().unwrap();
            *counts.entry(p).or_default() += 1;
            misplaced += usize::from(p != zs_want(mode, x, y));
        }
    }
    let name = |c: &[u8; 4]| match *c {
        RED => "red".to_string(),
        GREEN => "green".to_string(),
        BLUE => "blue".to_string(),
        MAGENTA => "magenta".to_string(),
        c => format!("{c:02x?}"),
    };
    let listed: Vec<String> = counts.iter().map(|(c, n)| format!("{} {n}", name(c))).collect();
    let row: Vec<String> = (0..W)
        .step_by(16)
        .map(|x| format!("x{x} {}", name(&px[(32 * W + x) * 4..][..4].try_into().unwrap())))
        .collect();
    let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    let (subs, open, live, live_hw, bytes, bytes_hw) = osal::counts().ok_or("no OSAL state")?;
    let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
    let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
    println!("panthor-gl: read-back: {}; row 32: {}; misplaced {misplaced}", listed.join(", "), row.join(" "));
    let fnv1a = px.iter().fold(2166136261u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(16777619));
    let hash_ok = match mode {
        Mode::PreloadZs(_) => {
            println!("panthor-gl: fnv1a {fnv1a:08x} (Mesa's {PRELOAD_ZS_FNV1A:08x})");
            fnv1a == PRELOAD_ZS_FNV1A
        }
        _ => true,
    };
    println!(
        "panthor-gl: gpu work lost {lost}, draws refused {refused}; osal submits {subs}, unwaited fences {open}, BOs live {live} (high-water {live_hw}), bytes live {bytes} (high-water {bytes_hw}); group state {:#x} fatal queues {:#x}; vm usable {usable}",
        gs.0, gs.1
    );
    if let Some(path) = fb {
        blit(path, &px, scale)?;
    }
    Ok(misplaced == 0 && hash_ok && lost == 0 && refused == 0 && gs == (0, 0) && usable)
}

/// `--delete-held` (the module documentation), after the clear of texture
/// `tex` through the bound FBO.
unsafe fn delete_held(tex: GLuint, lost0: u32, refused0: u32) -> Result<bool, String> {
    // SAFETY: plain GL calls on the caller's context.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, TRI_VS)?);
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, TRI_FS)?);
        glBindAttribLocation(p, 0, c"pos".as_ptr());
        glLinkProgram(p);
        glUseProgram(p);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, l1::VERTICES.as_ptr().cast());
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFlush();
        gl_error("the draw into A")?;
        let (held0, _) = osal::pending().ok_or("no OSAL state")?;
        glDeleteTextures(1, &tex);
        let (held1, _) = osal::pending().ok_or("no OSAL state")?;
        println!("panthor-gl: after glDeleteTextures(A): osal BOs held for a kick {held1} (before it {held0})");
        let (mut b, mut fbo) = (0, 0);
        glGenTextures(1, &mut b);
        glBindTexture(GL_TEXTURE_2D, b);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            W as GLsizei,
            H as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            core::ptr::null(),
        );
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, b, 0);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        let mut px = vec![0u8; W * H * 4];
        glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        glFinish();
        gl_error("B's clear and read-back")?;
        let (held2, held_hw) = osal::pending().ok_or("no OSAL state")?;
        let t = tally(&px, None, 0..0);
        let lost = yakogl::backend::yakogl_gpu_work_lost() - lost0;
        let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
        let gs = osal::group_state().ok_or("no group")?.map_err(|e| e.to_string())?;
        let usable = osal::vm_usable().ok_or("no vm")?.map_err(|e| e.to_string())?;
        println!(
            "panthor-gl: after B's read-back: osal BOs held for a kick {held2} (high-water {held_hw}); B: red {} green {} blue {} other {} misplaced {}",
            t.red, t.green, t.blue, t.other, t.misplaced
        );
        println!("panthor-gl: gpu work lost {lost}, draws refused {refused}; group state {:#x} fatal queues {:#x}; vm usable {usable}", gs.0, gs.1);
        Ok(held1 == held0 + 1
            && held2 == 0
            && t == Tally { blue: 4096, ..Tally::default() }
            && lost == 0
            && refused == 0
            && gs == (0, 0)
            && usable)
    }
}

/// A sysfs attribute of the framebuffer device `fb` (e.g. "fb0").
fn fb_attr(fb: &str, name: &str) -> Result<String, String> {
    let p = format!("/sys/class/graphics/{fb}/{name}");
    std::fs::read_to_string(&p).map(|s| s.trim().to_string()).map_err(|e| format!("read {p}: {e}"))
}

fn blit(path: &str, target: &[u8], scale: u32) -> Result<(), String> {
    let fb = path.rsplit('/').next().unwrap_or(path);
    let bpp = fb_attr(fb, "bits_per_pixel")?;
    let stride: u32 = fb_attr(fb, "stride")?.parse().map_err(|_| format!("{fb}: stride"))?;
    let vsize = fb_attr(fb, "virtual_size")?;
    if bpp != "32" {
        return Err(format!("{path}: {bpp} bits per pixel, the blit writes 32-bpp XRGB8888 only"));
    }
    let (w, h) = vsize
        .split_once(',')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .ok_or_else(|| format!("{fb}: virtual_size {vsize:?}"))?;
    let (rect, rows) = l1::fb_rows(target, scale, w, h, stride)?;
    let f = OpenOptions::new().write(true).open(path).map_err(|e| format!("open {path}: {e}"))?;
    for (off, row) in &rows {
        f.write_all_at(row, *off as u64).map_err(|e| format!("write {path} at {off:#x}: {e}"))?;
    }
    println!(
        "panthor-gl: fb {path}: wrote x [{}, {}) y [{}, {}): the 64x64 target at {scale}x inside a 1-pixel white border; nothing else written",
        rect.x0, rect.x1, rect.y0, rect.y1
    );
    Ok(())
}

fn main() -> ExitCode {
    println!("panthor-gl: build {}", yakogl_mali_panthor::BUILD);
    let (mut mode, mut fb, mut scale) = (Mode::Red, None, 8);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--green" => mode = Mode::Green,
            "--clear-only" => mode = Mode::ClearOnly,
            "--delete-held" => mode = Mode::DeleteHeld,
            "--varying" => mode = Mode::Varying,
            "--uniform-green" => mode = Mode::UniformGreen,
            "--uniform-red" => mode = Mode::UniformRed,
            "--mat4" => mode = Mode::Mat4,
            "--color-uv" => mode = Mode::ColorUv,
            "--fvg-fill-color" => mode = Mode::FvgFillColor,
            "--fvg-fill-color-scissor" => mode = Mode::FvgFillColorScissor,
            "--fragcoord" => mode = Mode::FragCoord(false),
            "--fragcoord-flip" => mode = Mode::FragCoord(true),
            "--depth" => mode = Mode::Depth,
            "--nodepth" => mode = Mode::NoDepth,
            "--clearhide" => mode = Mode::ClearHide,
            "--mask" => mode = Mode::Mask,
            "--nocolour" => mode = Mode::NoColour,
            "--scissor-clear" => mode = Mode::ScissorClear,
            "--lines" => mode = Mode::Lines,
            "--line-strip" => mode = Mode::LineStrip,
            "--line-loop" => mode = Mode::LineLoop,
            "--tri-strip" => mode = Mode::TriStrip,
            "--tri-fan" => mode = Mode::TriFan,
            "--flip-tri" => mode = Mode::FlipTri,
            "--flip-frontcw" => mode = Mode::FlipFrontCw,
            "--flip-viewport" => mode = Mode::FlipViewport,
            "--flip-scissor-draw" => mode = Mode::FlipScissorDraw,
            "--flip-scissor-clear" => mode = Mode::FlipScissorClear,
            "--flip-manylines" => mode = Mode::FlipManyLines,
            "--flip-manylines-colour1" => mode = Mode::FlipManyLinesColour1,
            "--tex-solid" => match args.next().and_then(|s| s.parse().ok()) {
                Some(w) if (2..=64).contains(&w) && w % 2 == 0 => mode = Mode::TexSolid(w),
                _ => {
                    eprintln!("panthor-gl: --tex-solid takes an even width 2..64");
                    return ExitCode::from(2);
                }
            },
            s if PRELOAD_ZS_MODES.iter().any(|t| t.0 == s) => {
                mode = Mode::PreloadZs(PRELOAD_ZS_MODES.iter().position(|t| t.0 == s).expect("found"))
            }
            s if FMT_MODES.iter().any(|t| t.0 == s) => {
                mode = Mode::Fmt(FMT_MODES.iter().position(|t| t.0 == s).expect("found"))
            }
            s if MIP_MODES.iter().any(|t| t.flag == s) => {
                mode = Mode::Mip(MIP_MODES.iter().position(|t| t.flag == s).expect("found"))
            }
            s if TEX_MODES.iter().any(|t| t.flag == s) => {
                mode = Mode::Tex(TEX_MODES.iter().position(|t| t.flag == s).expect("found"))
            }
            "--window-tri" => mode = Mode::WindowTri,
            "--preload-half" => mode = Mode::Preload(false),
            "--preload-strip" => mode = Mode::Preload(true),
            "--window-mid" => mode = Mode::WindowMid,
            "--window-xrgb-red" => mode = Mode::WindowXrgbRed,
            "--window-xrgb-mid" => mode = Mode::WindowXrgbMid,
            "--window-565-red" => mode = Mode::Window565Red,
            "--window-preload" => mode = Mode::WindowPreload(yakogl::gl::egl::GLES_NATIVE_FORMAT_RGBA8888),
            "--window-xrgb-preload" => mode = Mode::WindowPreload(yakogl::gl::egl::GLES_NATIVE_FORMAT_XRGB8888),
            "--window-565-preload" => mode = Mode::WindowPreload(yakogl::gl::egl::GLES_NATIVE_FORMAT_RGB565),
            "--window-565-mid" => mode = Mode::Window565Mid,
            "--idx-u16-client" => mode = Mode::IdxU16Client,
            "--idx-u16-ibo" => mode = Mode::IdxU16Ibo,
            "--idx-u16-offset" => mode = Mode::IdxU16Offset,
            "--idx-strip-ibo" => mode = Mode::IdxStripIbo,
            "--idx-u8-ibo" => mode = Mode::IdxU8Ibo,
            "--msaa-4x" => mode = Mode::Msaa(4),
            "--msaa-1x" => mode = Mode::Msaa(1),
            "--msaa-cov50" => mode = Mode::MsaaCov(false),
            "--msaa-cov50inv" => mode = Mode::MsaaCov(true),
            b if b.strip_prefix("--vcomp-").is_some_and(|n| VCOMP_MODES.iter().any(|m| m.0 == n)) => {
                let n = &b["--vcomp-".len()..];
                mode = Mode::Vcomp(VCOMP_MODES.iter().position(|m| m.0 == n).unwrap_or(0));
            }
            b if b.strip_prefix("--fsconst-").is_some_and(|n| FSCONST_MODES.iter().any(|m| m.0 == n)) => {
                let n = &b["--fsconst-".len()..];
                mode = Mode::Fsconst(FSCONST_MODES.iter().position(|m| m.0 == n).unwrap_or(0));
            }
            b if b.strip_prefix("--fneg-").is_some_and(|n| FNEG_MODES.iter().any(|m| m.0 == n)) => {
                let n = &b["--fneg-".len()..];
                mode = Mode::Fneg(FNEG_MODES.iter().position(|m| m.0 == n).unwrap_or(0));
            }
            b if b.strip_prefix("--fabs-").is_some_and(|n| FABS_MODES.iter().any(|m| m.0 == n)) => {
                let n = &b["--fabs-".len()..];
                mode = Mode::Fabs(FABS_MODES.iter().position(|m| m.0 == n).unwrap_or(0));
            }
            "--pcoord-fbo" => mode = Mode::Pcoord(false),
            "--pcoord-pb" => mode = Mode::Pcoord(true),
            b if b.strip_prefix("--points-").is_some_and(|n| POINT_MODES.iter().any(|m| m.0 == n)) => {
                let n = &b["--points-".len()..];
                mode = Mode::Points(POINT_MODES.iter().position(|m| m.0 == n).unwrap_or(0));
            }
            b if b.strip_prefix("--poff-").is_some_and(|n| POFFS.iter().any(|m| m.0 == n)) => {
                let n = &b["--poff-".len()..];
                mode = Mode::Poff(POFFS.iter().position(|m| m.0 == n).unwrap_or(0));
            }
            b if b.strip_prefix("--blend-").is_some_and(|n| BLENDS.iter().any(|(m, _)| *m == n)) => {
                let n = &b["--blend-".len()..];
                mode = Mode::Blend(BLENDS.iter().position(|(m, _)| *m == n).unwrap_or(0));
            }
            "--fb" => fb = args.next(),
            "--scale" => match args.next().and_then(|s| s.parse().ok()) {
                Some(n) => scale = n,
                None => {
                    eprintln!("panthor-gl: --scale takes a number");
                    return ExitCode::from(2);
                }
            },
            s => {
                eprintln!("panthor-gl: unknown argument {s}");
                return ExitCode::from(2);
            }
        }
    }
    let what = match mode {
        Mode::Red => "GL triangle (expect red 2048 blue 2048 other 0 misplaced 0)",
        Mode::Green => "GL triangle --green (expect green 2048 blue 2048 other 0 misplaced 0)",
        Mode::Varying => "GL tri_varying, green vertices (expect green 2048 blue 2048 other 0 misplaced 0)",
        Mode::UniformGreen => "GL uniform colour green (expect green 2048 blue 2048 other 0 misplaced 0)",
        Mode::UniformRed => "GL uniform colour red (expect red 2048 blue 2048 other 0 misplaced 0)",
        Mode::Mat4 => "GL mat4 translate +0.5 x (expect red 2048 in columns 16..48, blue 2048, other 0, misplaced 0)",
        Mode::ColorUv => "GL color_uv, two varyings (expect every pixel the per-pixel prediction, 4096 of 4096)",
        Mode::FvgFillColor => "GL femtovg fill_color.nodiscard.noscissor (expect every pixel R = B = 0 and G = A, a tent along x times a ramp along y; the picture's hash is compared with Mesa's)",
        Mode::FvgFillColorScissor => "GL femtovg fill_color.nodiscard, two frames (expect every pixel R = B = 0 and G = A, zero outside the 40x24 scissor window, the rounded frame differing from the rectangle's at the corners only; the pictures' hashes are compared with Mesa's)",
        Mode::FragCoord(false) => "GL fragcoord.c into a 64x48 FBO (expect every pixel R, G = gl_FragCoord.xy / 64, B 0, A 255; Mesa's fnv1a 99cb74c5)",
        Mode::FragCoord(true) => "GL fragcoord.c into the 64x48 top-down surface (expect every pixel R, G = gl_FragCoord.xy / 64, B 0, A 255; Mesa's fnv1a 99cb74c5)",
        Mode::ClearOnly => "GL clear only (expect blue 4096)",
        Mode::Depth => "GL zs.c depth: LESS, green near then red far (expect columns 0-15 red, 16-63 green)",
        Mode::NoDepth => "GL zs.c nodepth: test off (expect columns 0-47 red, 48-63 green)",
        Mode::ClearHide => "GL zs.c clearhide: depth cleared to 0.25 hides red at 0.5 (expect blue 4096)",
        Mode::Mask => "GL zs.c mask: R-only white clear over blue (expect magenta 4096)",
        Mode::NoColour => {
            "GL zs.c nocolour: depth-only green near, then red far (expect columns 0-15 red, the rest blue)"
        }
        Mode::ScissorClear => "GL scissored red clear (16,16,32,32) over blue (expect red 1024 in the box, blue 3072)",
        Mode::Lines => "GL prims.c lines (expect Mesa's red 127 other 0 fnv1a da93439d)",
        Mode::LineStrip => "GL prims.c line_strip (expect Mesa's red 84 other 0 fnv1a 793220e5)",
        Mode::LineLoop => "GL prims.c line_loop (expect Mesa's red 144 other 0 fnv1a 10261645)",
        Mode::TriStrip => "GL prims.c tri_strip (expect Mesa's red 2048 other 0 fnv1a f350fdc5)",
        Mode::TriFan => "GL prims.c tri_fan (expect Mesa's red 2048 other 0 fnv1a 8eb2fdc5)",
        Mode::FlipTri => "GL flip.c tri, top-down 64x48 (expect Mesa's pbuffer red 496 other 0 fnv1a 890abd45)",
        Mode::FlipFrontCw => "GL flip.c frontcw, top-down 64x48 (expect Mesa's pbuffer red 496 other 0 fnv1a 890abd45)",
        Mode::FlipViewport => "GL flip.c viewport, top-down 64x48 (expect Mesa's pbuffer red 512 other 0 fnv1a fd5285c5)",
        Mode::FlipScissorDraw => {
            "GL flip.c scissor_draw, top-down 64x48 (expect Mesa's pbuffer red 320 other 0 fnv1a 073d8bc5)"
        }
        Mode::FlipScissorClear => {
            "GL flip.c scissor_clear, top-down 64x48 (expect Mesa's pbuffer red 320 other 0 fnv1a 073d8bc5)"
        }
        Mode::FlipManyLines => {
            "GL gl_test case 56's calls, top-down 64x64 (expect Mesa's: 0 rows lit, the line pixel ff000000)"
        }
        Mode::FlipManyLinesColour1 => {
            "GL case 56 with a_color enabled, top-down 64x64 (expect Mesa's: every row lit, ffffffff)"
        }
        Mode::Tex(i) => TEX_MODES[i].what,
        Mode::TexSolid(_) => "GL a W x 4 all-green texture, nearest, clamped (expect green 4096)",
        Mode::Mip(i) => MIP_MODES[i].what,
        Mode::Fmt(i) => FMT_MODES[i].1,
        Mode::PreloadZs(i) => PRELOAD_ZS_MODES[i].1,
        Mode::Preload(false) => {
            "GL preload.c half: red, glFinish, green without a clear (expect red columns 0-31, green 32-63, Mesa's \
             fnv1a bffbedc5)"
        }
        Mode::Preload(true) => {
            "GL preload.c strip: red, glFinish, green over 16-47 without a clear (expect red 0-15, green 16-47, \
             blue 48-63, Mesa's fnv1a 4b370dc5)"
        }
        Mode::WindowMid => "GL window RGBA8888, fmt.c mid (expect Mesa's fnv1a b343e245, own memory top-down 48 of 48)",
        Mode::WindowXrgbRed => "GL window XRGB8888, fmt.c red (expect Mesa's fnv1a 890abd45, own memory top-down 48 of 48)",
        Mode::WindowXrgbMid => "GL window XRGB8888, fmt.c mid (expect Mesa's fnv1a b343e245, own memory top-down 48 of 48)",
        Mode::Window565Red => "GL window RGB565, fmt.c red (expect Mesa's fnv1a 890abd45, own memory top-down 48 of 48)",
        Mode::WindowPreload(_) => {
            "GL window, preload_fmt.c: red, glFinish, green over 16-47 without a clear (expect Mesa's fnv1a 9a3071c5, own \
             memory top-down 48 of 48)"
        }
        Mode::Window565Mid => {
            "GL window RGB565, fmt.c mid, dithered (expect Mesa's fnv1a 04c57ea5, own memory top-down 48 of 48)"
        }
        Mode::WindowTri => {
            "GL EGL window over own pool memory, flip.c tri (expect Mesa's pbuffer red 496 fnv1a 890abd45, own memory top-down 48 of 48)"
        }
        Mode::IdxU16Client => "GL idx.c u16_client (expect Mesa's red 2048 other 0 fnv1a 8eb2fdc5)",
        Mode::IdxU16Ibo => "GL idx.c u16_ibo (expect Mesa's red 2048 other 0 fnv1a 8eb2fdc5)",
        Mode::IdxU16Offset => "GL idx.c u16_offset (expect Mesa's red 2048 other 0 fnv1a 8eb2fdc5)",
        Mode::IdxStripIbo => "GL idx.c strip_ibo (expect Mesa's red 2048 other 0 fnv1a f350fdc5)",
        Mode::IdxU8Ibo => "GL idx.c u8_ibo (expect Mesa's red 2048 other 0 fnv1a 8eb2fdc5)",
        Mode::Msaa(4) => "GL case 16's draw, 4x pbuffer (expect Mesa's fnv1a 3c0190c5, the diagonal half covered)",
        Mode::Msaa(_) => "GL case 16's draw, 1x pbuffer (expect Mesa's fnv1a 570010c5)",
        Mode::MsaaCov(false) => "GL case 16's 4x draw, glSampleCoverage(0.5) (expect Mesa's fnv1a d8c31dc5)",
        Mode::MsaaCov(true) => "GL case 16's 4x draw, glSampleCoverage(0.5, inverted) (expect Mesa's fnv1a a8c31dc5)",
        Mode::Vcomp(i) => &format!("GL vcomp.c {} (expect Mesa's fnv1a {:08x})", VCOMP_MODES[i].0, VCOMP_MODES[i].4),
        Mode::Fsconst(i) => {
            &format!("GL fsconst.c {} (expect Mesa's fnv1a {:08x})", FSCONST_MODES[i].0, FSCONST_MODES[i].4)
        }
        Mode::Fneg(i) => &format!("GL fneg.c {} (expect Mesa's fnv1a {:08x})", FNEG_MODES[i].0, FNEG_MODES[i].4),
        Mode::Fabs(i) => &format!("GL fabs.c {} (expect Mesa's fnv1a {:08x})", FABS_MODES[i].0, FABS_MODES[i].4),
        Mode::Pcoord(false) => "GL pcoord.c fbo, case 83's sprite (expect Mesa's fnv1a fb970d45)",
        Mode::Pcoord(true) => "GL pcoord.c pb, case 83's sprite in a pbuffer (expect Mesa's fnv1a fb970d45)",
        Mode::Points(i) => &format!(
            "GL points.c {} (expect Mesa's red {} other 0 fnv1a {:08x})",
            POINT_MODES[i].0, POINT_MODES[i].1.red, POINT_MODES[i].1.fnv1a
        ),
        Mode::Poff(i) => &format!("GL poff.c {} (expect Mesa's fnv1a {:08x})", POFFS[i].0, POFFS[i].4),
        Mode::Blend(i) => &format!("GL blend.c {} (expect Mesa's fnv1a {:08x})", BLENDS[i].0, BLENDS[i].1),
        Mode::DeleteHeld => "GL delete while in flight (expect 1 BO held after glDeleteTextures, 0 after the next kick completes, blue 4096)",
    };
    println!("panthor-gl: {what}");
    match run(mode, fb.as_deref(), scale) {
        Ok(true) => {
            println!("panthor-gl: {what}: PASS");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("panthor-gl: {what}: FAIL");
            ExitCode::FAILURE
        }
        Err(e) => {
            println!("panthor-gl: {what}: ERROR {e}");
            ExitCode::FAILURE
        }
    }
}
