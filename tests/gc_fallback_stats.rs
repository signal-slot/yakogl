// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): `yakogl_gc_fallback_stats`, the library's answer to
//! glmark2's PERF page 4 (driver/samples/zephyr/glmark2/src/
//! gm_gc_fallback.h), counts the GPU mipmap chains, the glGenerateMipmap
//! calls left to the CPU filter and the bytes the CPU tiled copy and the CPU
//! filter move over the carve-out, since boot: a second `gc_init` resets
//! `gc_backend_mip_chains` but not these (plan #328, item fb-counters).
//!
//! A GPU chain is glGenerateMipmap of an uploaded RGBA8 texture (as in
//! gc_mipmap_gpu.rs); a CPU-filtered call is the back end's
//! `generate_mipmap` refusing a texture that is not RGBA8; a CPU tiled copy
//! is a vertex-stage sample of an uploaded texture (gc_tiled_copy_reuse.rs,
//! whose stand-in OSAL this is).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_fallback_stats
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits ---------
 *
 * The same stand-in as tests/gc_npot_repeat.rs.
 */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
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

/// `n` bytes of the stand-in's memory at GPU address `gpu`.
fn read_gpu(gpu: u64, n: usize) -> Vec<u8> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back().unwrap_or_else(|| panic!("no allocation at 0x{gpu:x}"));
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x}+{n} runs off the allocation at 0x{base:x} ({} bytes)", a.size);
        // SAFETY: inside the allocation (checked just above).
        unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
    })
}

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v));
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

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

/* ---- GL plumbing -------------------------------------------------------------------- */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

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

fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/// The vertex stage samples `s` (as terrain's displacement and normal maps);
/// the fragment stage samples nothing, so the vertex sampler is slot 0.
const VS: &str = "attribute vec4 p;\nuniform sampler2D s;\nvarying vec4 c;\nvoid main() { c = texture2D(s, vec2(0.5, 0.5)); gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvarying vec4 c;\nvoid main() { gl_FragColor = c; }\n";

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

const SIZE: u32 = 16;

/// One quad whose vertex shader samples the bound texture, then the kick:
/// returns the level 0 address of the descriptor in sampler slot 0.
fn draw_sampling() -> u64 {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = program(VS, FS);
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    no_error("glFinish");
    let stream = with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the quad produced no kick");
        p.kicks.last().cloned().unwrap()
    });
    let w = writes(&stream);
    let desc_gpu = w
        .iter()
        .rev()
        .find(|(r, _)| *r == nte_descriptor_addr(0))
        .map(|(_, v)| u64::from(*v))
        .expect("NTE_DESCRIPTOR_ADDR(0) was never written");
    assert_ne!(desc_gpu, 0, "no descriptor for slot 0");
    let d = read_gpu(desc_gpu, 4);
    u64::from(u32::from_le_bytes(d.try_into().unwrap()))
}

/// Texel (x, y) of the texture as the test uploads it, `salt` telling uploads apart.
fn texel(x: u32, y: u32, salt: u32) -> [u8; 4] {
    [x as u8, y as u8, salt as u8, 0xff]
}

fn image(salt: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            v.extend_from_slice(&texel(x, y, salt));
        }
    }
    v
}

/// A 16x16 RGBA8 texture, one level, NEAREST, bound to unit 0.
fn make_texture(salt: u32) -> GLuint {
    let mut t: GLuint = 0;
    // SAFETY: one name; plain GL calls; the image holds 16x16 RGBA texels.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            SIZE as GLint,
            SIZE as GLint,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            image(salt).as_ptr().cast(),
        );
    }
    no_error("make texture");
    t
}

/* ---- the counters ------------------------------------------------------------------- */

use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_RGB565, GLES_TEXTYPE_2D};
use yakogl::vivante::mipmap::{yakogl_gc_fallback_stats, YakoglGcFallbackStats};

/// The getter's answer, which must be 0.
fn stats() -> YakoglGcFallbackStats {
    let mut s = YakoglGcFallbackStats::default();
    // SAFETY: a valid out-pointer.
    assert_eq!(unsafe { yakogl_gc_fallback_stats(&mut s) }, 0, "yakogl_gc_fallback_stats");
    s
}

/// `after - before`, field by field.
fn delta(before: YakoglGcFallbackStats, after: YakoglGcFallbackStats) -> YakoglGcFallbackStats {
    YakoglGcFallbackStats {
        gpu_mip_chains: after.gpu_mip_chains - before.gpu_mip_chains,
        cpu_mip_calls: after.cpu_mip_calls - before.cpu_mip_calls,
        gpu_waits: after.gpu_waits - before.gpu_waits,
        carveout_cpu_bytes: after.carveout_cpu_bytes - before.carveout_cpu_bytes,
    }
}

/// (made, refused) since the last `gc_init`.
fn mip_counts() -> (u32, u32) {
    extern "C" {
        fn gc_backend_mip_chains(made: *mut u32, refused: *mut u32);
    }
    let (mut made, mut refused) = (0u32, 0u32);
    // SAFETY: two valid u32 out-pointers.
    unsafe { gc_backend_mip_chains(&mut made, &mut refused) };
    (made, refused)
}

/// glGenerateMipmap of a 64x64 RGBA8 texture uploaded with glTexImage2D:
/// the GPU chain.
fn gpu_chain() {
    let mut tex: GLuint = 0;
    let level0 = vec![0x5au8; 64 * 64 * 4];
    // SAFETY: plain GL calls; `level0` holds 64x64 RGBA texels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, level0.as_ptr().cast());
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    no_error("glGenerateMipmap");
}

/// The back end's `generate_mipmap` asked for the 7-level chain of a 64x64
/// RGB565 texture: refused (not RGBA8), the GL core's CPU filter's call.
fn cpu_mip_call() {
    let b = GlesTextureBind {
        format: GLES_TEXFMT_RGB565,
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: 0x10_0000,
        width: 64,
        height: 64,
        stride_texels: 64,
        bytes: 64 * 64 * 2,
        mip_levels: 7,
        ..GlesTextureBind::ZERO
    };
    let f = yakogl::vivante::backend::yakogl_backend_vivante
        .generate_mipmap
        .expect("the Vivante back end answers generate_mipmap");
    // SAFETY: a valid binding for the call.
    assert_eq!(unsafe { f(&b, 7) }, 0, "an RGB565 chain is left to the CPU");
}

/// What the CPU filter moves for that chain: level l - 1 read and level l
/// written, 2 bytes a texel, 64x64 down to 1x1.
const CPU_MIP_BYTES: u64 = {
    let mut s = 64u64;
    let mut n = 0u64;
    while s > 1 {
        n += (s * s + (s / 2) * (s / 2)) * 2;
        s /= 2;
    }
    n
};

#[test]
fn the_getter_answers_0_and_rejects_a_null_pointer() {
    on_gc7000(|| {
        stats();
        // SAFETY: null is allowed by the contract.
        assert_eq!(unsafe { yakogl_gc_fallback_stats(core::ptr::null_mut()) }, -22);
    });
}

#[test]
fn mip_chains_cpu_mip_calls_and_tiled_copy_bytes_count_since_boot() {
    let mut end_of_first = YakoglGcFallbackStats::default();
    on_gc7000(|| {
        let s0 = stats();
        let m0 = mip_counts();

        gpu_chain();
        assert_eq!(mip_counts(), (m0.0 + 1, m0.1), "the GL core's offer reached the GPU chain");
        let s1 = stats();
        assert_eq!(
            delta(s0, s1),
            YakoglGcFallbackStats { gpu_mip_chains: 1, ..Default::default() },
            "a GPU chain: one chain, no CPU bytes"
        );

        cpu_mip_call();
        let s2 = stats();
        assert_eq!(
            delta(s1, s2),
            YakoglGcFallbackStats { cpu_mip_calls: 1, carveout_cpu_bytes: CPU_MIP_BYTES, ..Default::default() },
            "a CPU-filtered call: the call and the filter's bytes"
        );

        /* the vertex stage samples an uploaded 16x16 texture: the CPU tiled
         * copy reads the storage, zero-fills the 16x16 copy and writes it */
        make_texture(1);
        let stride = {
            let c = yakogl::gl::context::current().expect("a current context");
            let t = c
                .objs_ref()
                .textures
                .iter()
                .find(|t| t.used && t.width == SIZE && t.height == SIZE)
                .expect("the 16x16 texture");
            t.level_pitch(SIZE)
        };
        draw_sampling();
        let s3 = stats();
        let copy = u64::from(SIZE * SIZE * 4);
        assert_eq!(
            delta(s2, s3),
            YakoglGcFallbackStats { carveout_cpu_bytes: u64::from(stride * SIZE * 4) + 2 * copy, ..Default::default() },
            "a CPU tiled copy: its bytes only"
        );
        /* an unchanged texture keeps its copy: no new pass */
        draw_sampling();
        assert_eq!(stats(), s3, "a draw reusing the copy moved no byte");
        end_of_first = s3;
    });

    on_gc7000(|| {
        assert_eq!(mip_counts(), (0, 0), "gc_init resets the per-init counts (gc_glmark2_mip_offer.rs relies on it)");
        assert_eq!(stats(), end_of_first, "the since-boot counts survive a second gc_init");
        gpu_chain();
        assert_eq!(stats().gpu_mip_chains, end_of_first.gpu_mip_chains + 1);
        assert_eq!(stats().gpu_waits, 0, "nothing counts the waits yet");
    });
}
