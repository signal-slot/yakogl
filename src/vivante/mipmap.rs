// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glGenerateMipmap on the GPU: the levels of a 2D RGBA8 chain drawn one
//! quad each, into the GL core's own storage.
//!
//! The GL core box-filters a chain on the CPU (gl/texture.rs
//! glGenerateMipmap) unless the backend makes it
//! (`YakoglBackend::generate_mipmap`). Here each level above 0 is one draw
//! into that level -- a linear RGBA8 render target at the level's address
//! and the chain's pitch, the pixel engine's own write -- holding one
//! full-screen quad whose fragment program
//! (tools/shaders/internal/es/mipgen.frag, our compiler's, row
//! `GC_INTERNAL_MIPGEN_PS` of `internal.rs`) samples the level above through
//! a one-level bilinear CLAMP_TO_EDGE sampler at the centre of each texel:
//! the 2x2 box average where the level is half the one above (every level
//! of a power-of-two chain), the bilinear sample between the nearest four on
//! a NPOT chain's odd steps (glmark2's 800x600 terrain: 75 -> 37, 25 -> 12;
//! ES 3.0.6 3.8.9 leaves the filter open). The PowerVR's path
//! (src/pvr/backend.rs `pvr_generate_mipmap`) is the model; the V3D's TFU
//! path was the first.
//!
//! The levels are written where the texture unit walks them from level 0's
//! address (`texture.rs` `build_descriptor`: every level at the chain's one
//! LINEAR_STRIDE, each starting on a 64-byte boundary, the layout the GL
//! core keeps for this backend through `mip_level_align`), so the GL core
//! asks nothing back but a wait ([`gc_return_mipmap_chain`]) before it
//! reads or writes them on the CPU. Each level's draw goes through
//! [`super::backend::draw_one`], so the frame still open (the one that
//! rendered level 0, or the level before) is submitted first and the level
//! kicks follow it in order; nothing is waited for.
//!
//! The channel order in memory does not matter to the filter and is kept
//! as it is: the pixel engine writes B,G,R,A (`rt_rgba8888_is_bgra`), and
//! the level above is always bound as B,G,R,A too (`bgra` 1, the
//! A8R8G8B8 descriptor). A texture a draw rendered into holds B,G,R,A
//! texels (`pe_order`) and is sampled right; an uploaded R,G,B,A texture is
//! sampled with red and blue swapped and written back swapped again, which
//! puts every byte where it was read from, since the filter is the same on
//! every channel. One fragment program serves both.
//!
//! Why: glmark2's terrain (800x600, ten levels, five chains a frame) and
//! refract (1600x1200, eleven levels) ask for their chains every frame, and
//! the CPU filter was 8.3 of terrain's 9.0 ms and 10.3 of refract's 12.4 ms
//! of GL-call time a frame on the host, more on the board, whose carve-out
//! is non-cacheable (docs/notes/imx8mp-glmark2-cpu.md, fix item 1).

use core::ffi::c_void;

use crate::backend::{
    GlesAttrib, GlesDraw, GlesDrawTarget, GlesTextureBind, GLES_ATTRIBF_FLOAT32, GLES_CMP_ALWAYS, GLES_CULL_NONE,
    GLES_PRIM_TRI_STRIP, GLES_SURF_RGBA8888, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, GLES_WRAP_CLAMP_TO_EDGE,
};
use crate::gl::context::CAP_MIP_LEVELS;
use crate::util::Global;

use super::backend::{self, g};
use super::internal::{GC_INTERNAL_MIPGEN_PS, GC_INTERNAL_TABLE};
use super::regs::GC_TEXDESC_LOD_ALIGN;

/// Bytes per RGBA8 texel.
const RGBA8_BYTES: u32 = 4;

/// One level of a chain as the texture unit walks it from level 0's address
/// (`texture.rs` `build_descriptor`): `off` bytes from level 0, on a
/// [`GC_TEXDESC_LOD_ALIGN`] boundary, the GL extent `w` x `h` in rows of
/// the chain's pitch.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct MipLevel {
    pub off: u32,
    pub w: u32,
    pub h: u32,
}

/// The first `levels` levels of the chain of a `w` x `h` RGBA8 2D texture
/// whose rows are `stride` texels apart, as the texture unit walks them and
/// as the GL core lays them out for this backend (gl/texture.rs
/// `chain_layout_npot` with `mip_level_align` 64: level l is max(w >> l, 1)
/// x max(h >> l, 1), `stride` x its height x 4 bytes, starting on a 64-byte
/// boundary after the level before). Returns how many levels were written,
/// and the first byte past the last one. `None` for a size of 0, fewer than
/// two levels, more than `out` holds, a stride narrower than the image, or
/// offsets past u32.
pub(super) fn chain_layout(w: u32, h: u32, stride: u32, levels: u32, out: &mut [MipLevel]) -> Option<(usize, u32)> {
    if w == 0 || h == 0 || stride < w || levels < 2 || levels as usize > out.len() {
        return None;
    }
    let align = GC_TEXDESC_LOD_ALIGN as u32;
    let (mut lw, mut lh) = (w, h);
    let mut off: u32 = 0;
    for l in 0..levels as usize {
        off = off.checked_add(align - 1)? & !(align - 1);
        out[l] = MipLevel { off, w: lw, h: lh };
        off = off.checked_add(stride.checked_mul(lh)?.checked_mul(RGBA8_BYTES)?)?;
        lw = if lw > 1 { lw / 2 } else { 1 };
        lh = if lh > 1 { lh / 2 } else { 1 };
    }
    Some((levels as usize, off))
}

/// The quad's four corners in clip space (x, y, z, w), a triangle strip
/// over the whole target, as the clear quad's ([`backend`] `emit_clear`).
static QUAD: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];

/// [`record_level`]'s draw: all-zero is a valid `GlesDraw` (integers,
/// floats, bools, null pointers). A static rather than a stack frame, as
/// the PowerVR keeps its (`MIP_DRAW`): [`backend::draw_one`] mutates the
/// backend while it reads the draw, and `GlesDraw` is large.
static MIP_DRAW: Global<GlesDraw> = Global::zeroed();

/// The fragment program's uniform block, `u_scale` = 1 / the level's size:
/// read by `draw_one` through `fs_uniform_cpu` while the draw records.
static MIP_SCALE: Global<[f32; 2]> = Global::zeroed();

/// The GPU mipmap quad's fragment program (internal/es/mipgen.frag): one
/// texture, the two-float block `u_scale`, the colour from gl_FragCoord.
pub(super) fn mipgen_ps() -> &'static crate::shaderc::ShcShader {
    &GC_INTERNAL_TABLE[GC_INTERNAL_MIPGEN_PS].shader
}

/// The draw of level `dst` of the chain at `base` from level `src`, both
/// `stride` texels a row: a linear RGBA8888 target at the level's address
/// (the pixel engine's B,G,R,A write, at the chain's pitch), one quad over
/// the level's extent, no depth or stencil, no blend, every channel, the
/// quad vertex program and [`mipgen_ps`] with `src` bound as a one-level
/// linear-filtered CLAMP_TO_EDGE B,G,R,A texture and 1 / the level's size
/// as the block. Through [`backend::draw_one`]: the frame still open is
/// submitted first, and the level's frame opens on its own target. 0, or
/// the draw's error.
fn record_level(base: u64, stride: u32, src: &MipLevel, dst: &MipLevel) -> i32 {
    let g = g();
    // SAFETY: single-threaded by contract; `d` and the block are read by
    // draw_one alone until it returns, and nothing else refers to the
    // statics meanwhile.
    let (d, scale) = unsafe { (MIP_DRAW.get(), MIP_SCALE.get()) };
    *scale = [1.0 / dst.w as f32, 1.0 / dst.h as f32];
    let vs = backend::clear_vs();

    *d = GlesDraw::ZERO;
    d.target_gpu = base.wrapping_add(u64::from(dst.off));
    d.target_w = dst.w;
    d.target_h = dst.h;
    d.target_format = GLES_SURF_RGBA8888;
    d.target_alpha_bits = 8; /* the sampled alpha, as the CPU filter averages it */
    /* the chain's one pitch, wider than the level (`draw_pitch`) */
    d.target_pitch = stride;
    d.targets[0] = GlesDrawTarget { gpu: d.target_gpu, w: dst.w, h: dst.h, format: GLES_SURF_RGBA8888, out_shift: 0 };
    d.num_targets = 1;
    d.samples = 1;
    d.prim = GLES_PRIM_TRI_STRIP;
    d.index_count = 4;
    d.instance_count = 1;
    d.vs = vs;
    d.fs = mipgen_ps();
    d.vs_usc_gpu = g.clear_vs_code.gpu;
    d.fs_usc_gpu = g.mipgen_ps_code.gpu;
    /* the quad as a client array: `draw_one` copies it into the frame arena
     * with the draw, as it does the GL core's staged arrays */
    d.attribs[0] = GlesAttrib {
        vi_reg: vs.viv.in_[0].reg,
        components: 4,
        format: GLES_ATTRIBF_FLOAT32,
        normalized: 0,
        stride: 16,
        offset: 0,
        buffer_gpu: 0,
        cpu: QUAD.as_ptr() as *const c_void,
        cpu_bytes: core::mem::size_of_val(&QUAD) as u32,
        per_instance: 0,
        divisor: 0,
        buffer_bytes: 0,
    };
    d.num_attribs = 1;
    d.fs_uniform_cpu = scale.as_ptr() as *const c_void;
    d.fs_uniform_dwords = 2;
    d.textures[0] = GlesTextureBind {
        unit: 0,
        format: GLES_TEXFMT_RGBA8,
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: base.wrapping_add(u64::from(src.off)),
        width: src.w,
        height: src.h,
        stride_texels: stride,
        bytes: stride.wrapping_mul(src.h).wrapping_mul(RGBA8_BYTES),
        linear_filter: 1,
        min_linear: 1,
        mip_filter: 0,
        mip_levels: 1,
        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
        /* B,G,R,A, whatever the storage holds: see the module's note */
        bgra: 1,
        ..GlesTextureBind::ZERO
    };
    d.num_textures = 1;
    d.viewport = [0, 0, dst.w as i32, dst.h as i32];
    d.depth_range = [0.0, 1.0];
    d.depth_func = GLES_CMP_ALWAYS;
    d.cull = GLES_CULL_NONE;
    d.color_mask = 0xf;
    d.color_write = true;
    d.line_width = 1.0;
    // SAFETY: `d` is a live draw for the length of the call; its client
    // array and uniform block are statics.
    unsafe { backend::draw_one(d) }
}

/// Chains made lately: the storage's base and the kick that drew its last
/// level, for [`chain_kick_pending`]. A ring of the newest; a texture whose
/// entry was pushed out has had this many chains made after it, so its
/// kicks are long complete.
const RECENT_CHAINS: usize = 8;
static RECENT: Global<[(u64, u32); RECENT_CHAINS]> = Global::zeroed();

fn note_chain(base: u64, kick: u32) {
    // SAFETY: single-threaded by contract; no caller holds RECENT.
    let r = unsafe { RECENT.get() };
    if let Some(e) = r.iter_mut().find(|e| e.0 == base) {
        e.1 = kick;
        return;
    }
    r.rotate_right(1);
    r[0] = (base, kick);
}

/// Is a kick that draws a level of the chain at `base` still to complete?
/// The GL core's stamps do not know the chain's kicks (nothing it stamps
/// moved), so a CPU reader of the storage inside this backend (the tiled
/// copies, `tiled_copy.rs` `refresh`) asks here before it reads.
pub(super) fn chain_kick_pending(base: u64, kick_pending: &impl Fn(u32) -> bool) -> bool {
    // SAFETY: as above.
    let r = unsafe { RECENT.get() };
    r.iter().any(|e| e.0 == base && e.1 != 0 && kick_pending(e.1))
}

/// Forget every chain ([`backend::gc_shutdown`]).
pub(super) fn reset() {
    // SAFETY: as above.
    unsafe { *RECENT.get() = [(0, 0); RECENT_CHAINS] };
}

/// [`gc_generate_mipmap`]: the chain, or why it is left to the GL core.
fn chain_gpu(t: &GlesTextureBind, levels: u32) -> Result<(), &'static str> {
    if t.format != GLES_TEXFMT_RGBA8 {
        return Err("not an RGBA8 texture");
    }
    if t.type_ != GLES_TEXTYPE_2D || t.layers > 1 {
        return Err("not a 2D texture");
    }
    if g().mipgen_ps_code.impl_.is_null() {
        return Err("no mipmap program");
    }
    let mut lv = [MipLevel::default(); CAP_MIP_LEVELS];
    let (n, end) = chain_layout(t.width, t.height, t.stride_texels, levels, &mut lv)
        .ok_or("a chain layout this backend does not walk")?;
    /* the MMU addresses 32 bits: every level inside them */
    match t.base_gpu.checked_add(u64::from(end)) {
        Some(e) if e <= u64::from(u32::MAX) + 1 => {}
        _ => return Err("an image address past the 32-bit address space"),
    }
    /* the copies the texture unit samples instead of this storage (a
     * texture the vertex stage reads) hold the old levels: none of them is
     * current once the kicks below have run, and the GL core's stamps,
     * which the copies are keyed by, do not move for a GPU-made chain */
    super::tiled_copy::storage_rewritten(t.base_gpu, |k| backend::gc_kick_pending(k));
    /* Every level is a stream of its own (a new target), whose first draw
     * writes every state group; forgetting them here as well keeps that
     * true if a level ever shares a stream with the draws around it. */
    super::dedup::invalidate();
    for l in 1..n {
        if record_level(t.base_gpu, t.stride_texels, &lv[l - 1], &lv[l]) != 0 {
            /* the levels recorded so far may be in flight, and the CPU
             * filter is about to rewrite them: wait them out first */
            let _ = backend::gc_finish();
            return Err("a level's draw was refused");
        }
    }
    if backend::gc_flush() != 0 {
        let _ = backend::gc_finish();
        return Err("the chain's last kick did not submit");
    }
    super::dedup::invalidate();
    let g = g();
    g.mip_fence = backend::last_submitted_fence();
    note_chain(t.base_gpu, backend::gc_kick_id().wrapping_sub(1));
    Ok(())
}

/// glGenerateMipmap on the 2D texture `b` describes, whose storage holds a
/// chain of `levels` levels (`YakoglBackend::generate_mipmap`): a 2D RGBA8
/// texture, in either channel order, gets its levels 1.. drawn on the GPU
/// ([`chain_gpu`]: one kick per level, every kick submitted before this
/// returns) and the answer is 1: the GL core filters nothing. 0 -- the GL
/// core's CPU box filter, as before -- for any other texture (another
/// format, a cube or array stack), a chain whose layout this backend does
/// not walk ([`chain_layout`]), a chain past the 32-bit address space, a
/// level's draw or the last kick refused (the kicks recorded so far are
/// waited for, since the CPU filter rewrites their levels), or a backend
/// that is not running. Logged once with the reason, and counted
/// ([`gc_backend_mip_chains`]).
///
/// # Safety
/// `b` is null or points at a binding that outlives the call.
pub unsafe extern "C" fn gc_generate_mipmap(b: *const GlesTextureBind, levels: u32) -> i32 {
    // SAFETY: the caller's contract.
    let Some(t) = (unsafe { b.as_ref() }) else { return 0 };
    if !g().inited {
        return 0;
    }
    match chain_gpu(t, levels) {
        Ok(()) => {
            let g = g();
            g.mip_chains = g.mip_chains.wrapping_add(1);
            if g.mip_chains == 1 {
                glog!(
                    "yakogl/vivante: the GPU made the {}-level chain of a {}x{} texture: glGenerateMipmap filters nothing",
                    levels,
                    t.width,
                    t.height
                );
            }
            1
        }
        Err(why) => {
            let g = g();
            g.mip_chains_refused = g.mip_chains_refused.wrapping_add(1);
            if g.mip_chains_refused == 1 {
                glog!(
                    "yakogl/vivante: the {}-level chain of a {}x{} texture (format {}) is left to the CPU: {}",
                    levels,
                    t.width,
                    t.height,
                    t.format,
                    why
                );
            }
            0
        }
    }
}

/// The GL core is about to read or write the texels of a texture whose last
/// glGenerateMipmap this backend made (`YakoglBackend::return_mipmap_chain`;
/// the GL core asks before every CPU write of a texture's texels and before
/// a level above 0 is attached): the levels are in the storage already,
/// written by the chain's kicks, so the wait for the last of them is all
/// there is. The fences are ordered, so the newest chain's fence covers
/// every chain before it.
///
/// # Safety
/// `b` is null or points at a binding that outlives the call.
pub unsafe extern "C" fn gc_return_mipmap_chain(_b: *const GlesTextureBind, _levels: u32) {
    let g = g();
    if !g.inited || g.mip_fence <= 0 {
        return;
    }
    let fence = g.mip_fence;
    if backend::completed_fence() >= fence {
        return;
    }
    g.waits += 1;
    if backend::wait_fence(fence) != 0 {
        glog_once!("yakogl/vivante: the kicks that made a mipmap chain did not complete: the CPU reads what they left");
    }
}

/// The GPU mipmap chains made, and asked for but left to the CPU, since the
/// backend started ([`gc_generate_mipmap`]); the PowerVR's
/// `pvr_backend_mip_chains`. For a harness's record.
///
/// # Safety
/// `made` and `refused` must each be null or valid for a write.
#[no_mangle]
pub unsafe extern "C" fn gc_backend_mip_chains(made: *mut u32, refused: *mut u32) {
    let g = g();
    // SAFETY: null or valid, per the contract above.
    unsafe {
        if !made.is_null() {
            *made = g.mip_chains;
        }
        if !refused.is_null() {
            *refused = g.mip_chains_refused;
        }
    }
}
