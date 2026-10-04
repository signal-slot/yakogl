// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL core's resolved draw ([`GlesDraw`]) in gallium's terms
//! ([`V3dJobState`]), and the draws this backend refuses.
//!
//! The job builder (`driver/src/v3d/v3d_job.c`) transcribes Mesa's v3d
//! driver, which takes gallium state; this file is the part of Mesa's
//! state tracker (st/mesa) that turns GL state into it, for what a
//! [`GlesDraw`] carries. Each rule names the st/mesa function it follows. It
//! is pure (no OS, no GPU), so every rule has a host test.
//!
//! What the M6 first cut does not draw is refused here by name, never drawn
//! another way (docs/online-shaderc/v3d-backend.md, "M6"). Each refusal is its
//! own message, because the backend's log is how a board run says which
//! gl_test case stopped at which missing feature.

use super::ffi::*;
use crate::backend::*;
use crate::shaderc::*;

/// Why a draw is not drawn. The string is the log line's reason, one per
/// cause (a histogram of refusals is keyed on it).
pub type Refusal = &'static str;

/// The draws the M6 first cut does not draw, checked before anything is
/// recorded.
pub fn refusal(d: &GlesDraw) -> Option<Refusal> {
    /* one reason per format: the log names the value it saw */
    match d.target_format {
        GLES_SURF_RGBA8888 | GLES_SURF_XRGB8888 | GLES_SURF_RGB565 => {}
        GLES_SURF_RGBA16F => return Some("an RGBA16F colour target is not written yet"),
        GLES_SURF_R16F => return Some("an R16F colour target is not written yet"),
        GLES_SURF_R32F => return Some("an R32F colour target is not written yet"),
        _ => return Some("a colour target of a format the backend does not know"),
    }
    /* 4x multisampling (docs/online-shaderc/v3d-backend.md
     * "Multisampling"): the V3D's one multisampled mode (V3D_MAX_SAMPLES 4,
     * Mesa's v3d_limits.h) */
    if d.samples > 1 && d.samples != 4 {
        return Some("a sample count other than 1 and 4");
    }
    /* several colour attachments: the backend's extra_targets and its
     * attachment-slot check (multiple render targets) */
    if d.index_count == 0 {
        return None; /* a clear: nothing below applies */
    }
    if d.index_count2 != 0 {
        return Some("a second index list in one draw is not implemented");
    }
    None
}

/// The draws whose GL state is legal but that this backend cannot render
/// yet. The backend refuses them with `-ENOTSUP`, not `-EINVAL`. `-EINVAL`
/// says the draw table is malformed or the core has a bug; `-ENOTSUP` says
/// the backend has a gap (src/backend.rs `draw`, its return contract).
/// Checked after [`refusal`].
pub fn unsupported(d: &GlesDraw) -> Option<Refusal> {
    /* Alpha-to-coverage (ES 2.0.25 4.1.3) on a 4x target. The V3D 4.2 has
     * no control-list bit for it (CFG_BITS, v3d_packet.xml:831-847,
     * mesa-26.2.1). Mesa computes the coverage in the fragment shader:
     * SETMSF(MSF & FTOC(alpha)) at the end of the shader
     * (nir_to_vir.c:2054-2063). That is MSAA step 4, parked. Drawing
     * without it is not an option: dEQP's constancy_alpha_to_coverage would
     * then pass (the red quad covers the green one fully) while nothing was
     * computed. */
    if d.alpha_to_coverage {
        return Some("alpha-to-coverage is not implemented");
    }
    /* glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}) (ES 3.0 4.2.1): the
     * job always stores RT 0 to the target (Target::gpu), so the draw would
     * write attachment 0. Refused rather than drawn into the wrong image
     * (review R-09). */
    if d.num_targets > 1 && d.targets[0].gpu == 0 {
        return Some("draw buffer 0 is GL_NONE while a later draw buffer is written");
    }
    None
}

/* ---- the clear quad -------------------------------------------------------- */

/// Does this clear's colour need the quad? The TLB clear (the RCL's
/// CLEAR_TILE_BUFFERS) writes whole tiles and every channel, so a clear the
/// GL core marked `clear_quad` (scissored, or a partial stencil mask) and a
/// colour clear whose mask leaves channels out are drawn instead, as Mesa's
/// st does for a screen without `clear_scissored` / `clear_masked`
/// (st_cb_clear.c:440-471, mesa-26.2.1).
pub fn colour_needs_quad(d: &GlesDraw) -> bool {
    d.do_clear && (d.clear_quad || d.clear_color_mask & 0xf != 0xf)
}

/// The backend's clear program, as a draw uses it.
#[derive(Clone, Copy)]
pub struct ClearProg<'a> {
    /// The linked program (the fragment shader record the V3D draws with).
    pub fs: &'a ShcShader,
    /// Where its code was uploaded.
    pub code_gpu: u64,
    /// The GL location of `a_pos`.
    pub location: u8,
}

/// Which buffers one clear quad writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadBuffers {
    pub colour: bool,
    pub depth: bool,
    pub stencil: bool,
}

/// The quad's six vertices (two triangles over the whole clip-space square),
/// every one at the clear depth: z = 2 * depth - 1 with the depth range
/// (0, 1), as st_cb_clear.c:309 passes `ctx->Depth.Clear * 2.0f - 1.0f`.
pub fn clear_quad_vertices(depth: f32) -> [[f32; 4]; 6] {
    let z = depth * 2.0 - 1.0;
    [
        [-1.0, -1.0, z, 1.0],
        [1.0, -1.0, z, 1.0],
        [1.0, 1.0, z, 1.0],
        [-1.0, -1.0, z, 1.0],
        [1.0, 1.0, z, 1.0],
        [-1.0, 1.0, z, 1.0],
    ]
}

/// A clear as an ordinary draw of the clear program, with the state of
/// Mesa's `clear_with_quad` (st_cb_clear.c:194-309, mesa-26.2.1):
///
/// - colour: the write mask is the clear's (`clear_color_mask`, :239), or 0
///   when the quad does not clear colour;
/// - depth: test ALWAYS with the write on when it clears depth (:254-255),
///   otherwise off;
/// - stencil: ALWAYS, REPLACE on fail, depth-fail and pass, value mask 0xff,
///   the clear's write mask and its value as the reference, both faces
///   (:261-268), otherwise off;
/// - the viewport the whole target and the depth range (0, 1) (:284-285),
///   no blend, no cull, no polygon offset.
///
/// One difference: Mesa's quad covers the scissor rectangle (:300-308); this
/// one covers the target and keeps the clear's scissor, whose translation
/// (the y flip included) every draw already goes through. `scissored` says
/// whether the clear's own scissor box applies: a `clear_quad` carries it, a
/// masked background clear has none (the GL core would have made it a
/// `clear_quad` otherwise), and a fused draw's scissor is the draw's.
///
/// Only the target and the clear's values are taken from `d`; everything
/// else starts from zero, because `d` may be an application's draw.
pub fn clear_quad_draw(
    d: &GlesDraw,
    b: QuadBuffers,
    scissored: bool,
    p: &ClearProg<'_>,
    vertices: &[[f32; 4]; 6],
    colour: &[f32; 4],
) -> GlesDraw {
    let mut q = GlesDraw::ZERO;
    (q.target_gpu, q.target_w, q.target_h) = (d.target_gpu, d.target_w, d.target_h);
    (q.target_format, q.target_alpha_bits) = (d.target_format, d.target_alpha_bits);
    (q.samples, q.num_targets, q.flip_y) = (d.samples, d.num_targets, d.flip_y);
    q.prim = GLES_PRIM_TRIANGLES;
    q.index_count = 6;
    q.fs = p.fs;
    q.fs_usc_gpu = p.code_gpu;
    q.attribs[0] = GlesAttrib {
        vi_reg: p.location,
        components: 4,
        format: GLES_ATTRIBF_FLOAT32,
        stride: 16,
        cpu: vertices.as_ptr() as *const core::ffi::c_void,
        cpu_bytes: core::mem::size_of_val(vertices) as u32,
        ..GlesAttrib::ZERO
    };
    q.num_attribs = 1;
    q.fs_uniform_cpu = colour.as_ptr() as *const core::ffi::c_void;
    q.fs_uniform_dwords = 4;
    q.viewport = [0, 0, d.target_w as i32, d.target_h as i32];
    q.depth_range = [0.0, 1.0];
    q.line_width = 1.0;
    q.cull = GLES_CULL_NONE;
    q.color_write = b.colour;
    q.color_mask = if b.colour { d.clear_color_mask & 0xf } else { 0 };
    q.depth_test = b.depth;
    q.depth_write = b.depth;
    q.depth_func = GLES_CMP_ALWAYS;
    q.stencil = b.stencil;
    if b.stencil {
        (q.stencil_func, q.stencil_back_func) = (GLES_CMP_ALWAYS, GLES_CMP_ALWAYS);
        (q.stencil_sfail, q.stencil_dpfail, q.stencil_dppass) = (GLES_SOP_REPLACE, GLES_SOP_REPLACE, GLES_SOP_REPLACE);
        (q.stencil_back_sfail, q.stencil_back_dpfail, q.stencil_back_dppass) =
            (GLES_SOP_REPLACE, GLES_SOP_REPLACE, GLES_SOP_REPLACE);
        (q.stencil_ref, q.stencil_back_ref) = (d.clear_stencil, d.clear_stencil);
        (q.stencil_cmp_mask, q.stencil_back_cmp_mask) = (0xff, 0xff);
        /* the glClear's mask, not the draw's: a clear fused into a draw
         * carries its own (GlesDraw::clear_stencil_write_mask, 0 = not
         * given), and Mesa's clear_with_quad writes both faces with the one
         * front write mask (st_cb_clear.c:261-268). Taking the draw's
         * (glStencilMask(0xfe) after the clear) kept bit 0 of the cleared
         * stencil: gl_test 9 check 9 on the Pi 4, 2026-09-24. */
        let mask = if d.clear_stencil_write_mask != 0 { d.clear_stencil_write_mask } else { d.stencil_write_mask };
        (q.stencil_write_mask, q.stencil_back_write_mask) = (mask, mask);
    }
    if scissored {
        (q.scissor, q.scissor_rect) = (d.scissor, d.scissor_rect);
    }
    q
}

/// `GlesPrim` to the V3D's primitive code. Points map too (the job builder
/// refuses them again); `None` for a value that is no primitive.
pub fn prim(p: GlesPrim) -> Option<u32> {
    Some(match p {
        GLES_PRIM_TRIANGLES => V3D_PRIM_TRIANGLES,
        GLES_PRIM_TRI_STRIP => V3D_PRIM_TRIANGLE_STRIP,
        GLES_PRIM_TRI_FAN => V3D_PRIM_TRIANGLE_FAN,
        GLES_PRIM_LINES => V3D_PRIM_LINES,
        GLES_PRIM_LINE_STRIP => V3D_PRIM_LINE_STRIP,
        GLES_PRIM_LINE_LOOP => V3D_PRIM_LINE_LOOP,
        GLES_PRIM_POINTS => V3D_PRIM_POINTS,
        _ => return None,
    })
}

/// `_mesa_get_viewport_xform` for `glViewport` and `glDepthRangef`
/// (main/viewport.c:538-556, GL_LOWER_LEFT, GL_NEGATIVE_ONE_TO_ONE), and
/// `st_update_viewport`'s flip for a window whose first row in memory is the
/// top of the picture (st_atom_viewport.c:55-70). The same arithmetic as
/// `v3d_viewport_gl()` in `driver/src/v3d/v3d_frame.c`, which drew M5 on the
/// board; the test below holds this to that function's numbers.
pub fn viewport(d: &GlesDraw) -> V3dViewport {
    let [x, y, w, h] = d.viewport;
    let (n, f) = (d.depth_range[0] as f64, d.depth_range[1] as f64);
    let (half_w, half_h) = (0.5 * w as f32, 0.5 * h as f32);
    let mut vp = V3dViewport {
        scale: [half_w, half_h, (0.5 * (f - n)) as f32],
        translate: [half_w + x as f32, half_h + y as f32, (0.5 * (n + f)) as f32],
    };
    if d.flip_y {
        vp.scale[1] = -vp.scale[1];
        vp.translate[1] = d.target_h as f32 - vp.translate[1];
    }
    vp
}

/// gl_FragCoord.y's transform: `STATE_FB_WPOS_Y_TRANSFORM`'s XY pair
/// (prog_statevars.c:736-751) with `flip_y` for the draw buffer's `FlipY`,
/// the same flip and height the viewport takes, so the two cannot disagree.
/// The same numbers as `v3d_wpos_gl()` in `driver/src/v3d/v3d_frame.c`; the
/// uniform writer refuses any other pair for the job's target.
pub fn wpos(d: &GlesDraw) -> V3dWpos {
    if d.flip_y {
        V3dWpos { y_scale: -1.0, y_offset: d.target_h as f32 }
    } else {
        V3dWpos { y_scale: 1.0, y_offset: 0.0 }
    }
}

/// `st_update_scissor` (st_atom_scissor.c:55-95): the box clipped to the
/// framebuffer, an empty box as all zero, and flipped for a window. `None`
/// when the scissor test is off.
pub fn scissor(d: &GlesDraw) -> Option<[u32; 4]> {
    if !d.scissor {
        return None;
    }
    let (fw, fh) = (d.target_w as i64, d.target_h as i64);
    let [x, y, w, h] = d.scissor_rect.map(|v| v as i64);
    let (mut minx, mut miny, mut maxx, mut maxy) = (0i64, 0i64, fw, fh);
    let xmax = (x + w).max(0);
    let ymax = (y + h).max(0);
    if x > minx {
        minx = x;
    }
    if y > miny {
        miny = y;
    }
    if xmax < maxx {
        maxx = xmax;
    }
    if ymax < maxy {
        maxy = ymax;
    }
    if minx >= maxx || miny >= maxy {
        (minx, miny, maxx, maxy) = (0, 0, 0, 0);
    }
    if d.flip_y {
        (miny, maxy) = (fh - maxy, fh - miny);
    }
    /* every value is now within 0..=the target size, which is a u32 */
    Some([minx as u32, miny as u32, maxx as u32, maxy as u32])
}

/// `GlesStencilOp` (the ISP's order) to the V3D's "Stencil Op", which puts
/// ZERO first and KEEP second.
pub fn stencil_op(op: GlesStencilOp) -> Option<u32> {
    Some(match op {
        GLES_SOP_KEEP => V3D_STENCIL_OP_KEEP,
        GLES_SOP_ZERO => V3D_STENCIL_OP_ZERO,
        GLES_SOP_REPLACE => V3D_STENCIL_OP_REPLACE,
        GLES_SOP_INCR_SAT => V3D_STENCIL_OP_INCR,
        GLES_SOP_DECR_SAT => V3D_STENCIL_OP_DECR,
        GLES_SOP_INVERT => V3D_STENCIL_OP_INVERT,
        GLES_SOP_INCR_WRAP => V3D_STENCIL_OP_INCWRAP,
        GLES_SOP_DECR_WRAP => V3D_STENCIL_OP_DECWRAP,
        _ => return None,
    })
}

/// `v3d_factor()` (v3dx_emit.c) for the `shc_blend_factor` codes, which are
/// gallium's `pipe_blendfactor`. `dst_alpha_one`: the target has no alpha, so
/// its alpha reads 1.
pub fn blend_factor(f: u32, dst_alpha_one: bool) -> Option<u32> {
    Some(match f {
        SHC_BF_ZERO => V3D_BLEND_FACTOR_ZERO,
        SHC_BF_ONE => V3D_BLEND_FACTOR_ONE,
        SHC_BF_SRC_COLOR => V3D_BLEND_FACTOR_SRC_COLOR,
        SHC_BF_INV_SRC_COLOR => V3D_BLEND_FACTOR_INV_SRC_COLOR,
        SHC_BF_DST_COLOR => V3D_BLEND_FACTOR_DST_COLOR,
        SHC_BF_INV_DST_COLOR => V3D_BLEND_FACTOR_INV_DST_COLOR,
        SHC_BF_SRC_ALPHA => V3D_BLEND_FACTOR_SRC_ALPHA,
        SHC_BF_INV_SRC_ALPHA => V3D_BLEND_FACTOR_INV_SRC_ALPHA,
        SHC_BF_DST_ALPHA if dst_alpha_one => V3D_BLEND_FACTOR_ONE,
        SHC_BF_DST_ALPHA => V3D_BLEND_FACTOR_DST_ALPHA,
        SHC_BF_INV_DST_ALPHA if dst_alpha_one => V3D_BLEND_FACTOR_ZERO,
        SHC_BF_INV_DST_ALPHA => V3D_BLEND_FACTOR_INV_DST_ALPHA,
        SHC_BF_CONST_COLOR => V3D_BLEND_FACTOR_CONST_COLOR,
        SHC_BF_INV_CONST_COLOR => V3D_BLEND_FACTOR_INV_CONST_COLOR,
        SHC_BF_CONST_ALPHA => V3D_BLEND_FACTOR_CONST_ALPHA,
        SHC_BF_INV_CONST_ALPHA => V3D_BLEND_FACTOR_INV_CONST_ALPHA,
        SHC_BF_SRC_ALPHA_SATURATE if dst_alpha_one => V3D_BLEND_FACTOR_ZERO,
        SHC_BF_SRC_ALPHA_SATURATE => V3D_BLEND_FACTOR_SRC_ALPHA_SATURATE,
        _ => return None,
    })
}

/// A float as an IEEE half, rounded to nearest even: what Mesa's
/// `_mesa_float_to_half` gives for `v3d_set_blend_color`'s `hf[]`
/// (util/half_float.c), including subnormal results, overflow to infinity
/// and NaN.
pub fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let man = bits & 0x7f_ffff;
    if exp == 0xff {
        /* infinity, or a NaN kept a (quiet) NaN */
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    /* the value is (1.man) * 2^(exp - 127); a half has 10 mantissa bits and
     * exponents -14..15 (normal), down to 2^-24 (subnormal) */
    let e = exp - 127;
    if e > 15 {
        return sign | 0x7c00;
    }
    /* the significand with its implicit bit, 24 bits */
    let sig = if exp == 0 { man } else { man | 0x80_0000 };
    /* how far to shift it right to land at the half's precision: 13 for a
     * normal half, more for a subnormal one */
    let shift = if e >= -14 { 13 } else { 13 + (-14 - e) } as u32;
    if shift > 24 {
        return sign; /* below half of the smallest subnormal */
    }
    let kept = sig >> shift;
    let rest = sig & ((1u32 << shift) - 1);
    let half = 1u32 << (shift - 1);
    let rounded = kept + u32::from(rest > half || (rest == half && kept & 1 == 1));
    if e >= -14 {
        /* normal: the exponent field, then the 10 mantissa bits; a round
         * that carries out of the mantissa bumps the exponent, which is
         * exactly the next representable value (up to infinity) */
        let field = (((e + 15) as u32) << 10) + (rounded - 0x400);
        sign | field.min(0x7c00) as u16
    } else {
        /* subnormal: the mantissa field alone (a carry to 0x400 is the
         * smallest normal, which the same bits encode) */
        sign | rounded as u16
    }
}

/// `util_pack_color` for R8G8B8A8_UNORM (`_mesa_float_to_unorm(f, 8)`,
/// which rounds half to even), R in bits 0..7: the clear colour
/// `v3d_tlb_clear` writes.
pub fn pack_rgba8(c: [f32; 4]) -> u32 {
    let mut out = 0u32;
    for (i, v) in c.iter().enumerate() {
        let x = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) } * 255.0;
        /* round half to even, as Mesa's lroundevenf does; x is in 0..=255,
         * where truncation is the floor (core has no f32::floor: no libm) */
        let fl = (x as u32) as f32;
        let frac = x - fl;
        let r = if frac > 0.5 || (frac == 0.5 && (fl as u32) & 1 == 1) { fl + 1.0 } else { fl };
        out |= (r as u32 & 0xff) << (8 * i);
    }
    out
}

/// The whole state of one draw, or why it cannot be expressed.
pub fn job_state(d: &GlesDraw) -> Result<V3dJobState, Refusal> {
    /* sample_mask: glSampleCoverage as the GL core made it (0xf on a
     * single-sampled target; the job builder writes it only in a 4x job) */
    let mut st = V3dJobState {
        vp: viewport(d),
        wpos: wpos(d),
        line_width: d.line_width,
        sample_mask: u32::from(d.sample_mask),
        ..V3dJobState::default()
    };

    if let Some([minx, miny, maxx, maxy]) = scissor(d) {
        st.scissor = true;
        (st.scissor_minx, st.scissor_miny, st.scissor_maxx, st.scissor_maxy) = (minx, miny, maxx, maxy);
    }

    /* st_update_rasterizer (st_atom_rasterizer.c:77-92): glFrontFace ==
     * GL_CCW, inverted for an FBO (Y_0_BOTTOM, not flip_y) and not for a
     * window, whose viewport carries the flip instead. !front_cw ^ !flip_y */
    st.front_ccw = d.front_cw ^ d.flip_y;
    /* which face the culled winding is: the GL core names the winding it
     * culls (draw.rs map_cull); the front face's winding is glFrontFace's */
    match d.cull {
        GLES_CULL_NONE => {}
        GLES_CULL_CW | GLES_CULL_CCW => {
            let culls_cw = d.cull == GLES_CULL_CW;
            if culls_cw == d.front_cw {
                st.cull_front = true;
            } else {
                st.cull_back = true;
            }
        }
        _ => return Err("an unknown cull mode"),
    }
    /* offset_units, offset_scale and offset_clamp are GL's own
     * (st_atom_rasterizer.c:161-163) */
    if d.polygon_offset {
        st.offset_tri = true;
        st.offset_units = d.polygon_offset_units;
        st.offset_scale = d.polygon_offset_factor;
        st.offset_clamp = 0.0;
    }

    /* st_update_depth_stencil_alpha: GLES_CMP_* is the V3D's compare order
     * (pinned in ffi_layout_check.c) */
    if d.depth_func > V3D_COMPARE_FUNC_ALWAYS
        || d.stencil_func > V3D_COMPARE_FUNC_ALWAYS
        || d.stencil_back_func > V3D_COMPARE_FUNC_ALWAYS
    {
        return Err("an unknown compare function");
    }
    st.depth_func = V3D_COMPARE_FUNC_ALWAYS;
    if d.depth_test {
        st.depth_enabled = true;
        st.depth_writemask = d.depth_write;
        st.depth_func = d.depth_func;
    }
    if d.stencil {
        let face = |func, sfail, zfail, zpass, ref_: u8, mask: u8, wmask: u8| -> Result<V3dJobStencilFace, Refusal> {
            Ok(V3dJobStencilFace {
                func,
                sfail_op: stencil_op(sfail).ok_or("an unknown stencil op")?,
                zfail_op: stencil_op(zfail).ok_or("an unknown stencil op")?,
                zpass_op: stencil_op(zpass).ok_or("an unknown stencil op")?,
                ref_: ref_ as u32,
                value_mask: mask as u32,
                write_mask: wmask as u32,
            })
        };
        st.stencil_enabled = true;
        st.stencil_front = face(
            d.stencil_func,
            d.stencil_sfail,
            d.stencil_dpfail,
            d.stencil_dppass,
            d.stencil_ref,
            d.stencil_cmp_mask,
            d.stencil_write_mask,
        )?;
        let back = face(
            d.stencil_back_func,
            d.stencil_back_sfail,
            d.stencil_back_dpfail,
            d.stencil_back_dppass,
            d.stencil_back_ref,
            d.stencil_back_cmp_mask,
            d.stencil_back_write_mask,
        )?;
        /* st/mesa enables the back state only when the two faces differ
         * (ctx->Stencil._TestTwoSide); v3d then emits one STENCIL_CFG that
         * configures both */
        if back != st.stencil_front {
            st.stencil_back_enabled = true;
            st.stencil_back = back;
        }
    }

    if d.blend {
        let dst_alpha_one = d.target_alpha_bits == 0;
        let f = |x: u8| blend_factor(x as u32, dst_alpha_one).ok_or("an unknown blend factor");
        if d.blend_eq_rgb as u32 > SHC_BLEND_MAX || d.blend_eq_alpha as u32 > SHC_BLEND_MAX {
            return Err("an unknown blend equation");
        }
        st.blend_enabled = true;
        /* shc_blend_func is pipe_blend_func, the V3D's "Blend Mode" order
         * for the five of them (pinned in ffi_layout_check.c) */
        st.rgb_func = d.blend_eq_rgb as u32;
        st.rgb_src = f(d.blend_src_rgb)?;
        st.rgb_dst = f(d.blend_dst_rgb)?;
        st.alpha_func = d.blend_eq_alpha as u32;
        st.alpha_src = f(d.blend_src_alpha)?;
        st.alpha_dst = f(d.blend_dst_alpha)?;
    }
    st.colormask = if d.color_write { (d.color_mask & 0xf) as u32 } else { 0 };
    /* A format with no alpha (GL_RGB, GL_RGB565) is stored as RGBA8 and reads
     * alpha 1 (ES 2.0 4.3.1); the GL core samples and reads that storage
     * as RGBA8, so the stored alpha must BE 1. The job loads it as 1
     * (V3dJobClear::force_alpha_1) and no draw writes it. Mesa's v3d need
     * not mask it (its X8 formats read through a SWIZ_XYZ1 view). A format
     * with 1 or 4 alpha bits (RGB5_A1, RGBA4) is rendered as RGBA8, as the
     * PowerVR does; the GL core quantises its clear. */
    if d.target_alpha_bits == 0 {
        st.colormask &= 0x7;
    }
    st.blend_color_f16 = d.blend_color.map(f32_to_f16);
    Ok(st)
}

/// What a draw does with the depth and the stencil the tile buffer holds:
/// (reads depth, reads stencil, writes depth, writes stencil).
pub fn zs_use(d: &GlesDraw) -> (bool, bool, bool, bool) {
    let reads_z = d.depth_test;
    let reads_s = d.stencil;
    let writes_z = d.depth_test && d.depth_write;
    let writes_s = d.stencil && (d.stencil_write_mask != 0 || d.stencil_back_write_mask != 0);
    (reads_z, reads_s, writes_z, writes_s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw() -> GlesDraw {
        GlesDraw {
            target_gpu: 0x0100_0000,
            target_w: 64,
            target_h: 64,
            target_format: GLES_SURF_RGBA8888,
            target_alpha_bits: 8,
            samples: 1,
            prim: GLES_PRIM_TRIANGLES,
            index_count: 3,
            line_width: 1.0,
            viewport: [0, 0, 64, 64],
            depth_range: [0.0, 1.0],
            flip_y: true,
            color_mask: 0xf,
            color_write: true,
            depth_func: GLES_CMP_LESS,
            ..GlesDraw::ZERO
        }
    }

    /// The viewport is `v3d_viewport_gl()`'s: M5 drew with (0, 0, 1280, 360),
    /// depth (0, 1), y0_top, on a 360-row target. These are the numbers that
    /// function's arithmetic gives (v3d_frame.c: half sizes, the y negated
    /// and measured from the bottom row).
    #[test]
    fn the_viewport_is_the_m5_frames() {
        let mut d = draw();
        (d.target_w, d.target_h, d.viewport) = (1280, 360, [0, 0, 1280, 360]);
        let vp = viewport(&d);
        assert_eq!(vp.scale, [640.0, -180.0, 0.5]);
        assert_eq!(vp.translate, [640.0, 180.0, 0.5]);
        /* an FBO (no flip) keeps GL's orientation */
        d.flip_y = false;
        let vp = viewport(&d);
        assert_eq!((vp.scale[1], vp.translate[1]), (180.0, 180.0));
        /* glDepthRangef(0.25, 0.75) and an offset viewport */
        (d.depth_range, d.viewport) = ([0.25, 0.75], [10, 20, 100, 50]);
        let vp = viewport(&d);
        assert_eq!(vp.scale, [50.0, 25.0, 0.25]);
        assert_eq!(vp.translate, [60.0, 45.0, 0.5]);
    }

    /// gl_FragCoord.y's transform: a window (flip_y, first row in memory at
    /// the top) gets (-1, the target's height), an FBO (1, 0); the job state
    /// carries the same pair. The height is the target's, not the viewport's.
    #[test]
    fn a_flipped_target_gets_minus_one_and_its_height_and_an_fbo_gets_one_and_zero() {
        let mut d = draw();
        (d.target_w, d.target_h, d.viewport) = (1280, 360, [10, 20, 100, 50]);
        assert_eq!(wpos(&d), V3dWpos { y_scale: -1.0, y_offset: 360.0 });
        assert_eq!(job_state(&d).unwrap().wpos, V3dWpos { y_scale: -1.0, y_offset: 360.0 });
        d.flip_y = false;
        assert_eq!(wpos(&d), V3dWpos { y_scale: 1.0, y_offset: 0.0 });
        assert_eq!(job_state(&d).unwrap().wpos, V3dWpos { y_scale: 1.0, y_offset: 0.0 });
        /* on a pixel centre: memory row 0 of a window is the top one, where
         * GL's y is height - 0.5 */
        d.flip_y = true;
        let w = wpos(&d);
        assert_eq!(0.5 * w.y_scale + w.y_offset, 359.5);
    }

    #[test]
    fn the_scissor_is_clipped_emptied_and_flipped_as_st_mesa_does() {
        let mut d = draw();
        assert_eq!(scissor(&d), None, "off");
        d.scissor = true;
        (d.flip_y, d.scissor_rect) = (false, [8, 4, 16, 20]);
        assert_eq!(scissor(&d), Some([8, 4, 24, 24]));
        /* a window: rows counted from the top */
        d.flip_y = true;
        assert_eq!(scissor(&d), Some([8, 40, 24, 60]));
        /* clipped to the target; a negative origin */
        d.scissor_rect = [-10, -10, 100, 30];
        assert_eq!(scissor(&d), Some([0, 44, 64, 64]));
        /* empty: all zero; st/mesa flips the empty box too */
        (d.flip_y, d.scissor_rect) = (false, [16, 16, 0, 32]);
        assert_eq!(scissor(&d), Some([0, 0, 0, 0]));
        d.flip_y = true;
        assert_eq!(scissor(&d), Some([0, 64, 0, 64]));
    }

    /// glFrontFace and the flip compose as st/mesa composes them
    /// (st_atom_rasterizer.c:77-92): front_ccw is glFrontFace == GL_CCW,
    /// inverted for an FBO (Y_0_BOTTOM: "Drawing to an FBO. The viewport will
    /// be inverted.") and NOT for a window (Y_0_TOP, whose viewport is the
    /// flipped one, st_atom_viewport.c:65-68). The table had the window and
    /// the FBO the other way round until gl_test 12 on the Pi 4 showed both
    /// faces swapped on both (2026-09-24). The culled winding names a face.
    #[test]
    fn front_face_and_culling_follow_st_mesa() {
        let mut d = draw();
        /* (glFrontFace is CW, a window (flip_y), gallium front_ccw) */
        for (front_cw, flip, ccw) in
            [(false, true, true), (true, true, false), (false, false, false), (true, false, true)]
        {
            (d.front_cw, d.flip_y) = (front_cw, flip);
            assert_eq!(job_state(&d).unwrap().front_ccw, ccw, "front_cw {front_cw} flip {flip}");
        }
        d.flip_y = false;
        /* glCullFace(GL_BACK) with a CCW front: the GL core culls CW */
        (d.front_cw, d.cull) = (false, GLES_CULL_CW);
        let st = job_state(&d).unwrap();
        assert!(st.cull_back && !st.cull_front);
        /* glCullFace(GL_FRONT), CCW front: culls CCW, the front */
        d.cull = GLES_CULL_CCW;
        let st = job_state(&d).unwrap();
        assert!(st.cull_front && !st.cull_back);
        /* a CW front, culling CW: the front */
        (d.front_cw, d.cull) = (true, GLES_CULL_CW);
        let st = job_state(&d).unwrap();
        assert!(st.cull_front && !st.cull_back);
        d.cull = GLES_CULL_NONE;
        let st = job_state(&d).unwrap();
        assert!(!st.cull_front && !st.cull_back);
    }

    #[test]
    fn every_stencil_op_maps_and_keep_is_not_zero() {
        let want = [
            (GLES_SOP_KEEP, V3D_STENCIL_OP_KEEP),
            (GLES_SOP_ZERO, V3D_STENCIL_OP_ZERO),
            (GLES_SOP_REPLACE, V3D_STENCIL_OP_REPLACE),
            (GLES_SOP_INCR_SAT, V3D_STENCIL_OP_INCR),
            (GLES_SOP_DECR_SAT, V3D_STENCIL_OP_DECR),
            (GLES_SOP_INVERT, V3D_STENCIL_OP_INVERT),
            (GLES_SOP_INCR_WRAP, V3D_STENCIL_OP_INCWRAP),
            (GLES_SOP_DECR_WRAP, V3D_STENCIL_OP_DECWRAP),
        ];
        for (gl, v3d) in want {
            assert_eq!(stencil_op(gl), Some(v3d), "op {gl}");
        }
        assert_eq!(stencil_op(8), None);
        assert_ne!(V3D_STENCIL_OP_KEEP, GLES_SOP_KEEP, "the two orders differ: the map is not the identity");
    }

    #[test]
    fn stencil_is_one_sided_until_the_faces_differ() {
        let mut d = draw();
        d.stencil = true;
        (d.stencil_func, d.stencil_back_func) = (GLES_CMP_EQUAL, GLES_CMP_EQUAL);
        (d.stencil_ref, d.stencil_back_ref) = (3, 3);
        (d.stencil_cmp_mask, d.stencil_back_cmp_mask) = (0xff, 0xff);
        (d.stencil_write_mask, d.stencil_back_write_mask) = (0xff, 0xff);
        d.stencil_dppass = GLES_SOP_REPLACE;
        d.stencil_back_dppass = GLES_SOP_REPLACE;
        let st = job_state(&d).unwrap();
        assert!(st.stencil_enabled && !st.stencil_back_enabled);
        assert_eq!(st.stencil_front.zpass_op, V3D_STENCIL_OP_REPLACE);
        assert_eq!(st.stencil_front.sfail_op, V3D_STENCIL_OP_KEEP, "GLES_SOP_KEEP is 0 and must not read as ZERO");
        assert_eq!(st.stencil_front.ref_, 3);
        assert_eq!(st.stencil_back, V3dJobStencilFace::default(), "no back state when it is the front's");
        d.stencil_back_ref = 4;
        let st = job_state(&d).unwrap();
        assert!(st.stencil_back_enabled);
        assert_eq!(st.stencil_back.ref_, 4);
    }

    #[test]
    fn blend_factors_are_v3d_factor() {
        let pairs = [
            (SHC_BF_ZERO, V3D_BLEND_FACTOR_ZERO),
            (SHC_BF_ONE, V3D_BLEND_FACTOR_ONE),
            (SHC_BF_SRC_COLOR, V3D_BLEND_FACTOR_SRC_COLOR),
            (SHC_BF_INV_SRC_COLOR, V3D_BLEND_FACTOR_INV_SRC_COLOR),
            (SHC_BF_DST_COLOR, V3D_BLEND_FACTOR_DST_COLOR),
            (SHC_BF_INV_DST_COLOR, V3D_BLEND_FACTOR_INV_DST_COLOR),
            (SHC_BF_SRC_ALPHA, V3D_BLEND_FACTOR_SRC_ALPHA),
            (SHC_BF_INV_SRC_ALPHA, V3D_BLEND_FACTOR_INV_SRC_ALPHA),
            (SHC_BF_DST_ALPHA, V3D_BLEND_FACTOR_DST_ALPHA),
            (SHC_BF_INV_DST_ALPHA, V3D_BLEND_FACTOR_INV_DST_ALPHA),
            (SHC_BF_CONST_COLOR, V3D_BLEND_FACTOR_CONST_COLOR),
            (SHC_BF_INV_CONST_COLOR, V3D_BLEND_FACTOR_INV_CONST_COLOR),
            (SHC_BF_CONST_ALPHA, V3D_BLEND_FACTOR_CONST_ALPHA),
            (SHC_BF_INV_CONST_ALPHA, V3D_BLEND_FACTOR_INV_CONST_ALPHA),
            (SHC_BF_SRC_ALPHA_SATURATE, V3D_BLEND_FACTOR_SRC_ALPHA_SATURATE),
        ];
        for (shc, v3d) in pairs {
            assert_eq!(blend_factor(shc, false), Some(v3d), "factor {shc:#x}");
        }
        /* no alpha in the target: its alpha reads 1 */
        assert_eq!(blend_factor(SHC_BF_DST_ALPHA, true), Some(V3D_BLEND_FACTOR_ONE));
        assert_eq!(blend_factor(SHC_BF_INV_DST_ALPHA, true), Some(V3D_BLEND_FACTOR_ZERO));
        assert_eq!(blend_factor(SHC_BF_SRC_ALPHA_SATURATE, true), Some(V3D_BLEND_FACTOR_ZERO));
        assert_eq!(blend_factor(0, false), None, "0 is no factor while blending is on");
        assert_eq!(blend_factor(0x16, false), None, "no ES 2.0 factor has this code");
    }

    #[test]
    fn halves_round_to_nearest_even() {
        let cases: [(f32, u16); 14] = [
            (0.0, 0x0000),
            (-0.0, 0x8000),
            (1.0, 0x3c00),
            (0.5, 0x3800),
            (-2.0, 0xc000),
            (65504.0, 0x7bff),
            (65520.0, 0x7c00), /* halfway to the next: to even, which is infinity */
            (65519.0, 0x7bff),
            (f32::INFINITY, 0x7c00),
            (6.103_515_6e-5, 0x0400),     /* the smallest normal */
            (5.960_464_5e-8, 0x0001),     /* the smallest subnormal */
            (2.980_232_2e-8, 0x0000),     /* half of it: to even, zero */
            (1.0 + 1.0 / 2048.0, 0x3c00), /* halfway between 1 and the next: even */
            (1.0 + 3.0 / 2048.0, 0x3c02), /* halfway, odd below: up */
        ];
        for (v, h) in cases {
            assert_eq!(f32_to_f16(v), h, "{v}");
        }
        assert_eq!(f32_to_f16(f32::NAN) & 0x7c00, 0x7c00);
        assert_ne!(f32_to_f16(f32::NAN) & 0x3ff, 0, "a NaN stays a NaN");
    }

    #[test]
    fn the_clear_colour_rounds_like_util_pack_color() {
        assert_eq!(pack_rgba8([1.0, 0.0, 0.0, 1.0]), 0xff00_00ff);
        assert_eq!(pack_rgba8([0.0, 1.0, 0.0, 0.5]), 0x8000_ff00, "0.5 * 255 = 127.5: to even, 128");
        assert_eq!(pack_rgba8([2.0, -1.0, f32::NAN, 0.25]), 0x4000_00ff, "clamped; 63.75 rounds to 64");
        assert_eq!(pack_rgba8([0.125, 0.125, 0.125, 1.0]), 0xff20_2020, "31.875 -> 32");
    }

    #[test]
    fn a_colour_mask_and_a_colour_write_off() {
        let mut d = draw();
        d.color_mask = 0x7;
        assert_eq!(job_state(&d).unwrap().colormask, 0x7);
        d.color_write = false;
        assert_eq!(job_state(&d).unwrap().colormask, 0);
    }

    #[test]
    fn depth_off_is_always_and_no_write() {
        let mut d = draw();
        d.depth_write = true;
        let st = job_state(&d).unwrap();
        assert!(!st.depth_enabled && !st.depth_writemask);
        assert_eq!(st.depth_func, V3D_COMPARE_FUNC_ALWAYS);
        d.depth_test = true;
        let st = job_state(&d).unwrap();
        assert!(st.depth_enabled && st.depth_writemask);
        assert_eq!(st.depth_func, GLES_CMP_LESS);
    }

    /// A colour format with fewer than 8 alpha bits is drawn (dEQP lifetime's
    /// RGBA4 renderbuffer, gl_test 67's RGB texture). With none (GL_RGB,
    /// GL_RGB565) no draw writes alpha, whatever the GL mask, so the stored
    /// alpha stays the 1 the job loads; with 1 or 4 the GL mask is kept.
    #[test]
    fn a_format_with_fewer_alpha_bits_is_drawn_and_one_with_none_keeps_alpha() {
        for (bits, mask, want) in
            [(8u8, 0xfu8, 0xfu32), (4, 0xf, 0xf), (1, 0xf, 0xf), (0, 0xf, 0x7), (0, 0b1001, 0b0001), (0, 0b1000, 0)]
        {
            let mut d = draw();
            d.target_alpha_bits = bits;
            d.color_mask = mask;
            assert_eq!(refusal(&d), None, "{bits} alpha bits");
            assert_eq!(job_state(&d).unwrap().colormask, want, "{bits} alpha bits, GL mask {mask:#b}");
        }
    }

    /// An XRGB8888 target (a window or EGLImage whose bytes are B,G,R,X) is
    /// drawn: the job swaps R and B at the TLB's load and store (backend.rs),
    /// and with no alpha (the GL core reports 0 bits) nothing writes alpha.
    #[test]
    fn an_xrgb_target_is_drawn_without_alpha_writes() {
        let mut d = draw();
        (d.target_format, d.target_alpha_bits) = (GLES_SURF_XRGB8888, 0);
        assert_eq!(refusal(&d), None);
        assert_eq!(job_state(&d).unwrap().colormask, 0x7);
    }

    /// One distinct reason per refusal: they are what the board run's log
    /// is bucketed on.
    #[test]
    fn every_refusal_has_its_own_reason() {
        /* each case: how to break the plain draw, and a word its reason names */
        type Case = (fn(&mut GlesDraw), &'static str);
        let cases: [Case; 6] = [
            (|d| d.target_format = GLES_SURF_RGBA16F, "RGBA16F"),
            (|d| d.target_format = GLES_SURF_R16F, "R16F"),
            (|d| d.target_format = GLES_SURF_R32F, "R32F"),
            (|d| d.target_format = 99, "does not know"),
            (|d| d.samples = 2, "sample count"),
            (|d| d.index_count2 = 3, "second index list"),
        ];
        let mut seen: std::vec::Vec<&str> = std::vec::Vec::new();
        assert_eq!(refusal(&draw()), None, "the plain draw is drawn");
        for (make, word) in cases {
            let mut d = draw();
            make(&mut d);
            let r = refusal(&d).expect("refused");
            assert!(r.contains(word), "{r}");
            assert!(!seen.contains(&r), "{r} twice");
            seen.push(r);
        }
        /* a clear is not refused for what only a draw needs */
        let mut d = draw();
        (d.index_count, d.prim, d.num_textures) = (0, GLES_PRIM_POINTS, 1);
        assert_eq!(refusal(&d), None);
    }

    /// Alpha-to-coverage is legal GL state the backend cannot render yet:
    /// it is refused by [`unsupported`] (the backend returns -ENOTSUP), not
    /// by [`refusal`] (-EINVAL, a malformed draw or a core bug). Its reason
    /// is its own string, apart from every [`refusal`] reason.
    /// glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}) (review R-09): the
    /// job always stores RT 0 to the target, so a draw whose slot 0 the GL
    /// core left without a target is refused; slot 0 written is not.
    #[test]
    fn a_draw_whose_draw_buffer_0_is_none_is_unsupported() {
        let mut d = GlesDraw::ZERO;
        d.target_gpu = 0x1000;
        d.num_targets = 2;
        d.targets[1] = GlesDrawTarget { gpu: 0x2000, w: 8, h: 8, format: GLES_SURF_RGBA8888 };
        assert_eq!(unsupported(&d), Some("draw buffer 0 is GL_NONE while a later draw buffer is written"));
        d.targets[0] = GlesDrawTarget { gpu: 0x1000, w: 8, h: 8, format: GLES_SURF_RGBA8888 };
        assert_eq!(unsupported(&d), None);
    }

    #[test]
    fn alpha_to_coverage_is_a_gap_not_a_malformed_draw() {
        assert_eq!(unsupported(&draw()), None, "the plain draw is drawn");
        let mut d = draw();
        (d.samples, d.alpha_to_coverage) = (4, true);
        assert_eq!(refusal(&d), None, "not a malformed draw");
        assert_eq!(unsupported(&d), Some("alpha-to-coverage is not implemented"));
        /* its reason is none of the refusal reasons */
        let mut other = draw();
        for f in [GLES_SURF_RGBA16F, GLES_SURF_R16F, GLES_SURF_R32F, 99] {
            other.target_format = f;
            assert_ne!(refusal(&other), unsupported(&d));
        }
    }

    /// The quad takes the target and the clear's values from the draw it
    /// comes with and nothing else: that draw may be an application's, with
    /// its own blend, cull, textures, arrays and scissor.
    #[test]
    fn the_clear_quad_takes_nothing_from_an_applications_draw() {
        let mut d = draw();
        d.blend = true;
        d.cull = GLES_CULL_CW;
        d.polygon_offset = true;
        d.num_textures = 3;
        d.num_attribs = 5;
        d.index_cpu = 8 as *const core::ffi::c_void;
        (d.scissor, d.scissor_rect) = (true, [1, 2, 3, 4]);
        (d.depth_test, d.depth_write, d.depth_func) = (true, true, GLES_CMP_LESS);
        d.viewport = [5, 6, 7, 8];
        d.clear_color_mask = 0b0110;
        let fs = ShcShader::ZERO;
        let p = ClearProg { fs: &fs, code_gpu: 0x4000, location: 2 };
        let (v, c) = (clear_quad_vertices(0.5), [0.1, 0.2, 0.3, 0.4]);
        let b = QuadBuffers { colour: true, depth: false, stencil: false };
        let q = clear_quad_draw(&d, b, false, &p, &v, &c);
        assert!(!q.blend && q.cull == GLES_CULL_NONE && !q.polygon_offset);
        assert_eq!((q.num_textures, q.num_attribs, q.attribs[0].vi_reg), (0, 1, 2));
        assert!(q.index_cpu.is_null() && q.index_buf_gpu == 0);
        assert!(!q.scissor, "a fused draw's scissor is not the clear's");
        assert!(!q.depth_test && !q.depth_write && !q.stencil);
        assert_eq!(q.viewport, [0, 0, 64, 64], "the whole target");
        assert_eq!((q.color_write, q.color_mask), (true, 0b0110));
        assert_eq!((q.fs, q.fs_usc_gpu), (&fs as *const ShcShader, 0x4000));
        assert_eq!((q.target_gpu, q.target_w, q.target_h, q.flip_y), (d.target_gpu, 64, 64, true));
        assert!(!q.do_clear && !q.clear_quad, "the quad is a draw, not a clear");
        let st = job_state(&q).expect("the quad's state translates");
        assert!(!st.blend_enabled && !st.scissor && !st.depth_enabled);
        /* scissored: the clear's own box */
        let q = clear_quad_draw(&d, b, true, &p, &v, &c);
        assert_eq!((q.scissor, q.scissor_rect), (true, [1, 2, 3, 4]));
    }

    /// A stencil clear drawn by the quad writes under the glClear's write
    /// mask, on both faces, not under the mask of the draw it is fused into:
    /// gl_test 9 frame 5 clears with 0xff, then draws with glStencilMask(0xfe),
    /// and the quad took 0xfe and kept bit 0 (the Pi 4, 2026-09-24). 0 means
    /// "not given" and falls back to the draw's (src/backend.h).
    #[test]
    fn a_fused_stencil_clear_uses_the_clears_write_mask() {
        let mut d = draw();
        (d.stencil, d.stencil_write_mask, d.stencil_back_write_mask) = (true, 0xfe, 0xfe);
        (d.do_clear_stencil, d.clear_stencil, d.clear_stencil_write_mask) = (true, 0, 0xff);
        let fs = ShcShader::ZERO;
        let p = ClearProg { fs: &fs, code_gpu: 0x4000, location: 2 };
        let (v, c) = (clear_quad_vertices(1.0), [0.0; 4]);
        let b = QuadBuffers { colour: false, depth: false, stencil: true };
        let q = clear_quad_draw(&d, b, false, &p, &v, &c);
        assert_eq!((q.stencil_write_mask, q.stencil_back_write_mask), (0xff, 0xff), "the clear's mask");
        assert_eq!((q.stencil_ref, q.stencil_dppass), (0, GLES_SOP_REPLACE));
        d.clear_stencil_write_mask = 0;
        let q = clear_quad_draw(&d, b, false, &p, &v, &c);
        assert_eq!((q.stencil_write_mask, q.stencil_back_write_mask), (0xfe, 0xfe), "0: not given");
    }

    /// z = 2 * depth - 1 (st_cb_clear.c:309), so the depth range (0, 1)
    /// gives the clear depth back.
    #[test]
    fn the_quad_sits_at_the_clear_depth() {
        for (depth, z) in [(0.0, -1.0), (0.5, 0.0), (1.0, 1.0), (0.25, -0.5)] {
            assert!(clear_quad_vertices(depth).iter().all(|v| v[2] == z && v[3] == 1.0), "{depth}");
        }
        let v = clear_quad_vertices(0.5);
        let (xs, ys): (std::vec::Vec<f32>, std::vec::Vec<f32>) = v.iter().map(|v| (v[0], v[1])).unzip();
        assert!(xs.iter().all(|x| x.abs() == 1.0) && ys.iter().all(|y| y.abs() == 1.0), "the clip-space square");
    }

    #[test]
    fn zs_use_separates_reading_from_writing() {
        let mut d = draw();
        assert_eq!(zs_use(&d), (false, false, false, false));
        d.depth_test = true;
        assert_eq!(zs_use(&d), (true, false, false, false));
        d.depth_write = true;
        assert_eq!(zs_use(&d), (true, false, true, false));
        d.stencil = true;
        assert_eq!(zs_use(&d), (true, true, true, false), "a zero write mask writes no stencil");
        d.stencil_back_write_mask = 1;
        assert_eq!(zs_use(&d), (true, true, true, true));
    }
}
