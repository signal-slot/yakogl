// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the GPU mipmap chain stays in the open kick
//! (`gc_chain_open_kick`, `gc_debug_set_chain_open_kick`,
//! docs/notes/gc-kick-continue.md C5) beside the same chain with the knob
//! off, which submits the frame before the chain and the chain itself before
//! glGenerateMipmap returns (src/vivante/mipmap.rs `chain_gpu`).
//!
//! Two parts:
//!
//! 1. glmark2's terrain, shadow and refract frames through the shared
//!    harness (`common/gc_scenes.rs`, `gc_scene_kicks.rs`'s scenes and
//!    stand-in OSAL), the knob off and on, in both GPU models: the kicks a
//!    warm frame (the most over frames 3..=6), and what the streams hold.
//!    The kicks, as c5-chain-prep measured them before the code:
//!
//!    ```text
//!                 off   on
//!    terrain      11    1     (its chains no longer end the frame's kick)
//!    shadow        2    2     (no chain)
//!    refract       3    2     (the 11-level chain joins the 1600x1200 pass)
//!    ```
//!
//!    Measured here the same. The waits outside the swap in the `Lag` model
//!    go from terrain 11 and refract 2 to none. The draws and the
//!    resolve-engine jobs of a frame are the same with the knob on and off:
//!    the knob moves kick boundaries, not work.
//!
//! 2. A unit check over a stand-in OSAL of this file's own whose `submit`
//!    runs every mipmap quad draw of a stream as the GPU would (the model of
//!    `gc_mipmap_gpu.rs`, cut to what this test needs): glGenerateMipmap of a
//!    64x64 RGBA8 texture, then the chain asked back (`return_mipmap_chain`,
//!    what the GL core calls before a CPU read), then the levels read from
//!    the storage. With the knob on the chain is not submitted by
//!    glGenerateMipmap; asking it back submits its kick and waits for it,
//!    once. The levels are the same bytes with the knob on and off.
//!
//! The knob is set before the first context of each run and put back to off
//! after it; [`KNOB`] keeps another test of this file out meanwhile.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_chain_open_kick -- --nocapture
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;

use core::ffi::c_void;
use gc_scenes::{run_full, Counts, Gpu, Hooks, StreamCounts};
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::{GlesTextureBind, GLES_TEXTYPE_2D};
use yakogl::gl::consts::*;
use yakogl::gl::context::GlTexture;
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_debug_set_chain_open_kick, gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::internal::{GC_INTERNAL_MIPGEN_PS, GC_INTERNAL_TABLE};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/// Serialises the knob across this file's tests (they are threads of one
/// process, and the knob and the back end are process-global).
static KNOB: Mutex<()> = Mutex::new(());

/* ---- part 1: the scenes ---------------------------------------------------------------- */

/// One scene's frame, a GPU model and the knob: the most over the counted
/// frames, and a check that every counted frame is the same.
fn frame(on: bool, gpu: Gpu, scene: fn() -> Box<dyn FnMut()>) -> (Counts, StreamCounts) {
    let _k = KNOB.lock().unwrap_or_else(|e| e.into_inner());
    gc_debug_set_chain_open_kick(i32::from(on));
    let r = std::panic::catch_unwind(|| run_full(gpu, scene, Hooks::default()));
    gc_debug_set_chain_open_kick(0);
    let (frames, _, streams) = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(frames.windows(2).all(|w| w[0] == w[1]), "the counted frames differ: {frames:?}");
    assert!(streams.windows(2).all(|w| w[0] == w[1]), "the counted frames' streams differ: {streams:?}");
    (Counts::max_of(&frames), StreamCounts::max_of(&streams))
}

/// A scene's kicks a frame with the knob off and on, in both GPU models,
/// and its waits outside the swap in `Lag` (`waits`, off and on; `Done`
/// waits for nothing outside the swap either way); the streams hold the
/// same draws and RS jobs either way, and nothing calls `finish()` outside
/// the swap.
fn check(name: &str, scene: fn() -> Box<dyn FnMut()>, off: u32, on: u32, waits: (u32, u32)) {
    for gpu in [Gpu::Lag, Gpu::Done] {
        let (c_off, s_off) = frame(false, gpu, scene);
        let (c_on, s_on) = frame(true, gpu, scene);
        eprintln!("{name} {gpu:?} knob off: {c_off:?} {s_off:?}");
        eprintln!("{name} {gpu:?} knob on:  {c_on:?} {s_on:?}");
        assert_eq!(c_off.kicks, off, "{name} ({gpu:?}): kicks a frame, knob off");
        assert_eq!(c_on.kicks, on, "{name} ({gpu:?}): kicks a frame, knob on");
        assert_eq!(s_on, s_off, "{name} ({gpu:?}): the knob moves kick boundaries, not draws or RS jobs");
        assert_eq!((c_off.finishes_other, c_on.finishes_other), (0, 0), "{name} ({gpu:?}): no finish()");
        let want = if gpu == Gpu::Lag { waits } else { (0, 0) };
        assert_eq!((c_off.waits_other, c_on.waits_other), want, "{name} ({gpu:?}): waits outside the swap");
    }
}

/// terrain: its mipmap chains end the frame's kick with the knob off (11
/// kicks, and in `Lag` 11 waits outside the swap: each kick's slot waited
/// out); with it on the whole frame is the swap's one kick, and nothing
/// waits before the swap.
#[test]
fn terrain_chains_stay_in_the_open_kick() {
    check("terrain", gc_scenes::terrain, 11, 1, (11, 0));
}

/// shadow makes no chain: 2 kicks either way.
#[test]
fn shadow_is_unchanged_by_the_knob() {
    check("shadow", gc_scenes::shadow, 2, 2, (0, 0));
}

/// refract: its 11-level chain is a stream of its own with the knob off
/// (3 kicks, 2 waits outside the swap in `Lag`); with it on the chain stays
/// in the 1600x1200 pass's kick (2 kicks, no wait before the swap).
#[test]
fn refract_chain_joins_the_pass_before_it() {
    check("refract", gc_scenes::refract, 3, 2, (2, 0));
}

/* ---- part 2: the chain's texels, knob off and on ---------------------------------------- */

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
    allocs: BTreeMap<u64, (Box<[u64]>, usize)>,
    bump: BTreeMap<u64, u64>,
    /// The last fence handed out, and the last one completed: a kick
    /// completes when it is waited for (or a later one is).
    fence: i64,
    done: i64,
    kicks: u32,
    waits: Vec<i64>,
    /// The register file as the streams left it (the GPU keeps its state
    /// from one kick to the next).
    regs: BTreeMap<u32, u32>,
    plan: Option<Plan>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

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
        p.allocs.insert(gpu, (mem, size));
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        /* leaked, not dropped: a static of the library may still name the block */
        if let Some(b) = p.allocs.remove(&m.gpu) {
            core::mem::forget(b);
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// `n` bytes of the stand-in's memory at GPU address `gpu`.
fn read_gpu(p: &Platform, gpu: u64, n: usize) -> Result<Vec<u8>, String> {
    let (base, (mem, size)) =
        p.allocs.range(..=gpu).next_back().ok_or_else(|| format!("no allocation at 0x{gpu:x}"))?;
    let off = (gpu - base) as usize;
    if off + n > *size {
        return Err(format!("0x{gpu:x}+{n} runs off the allocation at 0x{base:x}"));
    }
    // SAFETY: inside the allocation (checked just above).
    Ok(unsafe { core::slice::from_raw_parts((mem.as_ptr() as *const u8).add(off), n) }.to_vec())
}

fn write_gpu(p: &mut Platform, gpu: u64, bytes: &[u8]) -> Result<(), String> {
    let (base, (mem, size)) =
        p.allocs.range_mut(..=gpu).next_back().ok_or_else(|| format!("no allocation at 0x{gpu:x}"))?;
    let off = (gpu - base) as usize;
    if off + bytes.len() > *size {
        return Err(format!("0x{gpu:x}+{} runs off the allocation at 0x{base:x}", bytes.len()));
    }
    // SAFETY: inside the allocation (checked just above).
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), (mem.as_mut_ptr() as *mut u8).add(off), bytes.len()) };
    Ok(())
}

/// Run the stream's draws as the GPU would: every draw whose fragment
/// program is the mipmap quad's is the plan's next level, a bilinear
/// CLAMP_TO_EDGE sample of the level above at the centre of each texel
/// (B,G,R,A in memory both ways, so the channels pass straight through).
fn run_stream(p: &mut Platform, words: &[u32]) -> Result<(), String> {
    let mipgen = &GC_INTERNAL_TABLE[GC_INTERNAL_MIPGEN_PS].shader;
    // SAFETY: a row of the generated table points at its `[u32; usc_dw]` static.
    let mipgen_words: &[u32] = unsafe { std::slice::from_raw_parts(mipgen.usc, mipgen.usc_dw as usize) };
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for (k, &v) in words.iter().skip(i + 1).take(count).enumerate() {
                    p.regs.insert(base + 4 * k as u32, v);
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                i += 4;
                let r = |reg: u32| p.regs.get(&reg).copied().ok_or(format!("register 0x{reg:x} never written"));
                let ps_at = u64::from(r(GC_PS_INST_ADDR)?);
                let ps: Vec<u32> = read_gpu(p, ps_at, mipgen_words.len() * 4)?
                    .chunks(4)
                    .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                    .collect();
                if ps != mipgen_words {
                    continue; /* not a mipmap quad: nothing this model draws */
                }
                let (target, desc_at) = (u64::from(r(pe_pipe_color_addr(0))?), u64::from(r(nte_descriptor_addr(0))?));
                let Some(plan) = p.plan.as_mut() else { return Err("a mipmap draw with no plan".into()) };
                let l = plan.next;
                if l == 0 || l >= plan.levels.len() {
                    return Err(format!("a draw past the chain's {} levels", plan.levels.len()));
                }
                plan.next += 1;
                let ((sw, sh, soff), (dw, dh, doff), base, stride) =
                    (plan.levels[l - 1], plan.levels[l], plan.base, plan.stride);
                if target != base + u64::from(doff) {
                    return Err(format!("level {l}: PE address {target:#x}, not {:#x}", base + u64::from(doff)));
                }
                let desc = read_gpu(p, desc_at, GC_TEXDESC_BYTES)?;
                let at = texdesc_lod_addr(0) as usize;
                let lod0 = u64::from(u32::from_le_bytes(desc[at..at + 4].try_into().unwrap()));
                if lod0 != base + u64::from(soff) {
                    return Err(format!("level {l}: source {lod0:#x}, not {:#x}", base + u64::from(soff)));
                }
                let src = read_gpu(p, lod0, (stride * sh * 4) as usize)?;
                let mut row = vec![0u8; (dw * 4) as usize];
                for y in 0..dh {
                    for x in 0..dw {
                        let u = (x as f32 + 0.5) * (1.0 / dw as f32) * sw as f32 - 0.5;
                        let v = (y as f32 + 0.5) * (1.0 / dh as f32) * sh as f32 - 0.5;
                        let (x0, y0) = (u.floor(), v.floor());
                        let (fx, fy) = (u - x0, v - y0);
                        let tx = |i: f32| (i.max(0.0) as u32).min(sw - 1);
                        let ty = |j: f32| (j.max(0.0) as u32).min(sh - 1);
                        let px = |i: u32, j: u32, ch: usize| src[((j * stride + i) * 4) as usize + ch] as f32;
                        for ch in 0..4 {
                            let top = px(tx(x0), ty(y0), ch) * (1.0 - fx) + px(tx(x0 + 1.0), ty(y0), ch) * fx;
                            let bot =
                                px(tx(x0), ty(y0 + 1.0), ch) * (1.0 - fx) + px(tx(x0 + 1.0), ty(y0 + 1.0), ch) * fx;
                            row[(x * 4) as usize + ch] =
                                ((top * (1.0 - fy) + bot * fy) + 0.5).floor().clamp(0.0, 255.0) as u8;
                        }
                    }
                    write_gpu(p, target + u64::from(y * stride * 4), &row)?;
                }
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    Ok(())
}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let words: Vec<u32> = match read_gpu(p, k.stream_gpu, k.stream_bytes as usize) {
            Ok(b) => b.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect(),
            Err(e) => {
                p.faults.push(e);
                return -22;
            }
        };
        if let Err(e) = run_stream(p, &words) {
            p.faults.push(e);
        }
        p.kicks += 1;
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    with_platform(|p| {
        p.waits.push(fence);
        p.done = p.done.max(fence.min(p.fence));
    });
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.done)
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

/// The `w` x `h` 2D texture of the current context.
fn texture(w: u32, h: u32) -> &'static GlTexture {
    let c = yakogl::gl::context::current().expect("a current context");
    c.objs_ref()
        .textures
        .iter()
        .find(|t| t.used && t.width == w && t.height == h && t.target == GL_TEXTURE_2D)
        .expect("the texture")
}

/// The texels of level `l` of `t`, `lw * 4` bytes a row, from the storage.
fn level_rows(t: &GlTexture, l: u32) -> Vec<u8> {
    let (lw, lh) = ((t.width >> l).max(1), (t.height >> l).max(1));
    let pitch = (t.level_pitch_at(l) * 4) as usize;
    let off = t.level_off[l as usize] as usize;
    // SAFETY: the texture's live storage, mem.size bytes.
    let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
    (0..lh as usize).flat_map(|y| mem[off + y * pitch..off + y * pitch + lw as usize * 4].iter().copied()).collect()
}

/// The binding the GL core hands the back end (`gl::draw::texture_bind_2d`).
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

/// What one run of the chain left: every level above 0, and the kicks and
/// waits seen around glGenerateMipmap and the two asks.
#[derive(Debug, PartialEq)]
struct ChainRun {
    levels: Vec<Vec<u8>>,
    /// Kicks glGenerateMipmap submitted; kicks and waits the first ask made;
    /// kicks and waits the second ask made.
    generate_kicks: u32,
    first_ask: (u32, usize),
    second_ask: (u32, usize),
}

/// glTexImage2D of a 64x64 RGBA8 texture, glGenerateMipmap, then the chain
/// asked back twice, with the knob as given, on the real back end over this
/// file's OSAL (the model drawing the levels at submit).
fn chain_run(on: bool) -> ChainRun {
    const W: u32 = 64;
    let level0 = texels(W, W, 5);
    let _k = KNOB.lock().unwrap_or_else(|e| e.into_inner());
    let _s = gc_scenes::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    gc_debug_set_chain_open_kick(i32::from(on));
    with_platform(|p| *p = Platform::default());
    // SAFETY: KNOB and SERIAL are held; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(W, W);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut tex: GLuint = 0;
        // SAFETY: plain GL calls; `level0` holds the W x W texels.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                W as GLsizei,
                W as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                level0.as_ptr() as *const c_void,
            );
        }
        no_error("glTexImage2D");
        let t = texture(W, W);
        let n = u32::from(t.chain_levels);
        assert_eq!(n, 7, "the whole chain in the storage");
        let plan_levels: Vec<(u32, u32, u32)> =
            (0..n).map(|l| ((W >> l).max(1), (W >> l).max(1), t.level_off[l as usize])).collect();
        let b0 = bind_of(t);
        with_platform(|p| {
            p.plan = Some(Plan { base: t.mem.gpu, stride: b0.stride_texels, levels: plan_levels, next: 1 })
        });
        let k0 = with_platform(|p| p.kicks);
        // SAFETY: a plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        no_error("glGenerateMipmap");
        let t = texture(W, W);
        assert!(t.chain_in_backend, "the GPU made the chain");
        let generate_kicks = with_platform(|p| p.kicks) - k0;
        let back =
            yakogl_backend_vivante.return_mipmap_chain.expect("the Vivante back end answers return_mipmap_chain");
        let b = bind_of(t);
        let ask = || {
            let (k, w) = with_platform(|p| (p.kicks, p.waits.len()));
            // SAFETY: a valid binding for the call.
            unsafe { back(&b, n) };
            with_platform(|p| (p.kicks - k, p.waits.len() - w))
        };
        let first_ask = ask();
        let second_ask = ask();
        let drawn = with_platform(|p| p.plan.as_ref().map_or(0, |pl| pl.next));
        assert_eq!(drawn, n as usize, "every level drawn, in order, by the time the chain is back");
        let levels: Vec<Vec<u8>> = (1..n).map(|l| level_rows(t, l)).collect();
        assert_eq!(level_rows(t, 0), level0, "level 0 untouched");
        ChainRun { levels, generate_kicks, first_ask, second_ask }
    }));
    gc_debug_set_chain_open_kick(0);
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    let faults = with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a);
        }
        core::mem::take(&mut p.faults)
    });
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract, or the model found a stream wrong: {faults:?}");
    r
}

/// The chain read back through `gc_return_mipmap_chain` is the same bytes
/// with the knob on and off. Off: glGenerateMipmap submits the chain's one
/// kick, the first ask waits for its fence, the second waits for nothing.
/// On: glGenerateMipmap submits nothing; the first ask submits the open kick
/// (the chain) and waits for it, once; the second finds it complete.
#[test]
fn the_chain_read_back_is_the_same_with_the_knob_on_and_off() {
    let off = chain_run(false);
    let on = chain_run(true);
    assert_eq!((off.generate_kicks, off.first_ask, off.second_ask), (1, (0, 1), (0, 0)), "knob off");
    assert_eq!((on.generate_kicks, on.first_ask, on.second_ask), (0, (1, 1), (0, 0)), "knob on");
    assert_eq!(on.levels, off.levels, "levels 1.. with the knob on and off");
    /* the model ran: level 1 is level 0's 2x2 box, within the 1 LSB a
     * bilinear sample at the centre of an exactly halving step rounds by */
    let level0 = texels(64, 64, 5);
    for y in 0..32usize {
        for x in 0..32usize {
            for ch in 0..4usize {
                let px = |i: usize, j: usize| f32::from(level0[(j * 64 + i) * 4 + ch]);
                let want =
                    (px(2 * x, 2 * y) + px(2 * x + 1, 2 * y) + px(2 * x, 2 * y + 1) + px(2 * x + 1, 2 * y + 1)) / 4.0;
                let got = f32::from(off.levels[0][(y * 32 + x) * 4 + ch]);
                assert!((got - want).abs() <= 1.0, "level 1 ({x},{y}) channel {ch}: {got} against the box's {want}");
            }
        }
    }
}
