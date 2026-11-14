// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): glGenerateMipmap on the 16-byte texture classes
//! (RGBA32F, RGB32F, RGBA32UI, RGBA32I; docs/notes/gc-128bit-texture-sampling.md).
//!
//! ES 3.0 3.8.14: only a colour-renderable, texture-filterable base level has
//! a generated chain, and none of these four is either (32-bit float needs
//! OES_texture_float_linear, the integer ones are never filterable). The GL
//! core answers GL_INVALID_OPERATION and leaves every level alone
//! (docs/es3/sized-formats.md; the `es3_formats` check of `glGenerateMipmap`),
//! before the back end is asked: so no GPU chain is drawn or refused
//! (`gc_backend_mip_chains`, the fallback stats) and the CPU box filter does
//! not run (`gl_mip_cpu_counters`). Pinned here for the four classes at 2D
//! sizes that are square, NPOT and 1 wide, with every level specified
//! beforehand, so "left alone" is a byte comparison of the whole storage.
//!
//! The classes must be in the GC sampled mask for the core to store them in
//! 16-byte texels. `backend::TEST_EXTRA_SAMPLED` is `#[cfg(test)]`, so an
//! integration test cannot add them (as tests/es3_wide128_planes_gc.rs): until
//! GC_SAMPLED_TEXFMTS holds them (item gc-w128-flip) a closed class is checked
//! on what the core does with it (an error, or the RGBA16F fallback) and
//! stderr says so; the INVALID_OPERATION and the unchanged counters are pinned
//! either way. From the flip on the storage is asserted to be 16-byte texels.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_mip_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::mipmap::{gc_backend_mip_chains, yakogl_gc_fallback_stats, YakoglGcFallbackStats};

/* ---- the stand-in OSAL (as tests/gc_tiled_copy_reuse.rs) ---------------- */

struct Alloc {
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
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
        p.allocs.insert(gpu, Alloc { mem });
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
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        p.kicks.push(words.to_vec());
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
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

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    let faults = with_platform(|p| {
        /* leaked, not freed, as in gc_npot_repeat.rs: addresses keep counting up */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- the pins ------------------------------------------------------------ */

/// (name, class, internal format, client format, client type, components)
type Class = (&'static str, u8, u32, GLenum, GLenum, usize);

const CLASSES: [Class; 4] = [
    ("RGBA32F", GLES_TEXFMT_RGBA32F, GL_RGBA32F, GL_RGBA, GL_FLOAT, 4),
    ("RGB32F", GLES_TEXFMT_RGBA32F, GL_RGB32F, GL_RGB, GL_FLOAT, 3),
    ("RGBA32UI", GLES_TEXFMT_RGBA32UI, GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, 4),
    ("RGBA32I", GLES_TEXFMT_RGBA32I, GL_RGBA32I, GL_RGBA_INTEGER, GL_INT, 4),
];

/// Square, NPOT and 1-wide / 1-high sizes.
const SIZES: [(u32, u32); 6] = [(8, 8), (13, 6), (5, 9), (1, 7), (7, 1), (1, 1)];

fn open(class: u8) -> bool {
    (sampled_texfmts() & gles_texfmt_bit(class) as u64) != 0
}

fn level_dims(w: u32, h: u32, l: u32) -> (u32, u32) {
    ((w >> l).max(1), (h >> l).max(1))
}

fn word(float: bool, level: u32, i: u32) -> u32 {
    let mut h = level.wrapping_mul(0x9e37_79b1) ^ i.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    if float {
        0x3f80_0000 | (h & 0x007f_ffff)
    } else {
        h
    }
}

/// Upload every level of the chain of `w`x`h` with distinct texels; the name.
fn upload(c: &Class, w: u32, h: u32) -> GLuint {
    let levels = 32 - w.max(h).leading_zeros();
    let float = c.4 == GL_FLOAT;
    // SAFETY: plain GL calls; each image holds lw*lh texels of c.5 words.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        for l in 0..levels {
            let (lw, lh) = level_dims(w, h, l);
            let img: Vec<u32> = (0..lw * lh * c.5 as u32).map(|i| word(float, l, i)).collect();
            glTexImage2D(
                GL_TEXTURE_2D,
                l as GLint,
                c.2 as GLint,
                lw as GLsizei,
                lh as GLsizei,
                0,
                c.3,
                c.4,
                img.as_ptr().cast(),
            );
        }
        t
    }
}

/// The whole storage of texture `name` and its stored class, if it has a CPU view.
fn storage(name: GLuint) -> Option<(u8, Vec<u8>)> {
    let ctx = yakogl::gl::context::current().expect("a current context");
    let slot = names::slot_of(ctx, Kind::Texture, name)?;
    let t = ctx.texture_object(slot)?;
    if t.mem.cpu.is_null() {
        return None;
    }
    // SAFETY: the texture's own CPU mapping, `size` bytes.
    Some((t.texfmt, unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) }.to_vec()))
}

/// (chains made, refused), the CPU filter's counters, (fallback gpu chains, cpu calls).
fn counters() -> ((u32, u32), (u64, u64, u64), (u64, u64)) {
    let (mut made, mut refused) = (0u32, 0u32);
    // SAFETY: two live outputs.
    unsafe { gc_backend_mip_chains(&mut made, &mut refused) };
    let mut s = YakoglGcFallbackStats::default();
    // SAFETY: a live output.
    assert_eq!(unsafe { yakogl_gc_fallback_stats(&mut s) }, 0);
    ((made, refused), gl_mip_cpu_counters(), (s.gpu_mip_chains, s.cpu_mip_calls))
}

/// glGenerateMipmap on each named class at every size: GL_INVALID_OPERATION,
/// the storage byte for byte as uploaded, and no chain made, refused, or
/// filtered.
fn check(only: &[&str]) {
    for c in CLASSES.iter().filter(|c| only.contains(&c.0)) {
        let is_open = open(c.1);
        if !is_open {
            eprintln!("{}: class {} is not in the GC sampled mask yet: checking the closed state", c.0, c.1);
        }
        for (w, h) in SIZES {
            // SAFETY: plain GL calls.
            while unsafe { glGetError() } != GL_NO_ERROR {}
            let name = upload(c, w, h);
            // SAFETY: plain GL call.
            let up_err = unsafe { glGetError() };
            let before = storage(name);
            if is_open {
                assert_eq!(up_err, GL_NO_ERROR, "{} {w}x{h}: upload", c.0);
                let (fmt, _) = before.as_ref().unwrap_or_else(|| panic!("{} {w}x{h}: a CPU view", c.0));
                assert_eq!(*fmt, c.1, "{} {w}x{h}: stored in its 16-byte class", c.0);
                assert_eq!(gles_texfmt_bytes(*fmt), 16);
            }
            let k0 = counters();
            // SAFETY: plain GL call.
            unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
            // SAFETY: plain GL call.
            assert_eq!(unsafe { glGetError() }, GL_INVALID_OPERATION, "{} {w}x{h}: glGenerateMipmap", c.0);
            assert!(before.map(|b| b.1) == storage(name).map(|a| a.1), "{} {w}x{h}: levels left alone", c.0);
            assert_eq!(counters(), k0, "{} {w}x{h}: no GPU chain drawn or refused, no CPU filter run", c.0);
            // SAFETY: a name made above.
            unsafe { glDeleteTextures(1, &name) };
        }
    }
}

/// RGB32F: 32-bit float is not texture-filterable (3.8.14), so
/// no chain, NPOT and 1-wide sizes included.
#[test]
fn float_32_has_no_generated_chain() {
    on_gc7000(|| check(&["RGB32F"]));
}

/// RGBA32UI and RGBA32I: an integer format is never filterable.
#[test]
fn integer_32_has_no_generated_chain() {
    on_gc7000(|| check(&["RGBA32UI", "RGBA32I"]));
}

/// The float box average of the 2x2 (or 1-wide / 1-high) source block: the
/// inputs are multiples of 1/4 below 2^10, so every sum and the quotient are
/// exact and the order of the additions does not matter.
fn float_box(src: &[u8], off: usize, pitch: usize, (x, y): (usize, usize), (sx, sy): (usize, usize), ch: usize) -> f32 {
    let mut sum = 0.0f32;
    for j in 0..sy {
        for i in 0..sx {
            let o = off + ((y * sy + j) * pitch + x * sx + i) * 16 + ch * 4;
            sum += f32::from_le_bytes(src[o..o + 4].try_into().unwrap());
        }
    }
    sum / (sx * sy) as f32
}

/// GL_RGBA32F with only level 0 given, on the sizes the CPU filter has odd
/// cases for: no GPU chain (`gc_backend_mip_chains` made and the fallback gpu count stay;
/// the refused count may move by one, `chain_gpu` refusing what is not RGBA8), the CPU filter runs once and its
/// levels 1.. are the float box averages in 16-byte texels, as a 2D RGBA8
/// texture's are bytes.
///
/// Held out until RGBA32F is stored as RGBA32F: `sized_format` maps GL_RGBA32F
/// to RGBA16F ("stored as halves") whatever the GC sampled mask holds (checked
/// with the three classes added to GC_SAMPLED_TEXFMTS), and the filter averages halves. The flip item
/// (gc-w128-flip) has to make both the mask and `sized_format` agree; drop the
/// `#[ignore]` with it. (ES 3.0 3.8.14 would make a sized GL_RGBA32F texture
/// GL_INVALID_OPERATION here, as RGB32F's is, `float_32_has_no_generated_chain`.)
#[test]
#[ignore = "GL_RGBA32F is stored as RGBA16F even with the three 128-bit classes in the GC mask (sized_format, texture.rs:6236): the CPU filter averages halves"]
fn rgba32f_chain_is_filtered_by_the_cpu_in_16_byte_texels() {
    on_gc7000(|| {
        assert!(open(GLES_TEXFMT_RGBA32F), "RGBA32F is in the GC sampled mask");
        for (w, h) in [(8u32, 8u32), (13, 6), (5, 9), (1, 7), (7, 1), (3, 3)] {
            let img: Vec<f32> = (0..w * h * 4).map(|i| ((i.wrapping_mul(2654435761) >> 22) as f32) * 0.25).collect();
            let mut name = 0;
            // SAFETY: plain GL calls; `img` holds w*h RGBA float texels.
            unsafe {
                while glGetError() != GL_NO_ERROR {}
                glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA32F as GLint,
                    w as GLsizei,
                    h as GLsizei,
                    0,
                    GL_RGBA,
                    GL_FLOAT,
                    img.as_ptr().cast(),
                );
                assert_eq!(glGetError(), GL_NO_ERROR, "{w}x{h} upload");
            }
            let k0 = counters();
            // SAFETY: plain GL call.
            unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
            // SAFETY: plain GL call.
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{w}x{h} glGenerateMipmap");
            let k1 = counters();
            assert_eq!(k1.0 .0, k0.0 .0, "{w}x{h}: no GPU chain made");
            assert!(k1.0 .1 <= k0.0 .1 + 1, "{w}x{h}: the GPU path refused it at most once");
            assert_eq!(k1.2 .0, k0.2 .0, "{w}x{h}: no GPU chain in the fallback stats");
            assert_eq!(k1.1 .2, k0.1 .2 + 1, "{w}x{h}: the CPU filter ran once");

            let ctx = yakogl::gl::context::current().expect("a current context");
            let slot = names::slot_of(ctx, Kind::Texture, name).expect("a texture name");
            let t = ctx.texture_object(slot).expect("a texture object");
            assert_eq!(t.texfmt, GLES_TEXFMT_RGBA32F, "{w}x{h}: 16-byte texels");
            // SAFETY: the texture's own CPU mapping.
            let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
            assert!(t.chain_levels >= 2, "{w}x{h}: a chain");
            let (mut lw, mut lh) = (w as usize, h as usize);
            for l in 1..t.chain_levels as usize {
                let (sx, sy) = (if lw > 1 { 2 } else { 1 }, if lh > 1 { 2 } else { 1 });
                let (dw, dh) = ((lw / 2).max(1), (lh / 2).max(1));
                let (sp, dp) = (t.level_pitch_at(l as u32 - 1) as usize, t.level_pitch_at(l as u32) as usize);
                let (so, dof) = (t.level_off[l - 1] as usize, t.level_off[l] as usize);
                for y in 0..dh {
                    for x in 0..dw {
                        for ch in 0..4 {
                            let want = float_box(mem, so, sp, (x, y), (sx, sy), ch);
                            let o = dof + (y * dp + x) * 16 + ch * 4;
                            let got = f32::from_le_bytes(mem[o..o + 4].try_into().unwrap());
                            assert_eq!(got, want, "{w}x{h} level {l} ({x},{y}) channel {ch}");
                        }
                    }
                }
                (lw, lh) = (dw, dh);
            }
            // SAFETY: a name made above.
            unsafe { glDeleteTextures(1, &name) };
        }
    });
}
