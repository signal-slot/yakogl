// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): glGenerateMipmap on the GPU (src/vivante/mipmap.rs)
//! makes the chain the GL core's CPU box filter makes, level for level.
//!
//! How it runs: the real GL core lays a texture out, uploads it
//! (glTexImage2D) and filters its chain (glGenerateMipmap), twice.
//!
//! 1. On a stand-in back end of the GC7000's shape that declines the chain
//!    (`generate_mipmap` absent): the GL core box-filters it on the CPU, at
//!    the layout it keeps for this back end (every level at level 0's
//!    pitch, on a 64-byte boundary). Those levels are the reference.
//! 2. On the real Vivante back end (`gc_init`, `gc_draw`, `gc_flush`) over
//!    a stand-in OSAL that keeps every allocation in host memory. Levels
//!    1.. of the storage are scribbled over, and the back end's
//!    `generate_mipmap` entry is asked for the chain as the GL core would
//!    ask it (the binding `gl::draw::texture_bind_2d` builds). The OSAL's
//!    `submit` then does what the GPU would do with each stream, as it
//!    arrives: it decodes each draw's LOAD_STATEs, reads the texture
//!    descriptor and the sampler, the pixel engine's target and the
//!    fragment program's uniforms, checks every one against the level it
//!    should name, and runs the mipmap quad's fragment program as a model
//!    -- one bilinear CLAMP_TO_EDGE sample of the source at the centre of
//!    each target texel, through the descriptor's channel order in and the
//!    pixel engine's out -- into the texture's storage. (At submit time,
//!    not after: the back end reuses a frame's arena, and the descriptors in
//!    it, four kicks on.)
//!
//! Then every level above 0 of the storage is compared with the reference:
//! within 1 LSB a channel where the level is half the one above (every
//! level of a power-of-two chain; dEQP's generate-mipmap compare allows as
//! much), and against the bilinear sample the quad asks for on a
//! non-power-of-two chain's odd steps (800x600's 75 -> 37; ES 3.0.6 3.8.9
//! leaves that filter open). The back end must have taken the GPU path
//! (answered 1, one kick per level, the chain counted), and left to the CPU
//! what it says it leaves.
//!
//! What it cannot see: the GPU. Whether the pixel engine and the texture
//! unit do what the model does with these registers is the board's answer
//! (dEQP-GLES2 texture.mipmap.*, glmark2's terrain and refract pictures).
//!
//! The GL core's own glGenerateMipmap reaches the entry too (#1106): a
//! texture a draw rendered into (`pe_order`, at the chain's one pitch), as
//! glmark2's terrain and refract targets are every frame, has its chain made
//! here and not by the CPU filter
//! (`glmark2s_rendered_into_chains_are_offered_to_the_gpu`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_mipmap_gpu
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{
    gles_attribf_bit, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_ATTRIBF_BYTE, GLES_ATTRIBF_FIXED,
    GLES_ATTRIBF_SHORT, GLES_ATTRIBF_UBYTE, GLES_ATTRIBF_USHORT, GLES_TEXFMT_L8, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D,
    GLES_TEXTYPE_CUBE,
};
use yakogl::gl::consts::*;
use yakogl::gl::context::GlTexture;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::{glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glFinish, glGetError, glPixelStorei, glViewport};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::{
    ShcShader, SHC_VIV_CONST_FRAGCOORD_Y_OFFSET, SHC_VIV_CONST_FRAGCOORD_Y_SCALE, SHC_VIV_CONST_UNIFORM,
};
use yakogl::vivante::backend::{gc_init, gc_shutdown, yakogl_backend_vivante};
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

/* ---- the GC7000's shape, declining the chain: the GL core's CPU filter ------------- */

unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
    0x4000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}

/// `yakogl_backend_vivante`'s flags the GL core lays a texture out by, with
/// no `generate_mipmap`: the GL core box-filters the chain on the CPU, at
/// the Vivante layout (`mip_level_align` 64).
static GC_SHAPE: YakoglBackend = YakoglBackend {
    name: c"GC7000 shape, no GPU mipmap".as_ptr() as *const c_char,
    native_attrib_formats: gles_attribf_bit(GLES_ATTRIBF_BYTE)
        | gles_attribf_bit(GLES_ATTRIBF_UBYTE)
        | gles_attribf_bit(GLES_ATTRIBF_SHORT)
        | gles_attribf_bit(GLES_ATTRIBF_USHORT)
        | gles_attribf_bit(GLES_ATTRIBF_FIXED),
    rt_rgba8888_is_bgra: true,
    mip_level_align: 64,
    compressed_blocks_in_rows: true,
    native_layered_textures: true,
    front_facing_shader_variant: true,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

/* ---- GL plumbing ------------------------------------------------------------------- */

/// Run `case` with `backend` installed over the stand-in OSAL and a fresh
/// 64x64 context; the real Vivante back end is brought up and down around
/// it.
fn with_backend<R>(backend: &'static YakoglBackend, case: impl FnOnce() -> R) -> R {
    let vivante = core::ptr::eq(backend, &yakogl_backend_vivante);
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = backend;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        if vivante {
            assert_eq!(gc_init(), 0, "gc_init");
        }
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        if vivante {
            gc_shutdown();
        }
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

/// Texels no box filter can get right by accident: every byte its own.
fn texels(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(w * 31 + h);
    (0..w * h * 4)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 7) as u8
        })
        .collect()
}

/// glTexImage2D of a `w` x `h` RGBA8 texture holding `level0`, under a
/// mipmap filter, then glGenerateMipmap, on the current context: the chain
/// exists in the storage after it (a non-power-of-two chain's storage is
/// grown by glGenerateMipmap, `gl::texture::grow_npot_chain`). Returns how
/// long glGenerateMipmap took (the CPU box filter on [`GC_SHAPE`], the GPU
/// path's recording on the Vivante back end, which the GL core offers the
/// chain to).
fn upload_and_filter(w: u32, h: u32, level0: &[u8]) -> std::time::Duration {
    upload(w, h, level0);
    let t0 = std::time::Instant::now();
    // SAFETY: a plain GL call.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    let took = t0.elapsed();
    no_error("glGenerateMipmap");
    took
}

/// glTexImage2D of a `w` x `h` RGBA8 texture holding `level0`, under a
/// mipmap filter, left bound; its name.
fn upload(w: u32, h: u32, level0: &[u8]) -> GLuint {
    let mut tex: GLuint = 0;
    // SAFETY: plain GL calls; `level0` holds the w x h texels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            w as GLsizei,
            h as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            level0.as_ptr() as *const c_void,
        );
        no_error("glTexImage2D");
    }
    tex
}

/// The `w` x `h` 2D texture of the current context, after the GL calls on
/// it are done (the record is read, not written, from here).
fn texture(w: u32, h: u32) -> &'static GlTexture {
    let c = yakogl::gl::context::current().expect("a current context");
    c.objs_ref()
        .textures
        .iter()
        .find(|t| t.used && t.width == w && t.height == h && t.target == GL_TEXTURE_2D)
        .expect("the texture")
}

/// The texels of level `l` of `t`, one row after another, `lw * 4` bytes a
/// row: read from the storage where the GL core put the level.
fn level_rows(t: &GlTexture, l: u32) -> Vec<u8> {
    let (lw, lh) = ((t.width >> l).max(1), (t.height >> l).max(1));
    let pitch = (t.level_pitch_at(l) * 4) as usize;
    let off = t.level_off[l as usize] as usize;
    // SAFETY: the texture's live storage, mem.size bytes.
    let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
    (0..lh as usize).flat_map(|y| mem[off + y * pitch..off + y * pitch + lw as usize * 4].iter().copied()).collect()
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

/// The chain the GL core's CPU box filter makes of `level0` (phase 1): the
/// rows of every level, the layout it used, and how long glGenerateMipmap
/// took.
struct CpuChain {
    levels: Vec<Vec<u8>>,
    level_off: Vec<u32>,
    chain_pitch: u32,
    took: std::time::Duration,
}

fn cpu_chain(w: u32, h: u32, level0: &[u8]) -> CpuChain {
    with_backend(&GC_SHAPE, || {
        let took = upload_and_filter(w, h, level0);
        let t = texture(w, h);
        assert!(!t.chain_in_backend, "the stand-in declines the chain: the CPU filtered it");
        let n = u32::from(t.chain_levels);
        CpuChain {
            levels: (0..n).map(|l| level_rows(t, l)).collect(),
            level_off: t.level_off[..n as usize].to_vec(),
            chain_pitch: t.chain_pitch,
            took,
        }
    })
}

/// The binding the GL core hands `generate_mipmap` (`gl::draw::texture_bind_2d`).
fn bind_of(t: &GlTexture) -> GlesTextureBind {
    let stride = t.level_pitch(t.width);
    GlesTextureBind {
        unit: 0,
        format: t.texfmt,
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: t.mem.gpu,
        width: t.width,
        height: t.height,
        stride_texels: stride,
        bytes: stride * t.height * 4,
        linear_filter: 1,
        min_linear: 1,
        mip_filter: 2,
        mip_levels: t.chain_levels,
        bgra: (t.bgra || t.pe_order) as u8,
        content_gen: t.content_gen,
        cpu: t.mem.cpu as *const c_void,
        ..GlesTextureBind::ZERO
    }
}

/// (made, refused) since the back end started.
fn mip_counts() -> (u32, u32) {
    extern "C" {
        fn gc_backend_mip_chains(made: *mut u32, refused: *mut u32);
    }
    let (mut made, mut refused) = (0u32, 0u32);
    // SAFETY: two valid u32 out-pointers.
    unsafe { gc_backend_mip_chains(&mut made, &mut refused) };
    (made, refused)
}

fn generate(b: &GlesTextureBind, levels: u32) -> i32 {
    let f = yakogl_backend_vivante.generate_mipmap.expect("the Vivante back end answers generate_mipmap");
    // SAFETY: a valid binding for the call.
    unsafe { f(b, levels) }
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

/* ---- the reference ---------------------------------------------------------------- */

/// Level `l` from level `l - 1` as the quad asks for it: the 2x2 box
/// average (round half up, the GL core's filter) where the level is half
/// the one above on both axes, the bilinear sample at the texel's centre
/// where it is not (an odd step of a non-power-of-two chain).
fn filtered(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    let exact = halves(sw, sh, dw, dh);
    for y in 0..dh {
        for x in 0..dw {
            for ch in 0..4usize {
                let px = |i: u32, j: u32| src[((j * sw + i) * 4) as usize + ch] as f32;
                let v = if exact {
                    let (sx, sy) = (if sw > 1 { 2 } else { 1 }, if sh > 1 { 2 } else { 1 });
                    let mut sum = 0.0;
                    for j in 0..sy {
                        for i in 0..sx {
                            sum += px(x * sx + i, y * sy + j);
                        }
                    }
                    sum / (sx * sy) as f32
                } else {
                    let u = (x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5;
                    let v = (y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5;
                    let (x0, y0) = (u.floor(), v.floor());
                    let (fx, fy) = (u - x0, v - y0);
                    let cx = |i: f32| (i.max(0.0) as u32).min(sw - 1);
                    let cy = |j: f32| (j.max(0.0) as u32).min(sh - 1);
                    let top = px(cx(x0), cy(y0)) * (1.0 - fx) + px(cx(x0 + 1.0), cy(y0)) * fx;
                    let bot = px(cx(x0), cy(y0 + 1.0)) * (1.0 - fx) + px(cx(x0 + 1.0), cy(y0 + 1.0)) * fx;
                    top * (1.0 - fy) + bot * fy
                };
                out[((y * dw + x) * 4) as usize + ch] = (v + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Is `dw` x `dh` exactly half of `sw` x `sh` (a side already 1 stays 1)?
fn halves(sw: u32, sh: u32, dw: u32, dh: u32) -> bool {
    (sw == 2 * dw || (sw == 1 && dw == 1)) && (sh == 2 * dh || (sh == 1 && dh == 1))
}

/// The largest difference of a channel between two levels of the same size.
fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

/* ---- the cases -------------------------------------------------------------------- */

/// One size through both paths: the GPU chain (the model) against the GL
/// core's CPU chain where the step halves exactly, and against the quad's
/// own filter everywhere; the layout the back end wrote is the GL core's.
fn chain_case(w: u32, h: u32, seed: u32) {
    let level0 = texels(w, h, seed);
    let cpu = cpu_chain(w, h, &level0);
    let levels = cpu.levels.len() as u32;
    assert_eq!(levels, 32 - w.max(h).leading_zeros(), "{w}x{h}: GL's whole chain in the storage");
    assert_eq!(cpu.levels[0], level0, "{w}x{h}: level 0 as uploaded");

    with_backend(&yakogl_backend_vivante, || {
        upload_and_filter(w, h, &level0);
        let t = texture(w, h);
        assert_eq!((t.chain_levels as u32, t.chain_pitch), (levels, cpu.chain_pitch), "{w}x{h}: the same layout");
        assert_eq!(&t.level_off[..levels as usize], &cpu.level_off[..], "{w}x{h}: the same level offsets");
        assert_ne!(t.chain_pitch, 0, "{w}x{h}: the Vivante chain is at level 0's pitch");
        scramble_chain(t);
        let b = bind_of(t);
        let plan_levels: Vec<(u32, u32, u32)> =
            (0..levels).map(|l| ((w >> l).max(1), (h >> l).max(1), t.level_off[l as usize])).collect();
        with_platform(|p| {
            p.plan = Some(Plan { base: t.mem.gpu, stride: b.stride_texels, levels: plan_levels, next: 1 })
        });
        let kicks0 = with_platform(|p| p.kicks.len());
        let before = mip_counts();
        assert_eq!(generate(&b, levels), 1, "{w}x{h}: the GPU path");
        assert_eq!(mip_counts(), (before.0 + 1, before.1), "{w}x{h}: the chain is counted");
        let (kicks, drawn, faults) = with_platform(|p| {
            (p.kicks.len() - kicks0, p.plan.as_ref().map_or(0, |pl| pl.next), core::mem::take(&mut p.faults))
        });
        assert!(faults.is_empty(), "{w}x{h}: the model found the streams wrong: {faults:#?}");
        assert_eq!(kicks, levels as usize - 1, "{w}x{h}: one kick per level above 0");
        assert_eq!(drawn, levels as usize, "{w}x{h}: every level drawn, in order");
        let chain_diff = check_gpu_levels(t, &level0, &cpu);

        /* For the record (`--nocapture`): the CPU the two paths cost on this
         * host -- the GL core's box filter of the whole chain against the
         * back end's recording of the chain's draws (the model off: what the
         * GPU does is not CPU time). Host time, this build's profile; the
         * board's CPU is slower and its carve-out non-cacheable. */
        with_platform(|p| p.plan = None);
        let t0 = std::time::Instant::now();
        const RUNS: u32 = 5;
        for _ in 0..RUNS {
            assert_eq!(generate(&b, levels), 1);
        }
        let record = t0.elapsed() / RUNS;
        println!(
            "{w}x{h}: {levels} levels, at most {chain_diff} LSB from the CPU chain over the exactly halving levels; \
             CPU box filter {:.3} ms, the GPU path's draws recorded in {:.3} ms",
            cpu.took.as_secs_f64() * 1e3,
            record.as_secs_f64() * 1e3
        );
    });
}

/// Every level above 0 of `t`'s storage against its reference, dEQP's way
/// (glsTextureTestUtil compareGenMipmapResult: level n's reference is made
/// from level n-1 OF THE RESULT, so a level is judged on its own filter and
/// not on the drift of the levels before it): within 1 LSB of the quad's
/// filter of the level above -- the box where the step halves exactly, the
/// bilinear sample where it does not. The GPU computes the texel centre in
/// float, so a box average that lands on .5 may round either way: that is
/// the 1 LSB. Level 1 is made from `level0`, so it is also within 1 LSB of
/// the GL core's own CPU box filter (`cpu`, the chain of the same `level0`);
/// the levels after it are compared with the CPU chain for the record only
/// (the two drift apart by a few LSB over ten levels, each rounding its own
/// way): the largest difference over the exactly halving levels is returned.
/// Level 0 must be `level0` still.
fn check_gpu_levels(t: &GlTexture, level0: &[u8], cpu: &CpuChain) -> u8 {
    let (w, h) = (t.width, t.height);
    let (mut sw, mut sh) = (w, h);
    let mut prev = level0.to_vec();
    let mut box_holds = true;
    let mut chain_diff = 0u8;
    for l in 1..u32::from(t.chain_levels) {
        let (dw, dh) = ((w >> l).max(1), (h >> l).max(1));
        let reference = filtered(&prev, sw, sh, dw, dh);
        let got = level_rows(t, l);
        let d = max_diff(&got, &reference);
        assert!(d <= 1, "{w}x{h} level {l} ({dw}x{dh}): {d} LSB from the quad's filter of level {}", l - 1);
        box_holds &= halves(sw, sh, dw, dh);
        if l == 1 && box_holds {
            let d_cpu = max_diff(&got, &cpu.levels[1]);
            assert!(d_cpu <= 1, "{w}x{h} level 1 ({dw}x{dh}): {d_cpu} LSB from the CPU box filter");
        }
        if box_holds {
            chain_diff = chain_diff.max(max_diff(&got, &cpu.levels[l as usize]));
        }
        (sw, sh, prev) = (dw, dh, got);
    }
    /* level 0 is nobody's to touch */
    assert_eq!(level_rows(t, 0), level0, "{w}x{h}: level 0 untouched");
    chain_diff
}

/* ---- a draw into a texture: what glmark2 does before its glGenerateMipmap ---------- */

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

const RENDER_VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const RENDER_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 1.0); }\n";
static RENDER_QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// A framebuffer with level 0 of texture `tex` as its colour, bound, and a
/// program that draws one flat quad, in use.
fn render_target(tex: GLuint) {
    // SAFETY: plain GL calls; RENDER_QUAD is a static.
    unsafe {
        let mut fb: GLuint = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "the texture's framebuffer");
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, RENDER_VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, RENDER_FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, RENDER_QUAD.as_ptr() as *const c_void);
    }
    no_error("the render target");
}

/// One draw into the bound framebuffer's texture, then glFinish: the kick
/// is out before the chain's (the stand-in runs no model on it, so level 0
/// keeps the texels it held, in the pixel engine's order from the first
/// draw on: `gl::texture::take_pe_order`).
fn render_into(w: u32, h: u32) {
    // SAFETY: plain GL calls.
    unsafe {
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glFinish();
    }
    no_error("the draw into the texture");
}

/// `level0` as the pixel engine holds it: B,G,R,A.
fn swap_red_blue(level0: &[u8]) -> Vec<u8> {
    level0.chunks(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect()
}

/// glmark2's frame on the GC7000 (terrain's and refract's
/// `update_mipmap`): a draw into level 0 of an RGBA8 texture through a
/// framebuffer, then glGenerateMipmap, each frame. The texture takes the
/// pixel engine's order at the first draw (`pe_order`) and its chain sits
/// at level 0's pitch (`chain_pitch`); the GL core offers that chain to
/// the back end, which draws it: the chain is the back end's
/// (`chain_in_backend`), counted, one kick per level above 0, and every
/// level within 1 LSB of the quad's filter of the level above (level 1 of
/// the GL core's CPU box filter too). Frame 1 grows a NPOT texture's
/// storage to a chain (`grow_npot_chain`) and makes it; frame 2, on the
/// same storage, is checked level by level.
fn rendered_into_case(w: u32, h: u32, seed: u32) {
    let level0 = swap_red_blue(&texels(w, h, seed));
    /* the reference: the CPU box filter treats every channel alike, so the
     * chain of the B,G,R,A texels is the B,G,R,A of the chain */
    let cpu = cpu_chain(w, h, &level0);
    let levels = cpu.levels.len() as u32;

    with_backend(&yakogl_backend_vivante, || {
        let tex = upload(w, h, &texels(w, h, seed));
        render_target(tex);
        let (made0, refused0) = mip_counts();
        for frame in 1..=2 {
            render_into(w, h);
            let t = texture(w, h);
            assert!(t.pe_order, "{w}x{h} frame {frame}: a texture a draw rendered into holds B,G,R,A");
            assert_eq!(level_rows(t, 0), level0, "{w}x{h} frame {frame}: level 0 in the pixel engine's order");
            if frame == 2 {
                /* the storage the chain is on, known since frame 1 */
                assert_ne!(t.chain_pitch, 0, "{w}x{h}: the Vivante chain is at level 0's pitch");
                assert_eq!(&t.level_off[..levels as usize], &cpu.level_off[..], "{w}x{h}: the same level offsets");
                scramble_chain(t);
                let plan_levels: Vec<(u32, u32, u32)> =
                    (0..levels).map(|l| ((w >> l).max(1), (h >> l).max(1), t.level_off[l as usize])).collect();
                let stride = t.level_pitch(t.width);
                with_platform(|p| p.plan = Some(Plan { base: t.mem.gpu, stride, levels: plan_levels, next: 1 }));
            }
            let kicks0 = with_platform(|p| p.kicks.len());
            // SAFETY: a plain GL call.
            unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
            no_error("glGenerateMipmap");
            let t = texture(w, h);
            assert!(t.chain_in_backend, "{w}x{h} frame {frame}: the GL core offered the chain and the GPU made it");
            assert_eq!(u32::from(t.chain_levels), levels, "{w}x{h} frame {frame}: GL's whole chain");
            assert_eq!(mip_counts(), (made0 + frame, refused0), "{w}x{h} frame {frame}: the chain is counted");
            let (kicks, drawn, faults) = with_platform(|p| {
                (p.kicks.len() - kicks0, p.plan.take().map_or(0, |pl| pl.next), core::mem::take(&mut p.faults))
            });
            assert!(faults.is_empty(), "{w}x{h} frame {frame}: the model found the streams wrong: {faults:#?}");
            assert_eq!(kicks, levels as usize - 1, "{w}x{h} frame {frame}: one kick per level above 0");
            if frame == 2 {
                assert_eq!(drawn, levels as usize, "{w}x{h}: every level drawn, in order");
                let chain_diff = check_gpu_levels(t, &level0, &cpu);
                println!(
                    "{w}x{h}, rendered into: {levels} levels made by the GPU through glGenerateMipmap, at most \
                     {chain_diff} LSB from the CPU chain over the exactly halving levels"
                );
            }
        }
    });
}

/// #1106: glmark2's terrain (800x600) and refract (1600x1200) targets, and
/// a power-of-two one, rendered into and then glGenerateMipmap'd through
/// the GL core: the GPU makes the chain, not the CPU box filter.
#[test]
fn glmark2s_rendered_into_chains_are_offered_to_the_gpu() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    rendered_into_case(64, 64, 9);
    rendered_into_case(800, 600, 10);
    rendered_into_case(1600, 1200, 11);
}

#[test]
fn a_power_of_two_chain_is_the_cpu_box_filters_within_1_lsb() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    chain_case(64, 64, 1);
    chain_case(32, 64, 2);
    chain_case(64, 1, 3);
}

/// glmark2's terrain target (800x600, ten levels) and refract's
/// (1600x1200, eleven): non-power-of-two chains, every level at the chain's
/// pitch; the steps that halve exactly are the box filter's, the odd ones
/// (75 -> 37, 25 -> 12, 9 -> 4, 3 -> 1) the bilinear sample's.
#[test]
fn glmark2s_npot_chains_are_drawn_level_by_level() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    chain_case(800, 600, 4);
    chain_case(1600, 1200, 5);
    chain_case(5, 3, 6);
}

/// What stays the GL core's CPU filter, each refused before any kick and
/// counted: another format, a cube face stack, one level, a stride
/// narrower than the image, a chain past the 32-bit address space. Either
/// channel order is made.
#[test]
fn chains_the_gpu_cannot_make_are_left_to_the_cpu_by_name() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(&yakogl_backend_vivante, || {
        upload_and_filter(64, 64, &texels(64, 64, 7));
        let ok = bind_of(texture(64, 64));
        assert_eq!((ok.format, ok.mip_levels), (GLES_TEXFMT_RGBA8, 7));
        let l8 = GlesTextureBind { format: GLES_TEXFMT_L8, ..ok };
        let cube = GlesTextureBind { type_: GLES_TEXTYPE_CUBE, layers: 6, ..ok };
        let narrow = GlesTextureBind { stride_texels: 63, ..ok };
        let past = GlesTextureBind { base_gpu: 0xffff_f000, ..ok };
        let cases: [(&str, &GlesTextureBind, u32); 5] = [
            ("l8", &l8, 7),
            ("cube", &cube, 7),
            ("one level", &ok, 1),
            ("narrow", &narrow, 7),
            ("past 4 GiB", &past, 7),
        ];
        let kicks0 = with_platform(|p| p.kicks.len());
        let (made0, refused0) = mip_counts();
        for (i, (what, b, levels)) in cases.iter().enumerate() {
            assert_eq!(generate(b, *levels), 0, "{what}");
            assert_eq!(with_platform(|p| p.kicks.len()), kicks0, "{what}: no kick");
            assert_eq!(mip_counts(), (made0, refused0 + i as u32 + 1), "{what}: counted");
        }
        let bgra = GlesTextureBind { bgra: 1, ..ok };
        assert_eq!(generate(&bgra, 7), 1, "a texture a draw rendered into (B,G,R,A) is made");
        assert_eq!(generate(&ok, 7), 1, "an uploaded one (R,G,B,A) too");
        assert_eq!(mip_counts(), (made0 + 2, refused0 + 5));
        assert_eq!(with_platform(|p| p.kicks.len()), kicks0 + 12, "six kicks each");
    });
}

/// The GL core about to read or write a GPU-made chain's texels on the CPU
/// (`return_mipmap_chain`): the chain's last kick is waited for, once; a
/// chain already complete waits for nothing.
#[test]
fn asking_the_chain_back_waits_for_its_last_kick() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(&yakogl_backend_vivante, || {
        let back =
            yakogl_backend_vivante.return_mipmap_chain.expect("the Vivante back end answers return_mipmap_chain");
        upload_and_filter(64, 64, &texels(64, 64, 8));
        let b = bind_of(texture(64, 64));
        with_platform(|p| p.waits.clear());
        // SAFETY: a valid binding for the call.
        unsafe { back(&b, 7) };
        /* glGenerateMipmap's chain (the GL core offered it) is complete: the
         * stand-in completes every kick at once */
        assert!(with_platform(|p| p.waits.is_empty()), "a complete chain: nothing to wait for");
        let fence0 = with_platform(|p| p.fence);
        assert_eq!(generate(&b, 7), 1);
        /* the chain's kicks stay in flight until waited for */
        with_platform(|p| p.hold = p.fence - fence0);
        let last = with_platform(|p| p.fence);
        assert_eq!(last - fence0, 6, "six level kicks");
        let before = with_platform(|p| p.waits.len());
        // SAFETY: as above.
        unsafe { back(&b, 7) };
        let waits = with_platform(|p| p.waits.clone());
        assert_eq!(waits.len(), before + 1, "{waits:?}");
        assert_eq!(waits.last(), Some(&last), "the chain's last kick");
        // SAFETY: as above.
        unsafe { back(&b, 7) };
        assert_eq!(with_platform(|p| p.waits.len()), before + 1, "complete: no second wait");
    });
}
