// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against d3ce1261 (vivante: a GPU mipmap chain is
//! recorded in one kick, not one kick a level). dEQP-GLES2 texture.mipmap.*
//! and texture.specification/vertex cases call glGenerateMipmap on the
//! GC7000; the i.MX8MP whole tree (0fbe61a3) predates the commit, and its
//! stage-1 promise is 0 Fail. tests/gc_mipmap_gpu.rs and
//! tests/gc_mip_one_kick.rs pin the chain's streams in an ES 3.0 build; this
//! file runs the ES 2.0 formats through the real GL core (a `report-es2`
//! build) and the real Vivante back end over a stand-in OSAL that keeps every
//! allocation in host memory, and asserts for each (format, type) of
//! glTexImage2D on a 2D texture (1x1 to 512x512, power of two, square and not)
//! and on a cube map:
//!
//! * glGenerateMipmap answers GL_NO_ERROR and `gc_refused_draws()` does not
//!   move; the record holds GL's whole chain (`chain_levels`, `level_defined`);
//! * RGBA8 storage (RGBA, RGB, RGB565, RGBA4444 and RGBA5551 uploads are converted
//!   to it): a 2D chain is made by the GPU, as one kick, and the model that
//!   runs the quad's streams (tests/gc_mipmap_gpu.rs's) leaves every level
//!   within 1 LSB a channel of a CPU 2x2 box filter of the level above; a
//!   cube map (the GPU path offers 2D only) is the GL core's CPU filter and
//!   equals the box filter exactly;
//! * L8, A8 and LA8 storage: the CPU filter, equal to the box filter
//!   exactly, a channel at a time;
//!
//! What it cannot see: the GPU (the board's dEQP-GLES2 texture.mipmap.* is
//! that answer).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_more_mipmap
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::{gles_texfmt_bytes, GLES_TEXFMT_A8, GLES_TEXFMT_L8, GLES_TEXFMT_LA8, GLES_TEXFMT_RGBA8};
use yakogl::gl::consts::*;
use yakogl::gl::context::{gl_mip_levels, GlTexture};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::{
    ShcShader, SHC_VIV_CONST_FRAGCOORD_Y_OFFSET, SHC_VIV_CONST_FRAGCOORD_Y_SCALE, SHC_VIV_CONST_UNIFORM,
};
use yakogl::vivante::backend::{gc_init, gc_refused_draws, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::internal::{GC_INTERNAL_CLEAR_VS, GC_INTERNAL_MIPGEN_PS, GC_INTERNAL_TABLE};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

/// The chain the model expects the streams to draw: level l at
/// `levels[l]` = (width, height, byte offset from `base`), `stride` texels
/// a row; `next` is the level the next draw must be.
struct Plan {
    base: u64,
    stride: u32,
    levels: Vec<(u32, u32, u32)>,
    next: usize,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    /// Fences `completed` does not report yet: a kick in flight.
    hold: i64,
    kicks: Vec<Vec<u32>>,
    /// Every `wait`, the fence asked for.
    waits: Vec<i64>,
    faults: Vec<String>,
    /// What `submit` runs the model against; `None` runs nothing.
    plan: Option<Plan>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem, size });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// The stream is kept, its fence raised, and -- with a plan -- its draws run
/// through the model now, while the memory they point at is what the back
/// end built for them. A model that finds the stream wrong says so as a
/// fault (a panic here would cross the `extern "C"` boundary).
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    let (fence, words, plan) = with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return (-22, Vec::new(), None);
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return (-22, Vec::new(), None);
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        let words = words.to_vec();
        p.kicks.push(words.clone());
        p.fence += 1;
        (p.fence, words, p.plan.take())
    });
    if let Some(mut plan) = plan {
        /* outside the lock: the model reads and writes the stand-in's memory */
        for d in draws(&words) {
            if let Err(e) = run_mip_draw(&d, &mut plan) {
                with_platform(|p| p.faults.push(e));
            }
        }
        with_platform(|p| p.plan = Some(plan));
    }
    fence
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    with_platform(|p| {
        p.waits.push(fence);
        /* waited for: complete, as the real one would be */
        if p.fence - p.hold < fence {
            p.hold = p.fence - fence;
        }
    });
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence - p.hold)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// `n` bytes of the stand-in's memory at GPU address `gpu`.
fn read_gpu(gpu: u64, n: usize) -> Result<Vec<u8>, String> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back().ok_or_else(|| format!("no allocation at 0x{gpu:x}"))?;
        let off = (gpu - base) as usize;
        if off + n > a.size {
            return Err(format!("0x{gpu:x}+{n} runs off the allocation at 0x{base:x} ({} bytes)", a.size));
        }
        // SAFETY: inside the allocation (checked just above).
        Ok(unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec())
    })
}

/// Write `bytes` into the stand-in's memory at GPU address `gpu` (the
/// model's pixel engine).
fn write_gpu(gpu: u64, bytes: &[u8]) -> Result<(), String> {
    with_platform(|p| {
        let (base, a) = p.allocs.range_mut(..=gpu).next_back().ok_or_else(|| format!("no allocation at 0x{gpu:x}"))?;
        let off = (gpu - base) as usize;
        if off + bytes.len() > a.size {
            return Err(format!("0x{gpu:x}+{} runs off the allocation at 0x{base:x}", bytes.len()));
        }
        // SAFETY: inside the allocation (checked just above).
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), (a.mem.as_mut_ptr() as *mut u8).add(off), bytes.len())
        };
        Ok(())
    })
}

/* ---- GL plumbing ------------------------------------------------------------------- */

fn with_backend<R>(case: impl FnOnce() -> R) -> R {
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    let faults = with_platform(|p| {
        /* The memory is leaked, not freed, and the addresses keep counting
         * up: a static of the library that still names an earlier case's
         * block must find memory, not a hole (tests/gc_npot_repeat.rs). */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        p.plan = None;
        core::mem::take(&mut p.faults)
    });
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract, or the model found a stream wrong: {faults:?}");
    r
}

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

/* ---- the model: what the GPU does with the streams ------------------------------- */

fn words_of(sh: &ShcShader) -> &'static [u32] {
    // SAFETY: a row of the generated table points at its `[u32; usc_dw]` static.
    unsafe { std::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }
}

/// One DRAW_INSTANCED of a stream, with the register file as the stream
/// had loaded it when the draw ran.
struct Draw {
    regs: BTreeMap<u32, u32>,
    vertices: u32,
    prim: u32,
}

/// The draws of a stream, in order.
fn draws(words: &[u32]) -> Vec<Draw> {
    let mut regs = BTreeMap::new();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    regs.insert(base + 4 * k as u32, words[i + 1 + k]);
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                out.push(Draw { regs: regs.clone(), vertices: words[i + 1] & 0x00ff_ffff, prim: (w >> 16) & 0xf });
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

/// 16.16 fixed point, as the viewport and scissor registers hold it.
fn fixp(v: u32) -> f32 {
    v as i32 as f32 / 65536.0
}

/// A texel as the texture unit reads it through format A8R8G8B8 (B,G,R,A
/// in memory), as (r, g, b, a).
fn read_texel(mem: &[u8], at: usize) -> [f32; 4] {
    [mem[at + 2] as f32, mem[at + 1] as f32, mem[at] as f32, mem[at + 3] as f32]
}

fn le_words(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

/// `check!(cond, "what")`: the model's assertion, as an `Err` (it runs
/// inside the OSAL's `submit`, where a panic may not unwind).
macro_rules! check {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            return Err(format!($($arg)*));
        }
    };
}

/// Run one mipmap quad draw as the GPU would: it must be the next level of
/// the plan; every register it stands on is checked against that level and
/// the one above, then the level is written into the storage.
fn run_mip_draw(d: &Draw, plan: &mut Plan) -> Result<(), String> {
    let l = plan.next;
    check!(l >= 1 && l < plan.levels.len(), "a draw past the chain's {} levels", plan.levels.len());
    let (sw, sh, soff) = plan.levels[l - 1];
    let (dw, dh, doff) = plan.levels[l];
    let (base, stride) = (plan.base, plan.stride);
    let what = format!("level {l} ({dw}x{dh} from {sw}x{sh})");
    let r = |reg: u32| d.regs.get(&reg).copied().ok_or_else(|| format!("{what}: register 0x{reg:x} never written"));
    let mipgen = &GC_INTERNAL_TABLE[GC_INTERNAL_MIPGEN_PS].shader;
    let quad_vs = &GC_INTERNAL_TABLE[GC_INTERNAL_CLEAR_VS].shader;

    /* the programs: the mipmap quad's, by their words */
    let ps = le_words(&read_gpu(u64::from(r(GC_PS_INST_ADDR)?), mipgen.usc_dw as usize * 4)?);
    check!(ps == words_of(mipgen), "{what}: the fragment program is not the mipmap quad's");
    let vs = le_words(&read_gpu(u64::from(r(GC_VS_INST_ADDR)?), quad_vs.usc_dw as usize * 4)?);
    check!(vs == words_of(quad_vs), "{what}: the vertex program is not the quad's");

    /* the quad: four clip-space corners, a strip over the whole target */
    check!((d.prim, d.vertices) == (GC_PRIM_TRIANGLE_STRIP, 4), "{what}: one four-vertex strip");
    let quad: Vec<f32> = le_words(&read_gpu(u64::from(r(nfe_vertex_streams_base_addr(0))?), 64)?)
        .into_iter()
        .map(f32::from_bits)
        .collect();
    check!(
        quad == [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0],
        "{what}: the quad's corners {quad:?}"
    );

    /* the viewport: the level, unflipped (gl_FragCoord.y is the row in
     * memory order), and the scissor the same */
    let vp = [
        fixp(r(GC_PA_VIEWPORT_SCALE_X)?),
        fixp(r(GC_PA_VIEWPORT_SCALE_Y)?),
        fixp(r(GC_PA_VIEWPORT_OFFSET_X)?),
        fixp(r(GC_PA_VIEWPORT_OFFSET_Y)?),
    ];
    let half = [dw as f32 / 2.0, dh as f32 / 2.0, dw as f32 / 2.0, dh as f32 / 2.0];
    check!(vp == half, "{what}: viewport {vp:?}, not {half:?}");
    let clip = [
        r(GC_SE_SCISSOR_LEFT)? >> 16,
        r(GC_SE_SCISSOR_TOP)? >> 16,
        (r(GC_SE_SCISSOR_RIGHT)? - GC_SE_SCISSOR_MARGIN_RIGHT) >> 16,
        (r(GC_SE_SCISSOR_BOTTOM)? - GC_SE_SCISSOR_MARGIN_BOTTOM) >> 16,
    ];
    check!(clip == [0, 0, dw, dh], "{what}: the scissor {clip:?} is not the level");

    /* the pixel engine: the level, linear A8R8G8B8 at the chain's pitch,
     * every channel, no depth or stencil, no blend */
    let pe_fmt = r(GC_PE_COLOR_FORMAT)?;
    check!(pe_color_format_format(pe_fmt) == GC_PE_FORMAT_A8R8G8B8, "{what}: PE format {pe_fmt:#x}");
    check!(pe_fmt & pe_color_format_components(0xf) == pe_color_format_components(0xf), "{what}: not every channel");
    check!(
        pe_fmt & (GC_PE_COLOR_FORMAT_SUPER_TILED | GC_PE_COLOR_FORMAT_FORMAT_MASK) == 0,
        "{what}: not linear 32-bit"
    );
    check!(r(GC_PE_COLOR_STRIDE)? == stride * 4, "{what}: PE stride {} is not the chain's", r(GC_PE_COLOR_STRIDE)?);
    let target = u64::from(r(pe_pipe_color_addr(0))?);
    check!(
        target == base + u64::from(doff),
        "{what}: PE address {target:#x}, not the level's {:#x}",
        base + u64::from(doff)
    );
    check!(r(pe_pipe_color_addr(1))? == target as u32, "{what}: the second pipe's address");
    check!(r(GC_PE_DEPTH_CONFIG)? & GC_PE_DEPTH_DISABLE_ZS != 0, "{what}: depth or stencil on");
    check!(r(GC_PE_ALPHA_CONFIG)? == 0, "{what}: blending on");

    /* the source: the level above, bound alone: linear 2D A8R8G8B8 at the
     * chain's pitch, both filters linear, no mipmaps, CLAMP_TO_EDGE */
    let desc = read_gpu(u64::from(r(nte_descriptor_addr(0))?), GC_TEXDESC_BYTES)?;
    let dw32 = |off: u32| u32::from_le_bytes(desc[off as usize..off as usize + 4].try_into().unwrap());
    let config0 = texdesc_type(GC_TEX_TYPE_2D)
        | texdesc_format(GC_TEX_FORMAT_A8R8G8B8)
        | texdesc_addressing_mode(GC_TEX_ADDRESSING_LINEAR);
    check!(
        dw32(GC_TEXDESC_CONFIG0) == config0,
        "{what}: the source descriptor's CONFIG0 {:#x}",
        dw32(GC_TEXDESC_CONFIG0)
    );
    check!(
        dw32(GC_TEXDESC_SIZE) == texdesc_size_width(sw) | texdesc_size_height(sh),
        "{what}: the source's size {:#x}",
        dw32(GC_TEXDESC_SIZE)
    );
    check!(dw32(GC_TEXDESC_LINEAR_STRIDE) == stride * 4, "{what}: the source's stride");
    check!(
        dw32(GC_TEXDESC_BASELOD) == texdesc_baselod_base(0) | texdesc_baselod_max(0),
        "{what}: the source has levels"
    );
    let lod0 = u64::from(dw32(texdesc_lod_addr(0)));
    check!(lod0 == base + u64::from(soff), "{what}: the source's address {lod0:#x}, not {:#x}", base + u64::from(soff));
    let ctrl0 = r(nte_descriptor_samp_ctrl0(0))?;
    let sampler = samp_uwrap(GC_TEX_WRAP_CLAMP_TO_EDGE)
        | samp_vwrap(GC_TEX_WRAP_CLAMP_TO_EDGE)
        | samp_wwrap(GC_TEX_WRAP_CLAMP_TO_EDGE)
        | samp_min(GC_TEX_FILTER_LINEAR)
        | samp_mip(GC_TEX_FILTER_NONE)
        | samp_mag(GC_TEX_FILTER_LINEAR);
    check!(ctrl0 & 0x7fff == sampler, "{what}: sampler {ctrl0:#x}");

    /* the fragment program's constants: u_scale = 1 / the level's size, and
     * gl_FragCoord.y unflipped (scale 1, offset 0) */
    let ps_base = GC_SH_HALTI5_UNIFORMS_PS + r(GC_PS_UNIFORM_BASE)? * 16;
    let (mut scale, mut y_scale, mut y_offset) = ([None, None], None, None);
    for (k, (&c, &data)) in mipgen.viv.const_contents().iter().zip(mipgen.viv.const_data()).enumerate() {
        let v = f32::from_bits(r(ps_base + 4 * k as u32)?);
        match c {
            SHC_VIV_CONST_UNIFORM => scale[data as usize] = Some(v),
            SHC_VIV_CONST_FRAGCOORD_Y_SCALE => y_scale = Some(v),
            SHC_VIV_CONST_FRAGCOORD_Y_OFFSET => y_offset = Some(v),
            _ => {}
        }
    }
    check!(scale == [Some(1.0 / dw as f32), Some(1.0 / dh as f32)], "{what}: u_scale {scale:?}");
    check!(
        (y_scale, y_offset) == (Some(1.0), Some(0.0)),
        "{what}: gl_FragCoord.y {y_scale:?} {y_offset:?} is not the row"
    );

    /* the quad, run: at the centre of each target texel, a bilinear
     * CLAMP_TO_EDGE sample of the source; the pixel engine writes B,G,R,A */
    let src_bytes = read_gpu(lod0, (stride * sh * 4) as usize)?;
    let mut out = vec![0u8; (dw * 4) as usize];
    for y in 0..dh {
        for x in 0..dw {
            let u = (x as f32 + 0.5) * (1.0 / dw as f32) * sw as f32 - 0.5;
            let v = (y as f32 + 0.5) * (1.0 / dh as f32) * sh as f32 - 0.5;
            let (x0, y0) = (u.floor(), v.floor());
            let (fx, fy) = (u - x0, v - y0);
            let tx = |i: f32| (i.max(0.0) as u32).min(sw - 1);
            let ty = |j: f32| (j.max(0.0) as u32).min(sh - 1);
            let at = |i: u32, j: u32| ((j * stride + i) * 4) as usize;
            let (x0i, x1i, y0i, y1i) = (tx(x0), tx(x0 + 1.0), ty(y0), ty(y0 + 1.0));
            let (p00, p10, p01, p11) = (
                read_texel(&src_bytes, at(x0i, y0i)),
                read_texel(&src_bytes, at(x1i, y0i)),
                read_texel(&src_bytes, at(x0i, y1i)),
                read_texel(&src_bytes, at(x1i, y1i)),
            );
            let mut rgba = [0u8; 4];
            for ch in 0..4 {
                let top = p00[ch] * (1.0 - fx) + p10[ch] * fx;
                let bot = p01[ch] * (1.0 - fx) + p11[ch] * fx;
                rgba[ch] = ((top * (1.0 - fy) + bot * fy) + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
            let o = (x * 4) as usize;
            out[o..o + 4].copy_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        write_gpu(target + u64::from(y * stride * 4), &out)?;
    }
    plan.next += 1;
    Ok(())
}

/* ---- the reference: the 2x2 box filter ------------------------------------------- */

/// Level `l` of `t` from the storage: `layers * lh` rows of `lw * bpp` bytes,
/// the faces of a cube map stacked one after another.
fn level_bytes(t: &GlTexture, l: u32) -> Vec<u8> {
    let (lw, lh) = ((t.width >> l).max(1) as usize, (t.height >> l).max(1) as usize);
    let bpp = gles_texfmt_bytes(t.texfmt) as usize;
    let pitch = t.level_pitch_at(l) as usize * bpp;
    let off = t.level_off[l as usize] as usize;
    let rows = t.layers.max(1) as usize * lh;
    // SAFETY: the texture's live storage, mem.size bytes.
    let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
    (0..rows).flat_map(|y| mem[off + y * pitch..off + y * pitch + lw * bpp].iter().copied()).collect()
}

/// Level `l` from level `l - 1`: the 2x2 box average of every byte, round
/// half up (a side already 1 stays 1), a face at a time. `src` is
/// `layers * sh` rows of `sw * bpp` bytes.
fn box_filter(src: &[u8], (sw, sh): (usize, usize), layers: usize, bpp: usize) -> Vec<u8> {
    let (dw, dh) = ((sw / 2).max(1), (sh / 2).max(1));
    let (nx, ny) = (if sw > 1 { 2 } else { 1 }, if sh > 1 { 2 } else { 1 });
    let mut out = Vec::with_capacity(layers * dh * dw * bpp);
    for f in 0..layers {
        for y in 0..dh {
            for x in 0..dw {
                for ch in 0..bpp {
                    let mut sum = 0usize;
                    for j in 0..ny {
                        for i in 0..nx {
                            sum += src[(((f * sh + y * ny + j) * sw) + x * nx + i) * bpp + ch] as usize;
                        }
                    }
                    out.push(((sum + nx * ny / 2) / (nx * ny)) as u8);
                }
            }
        }
    }
    out
}

/// Scribble over levels 1.. of `t`'s storage, so what the model leaves there
/// is the model's and not the CPU filter's.
fn scramble_chain(t: &GlTexture) {
    let from = t.level_off[1] as usize;
    // SAFETY: the texture's live storage, mem.size bytes; no kick uses it
    // (the stand-in completes every kick at once).
    let mem = unsafe { core::slice::from_raw_parts_mut(t.mem.cpu as *mut u8, t.mem.size) };
    for (i, b) in mem[from..].iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37) ^ 0x5a;
    }
}

/* ---- the cases -------------------------------------------------------------------- */

/// An ES 2.0 glTexImage2D (format, type) and the texel format the GL core
/// stores it as.
struct Fmt {
    name: &'static str,
    format: GLenum,
    type_: GLenum,
    /// Bytes of one texel as uploaded.
    upload_bpt: usize,
    stored: u8,
}

const fn fmt(name: &'static str, format: GLenum, type_: GLenum, upload_bpt: usize, stored: u8) -> Fmt {
    Fmt { name, format, type_, upload_bpt, stored }
}

const RGBA: Fmt = fmt("RGBA/UNSIGNED_BYTE", GL_RGBA, GL_UNSIGNED_BYTE, 4, GLES_TEXFMT_RGBA8);
const RGB: Fmt = fmt("RGB/UNSIGNED_BYTE", GL_RGB, GL_UNSIGNED_BYTE, 3, GLES_TEXFMT_RGBA8);
const RGBA4444: Fmt = fmt("RGBA/UNSIGNED_SHORT_4_4_4_4", GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, 2, GLES_TEXFMT_RGBA8);
const RGBA5551: Fmt = fmt("RGBA/UNSIGNED_SHORT_5_5_5_1", GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1, 2, GLES_TEXFMT_RGBA8);
const RGB565: Fmt = fmt("RGB/UNSIGNED_SHORT_5_6_5", GL_RGB, GL_UNSIGNED_SHORT_5_6_5, 2, GLES_TEXFMT_RGBA8);
const LUMINANCE: Fmt = fmt("LUMINANCE/UNSIGNED_BYTE", GL_LUMINANCE, GL_UNSIGNED_BYTE, 1, GLES_TEXFMT_L8);
const LUMINANCE_ALPHA: Fmt =
    fmt("LUMINANCE_ALPHA/UNSIGNED_BYTE", GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE, 2, GLES_TEXFMT_LA8);
const ALPHA: Fmt = fmt("ALPHA/UNSIGNED_BYTE", GL_ALPHA, GL_UNSIGNED_BYTE, 1, GLES_TEXFMT_A8);

/// Texels no box filter can get right by accident: every byte its own.
fn bytes(n: usize, seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(n as u32) | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 7) as u8
        })
        .collect()
}

/// The `target` texture of the current context, after the GL calls on it
/// are done (the record is read, not written, from here).
fn texture(target: GLenum) -> &'static GlTexture {
    let c = yakogl::gl::context::current().expect("a current context");
    c.objs_ref().textures.iter().find(|t| t.used && t.target == target).expect("the texture")
}

/// glTexImage2D of a `w` x `h` texture (six faces for a cube map) under a
/// mipmap filter, then glGenerateMipmap, in the ES 2.0 context: GL_NO_ERROR
/// all the way, no refused draw, GL's whole chain in the record, and every
/// level above 0 the 2x2 box filter of the one before it.
fn case(f: &Fmt, cube: bool, w: u32, h: u32, seed: u32) {
    assert!(!cube || w == h, "a cube face is square");
    let what = format!("{} {}{w}x{h}", f.name, if cube { "cube " } else { "" });
    let target = if cube { GL_TEXTURE_CUBE_MAP } else { GL_TEXTURE_2D };
    let faces = if cube { 6 } else { 1 };
    let levels = 32 - w.max(h).leading_zeros();
    with_backend(|| {
        let mut tex: GLuint = 0;
        // SAFETY: plain GL calls; every upload holds the w x h texels.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(target, tex);
            glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            for face in 0..faces {
                let data = bytes((w * h) as usize * f.upload_bpt, seed + face);
                let t = if cube { GL_TEXTURE_CUBE_MAP_POSITIVE_X + face } else { GL_TEXTURE_2D };
                let (iw, ih) = (w as GLsizei, h as GLsizei);
                glTexImage2D(t, 0, f.format as GLint, iw, ih, 0, f.format, f.type_, data.as_ptr() as *const c_void);
            }
        }
        no_error(&format!("{what}: glTexImage2D"));
        let t = texture(target);
        assert_eq!(t.texfmt, f.stored, "{what}: the stored format");
        let layers = t.layers.max(1) as usize;
        assert_eq!(layers, faces as usize, "{what}: layers");
        let l0 = level_bytes(t, 0);
        let gpu = !cube && t.texfmt == GLES_TEXFMT_RGBA8 && !t.bgra && levels >= 2;
        if gpu {
            assert_ne!(t.chain_pitch, 0, "{what}: the Vivante chain is at level 0's pitch");
            scramble_chain(t);
            let plan_levels: Vec<(u32, u32, u32)> =
                (0..levels).map(|l| ((w >> l).max(1), (h >> l).max(1), t.level_off[l as usize])).collect();
            let stride = t.level_pitch(t.width);
            with_platform(|p| p.plan = Some(Plan { base: t.mem.gpu, stride, levels: plan_levels, next: 1 }));
        }
        let (kicks0, refused0) = (with_platform(|p| p.kicks.len()), gc_refused_draws());
        // SAFETY: a plain GL call.
        unsafe { glGenerateMipmap(target) };
        no_error(&format!("{what}: glGenerateMipmap"));
        assert_eq!(gc_refused_draws(), refused0, "{what}: a draw was refused");
        let (kicks, drawn, faults) = with_platform(|p| {
            (p.kicks.len() - kicks0, p.plan.take().map_or(0, |pl| pl.next), core::mem::take(&mut p.faults))
        });
        assert!(faults.is_empty(), "{what}: the model found the streams wrong: {faults:#?}");
        let t = texture(target);
        assert_eq!(t.chain_in_backend, gpu, "{what}: who made the chain");
        let defined = ((1u32 << gl_mip_levels(w, h).min(15)) - 1) as u16;
        assert_eq!(t.level_defined, defined, "{what}: GL's whole chain exists in the record");
        assert_eq!(level_bytes(t, 0), l0, "{what}: level 0 untouched");
        if gpu {
            assert_eq!(kicks, 1, "{what}: one kick for the chain");
            assert_eq!(drawn, levels as usize, "{what}: every level drawn, in order");
        } else {
            assert_eq!(kicks, 0, "{what}: the CPU filter makes no kick");
        }
        assert_eq!(u32::from(t.chain_levels), levels, "{what}: the level count");
        let bpp = gles_texfmt_bytes(t.texfmt) as usize;
        let tol = u8::from(gpu);
        let mut prev = l0;
        for l in 1..levels {
            let src = ((w >> (l - 1)).max(1) as usize, (h >> (l - 1)).max(1) as usize);
            let reference = box_filter(&prev, src, layers, bpp);
            let got = level_bytes(t, l);
            assert_eq!(got.len(), reference.len(), "{what} level {l}: size");
            let d = got.iter().zip(&reference).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(d <= tol, "{what} level {l}: {d} LSB from the box filter of level {}", l - 1);
            prev = got;
        }
    });
}

const SIZES_2D: [(u32, u32); 13] = [
    (1, 1),
    (2, 2),
    (4, 1),
    (1, 8),
    (2, 16),
    (16, 16),
    (32, 8),
    (64, 64),
    (128, 32),
    (8, 128),
    (256, 256),
    (512, 512),
    (512, 64),
];
const SIZES_CUBE: [u32; 7] = [1, 2, 4, 16, 64, 128, 256];

#[test]
fn rgba8_formats_chain_on_the_gpu_in_one_kick() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (i, f) in [RGBA, RGB, RGB565, RGBA4444, RGBA5551].iter().enumerate() {
        for (j, &(w, h)) in SIZES_2D.iter().enumerate() {
            case(f, false, w, h, (i * 31 + j) as u32);
        }
    }
}

#[test]
fn narrow_formats_chain_on_the_cpu() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (i, f) in [LUMINANCE, LUMINANCE_ALPHA, ALPHA].iter().enumerate() {
        for (j, &(w, h)) in SIZES_2D.iter().enumerate() {
            case(f, false, w, h, (i * 17 + j) as u32);
        }
    }
}

#[test]
fn cube_faces_chain_a_face_at_a_time() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (i, f) in [RGBA, RGB, RGBA4444, RGBA5551, RGB565, LUMINANCE, LUMINANCE_ALPHA, ALPHA].iter().enumerate() {
        for (j, &s) in SIZES_CUBE.iter().enumerate() {
            case(f, true, s, s, (i * 13 + j) as u32);
        }
    }
}
