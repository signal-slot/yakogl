// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): object lifetime around a two-plane RGBA32 binding
//! (docs/notes/gc-128bit-texture-sampling.md; Mesa 26.2.1 etnaviv).
//!
//! A bound RGBA32F texture makes the draw's fragment shader a plane variant
//! and puts a companion descriptor in the next sampler slot. What happens to
//! that when the texture goes away:
//!
//! - (a) `glDeleteTextures` on the bound texture, then a draw: the unit is
//!   texture 0 again, nothing is keyed, no companion descriptor is written,
//!   GL_NO_ERROR.
//! - (b) a delete between two draws of one frame: the first draw's pair of
//!   descriptors stays as it was emitted in the stream, the second draw is
//!   not keyed.
//! - (c) a texture deleted on another unit than the one the program samples
//!   leaves no companion slot behind.
//! - (d) the name recreated as RGBA8 gives an unkeyed draw.
//!
//! Respecification, unit switches and relinking are item gc-w128-respec's.
//! The sampled mask hook that opens the
//! 128-bit classes (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so no
//! integration test can add them: until GC_SAMPLED_TEXFMTS holds RGBA32F
//! (item gc-w128-flip) these find the class closed, say so on stderr and
//! check the closed state (nothing is keyed); from the flip on the same
//! tests check the open one, with the state found at run time.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_lifetime_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
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
use yakogl::vivante::texture::wide128_sampled;

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

/* ---- the stand-in OSAL (as tests/gc_tiled_copy_reuse.rs) ---------------- */

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

fn words_at(gpu: u64, n: usize) -> Vec<u32> {
    let b = read_gpu(gpu, n * 4);
    let (words, _) = b.as_chunks::<4>();
    words.iter().map(|c| u32::from_le_bytes(*c)).collect()
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

/* ---- GL plumbing -------------------------------------------------------- */

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

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// A program sampling unit 0 (`s`) and, when `two`, unit 1 (`t`) as well.
fn program(two: bool) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let (decl, body) = if two {
        ("uniform sampler2D s;\nuniform sampler2D t;\n", "texture(s, vec2(0.5)) + texture(t, vec2(0.5))")
    } else {
        ("uniform sampler2D s;\n", "texture(s, vec2(0.5))")
    };
    let fs = format!("#version 300 es\nprecision highp float;\n{decl}out vec4 o;\nvoid main() {{ o = {body}; }}\n");
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        if two {
            glUniform1i(glGetUniformLocation(p, c"t".as_ptr()), 1);
        }
        p
    }
}

/// A 2x2 nearest-filtered texture of `internal` (RGBA32F or RGBA8), bound to
/// the active unit.
fn texture(internal: GLenum) -> GLuint {
    let floats: Vec<u8> =
        (0..4).flat_map(|i| [1.0f32 + i as f32, 2.0, 3.0, 4.0].into_iter().flat_map(|f| f.to_le_bytes())).collect();
    let bytes = [0x40u8; 16];
    // SAFETY: plain GL calls; the images hold 2x2 texels of the type named.
    unsafe {
        let mut t: GLuint = 0;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        if internal == GL_RGBA32F {
            glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 2, 2, 0, GL_RGBA, GL_FLOAT, floats.as_ptr().cast());
        } else {
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                internal as GLint,
                2,
                2,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                bytes.as_ptr().cast(),
            );
        }
        t
    }
}

fn delete(t: GLuint) {
    // SAFETY: plain GL call; one name.
    unsafe { glDeleteTextures(1, &t) };
    no_error("glDeleteTextures");
}

fn active(unit: u32) {
    // SAFETY: plain GL call.
    unsafe { glActiveTexture(GL_TEXTURE0 + unit) };
}

fn bound_2d() -> GLint {
    let mut b: GLint = -1;
    // SAFETY: plain GL call; one int out.
    unsafe { glGetIntegerv(GL_TEXTURE_BINDING_2D, &mut b) };
    b
}

fn quad() {
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
}

/// The register writes before each draw op of a stream (state is per draw).
fn per_draw(stream: &[u32]) -> Vec<Vec<(u32, u32)>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut i = 0usize;
    while i < stream.len() {
        let w = stream[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    if let Some(v) = stream.get(i + 1 + k) {
                        cur.push((base + 4 * k as u32, *v));
                    }
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                out.push(core::mem::take(&mut cur));
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

/// glFinish, and the state writes of each draw in the kick it produced.
fn finish() -> Vec<Vec<(u32, u32)>> {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    no_error("glFinish");
    let stream = with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the frame produced no kick");
        p.kicks.last().cloned().unwrap()
    });
    per_draw(&stream)
}

/// Every value one draw's state wrote to `reg`, in order.
fn all(draw: &[(u32, u32)], reg: u32) -> Vec<u32> {
    draw.iter().filter(|(r, _)| *r == reg).map(|(_, v)| *v).collect()
}

/// Did the draw write a companion descriptor into slot 1? (With only unit 0
/// sampled, slot 1 has no other use.)
fn keyed(draw: &[(u32, u32)]) -> bool {
    all(draw, nte_descriptor_addr(1)).iter().any(|a| *a != 0)
}

/// RGBA32F is in the sampled mask: the binding must be keyed. Closed: say so
/// and pin that nothing is keyed. Returns whether the class is open.
fn expect_bound_state(draw: &[(u32, u32)], what: &str) -> bool {
    if open(GLES_TEXFMT_RGBA32F) {
        assert!(keyed(draw), "{what}: an RGBA32F binding is keyed (a companion descriptor in slot 1)");
        true
    } else {
        eprintln!("{what}: RGBA32F is not in GC_SAMPLED_TEXFMTS; the plane key is not exercised (gc-w128-flip)");
        assert!(!keyed(draw), "{what}: closed class: no companion descriptor");
        false
    }
}

fn closed_is_closed() {
    if !open(GLES_TEXFMT_RGBA32F) {
        let t = GlesTextureBind {
            format: GLES_TEXFMT_RGBA32F,
            type_: GLES_TEXTYPE_2D,
            width: 2,
            height: 2,
            layers: 1,
            mip_levels: 1,
            ..Default::default()
        };
        assert!(!wide128_sampled(&t));
    }
}

/* ---- (a) delete, then draw ----------------------------------------------- */

#[test]
fn a_deleted_bound_texture_leaves_an_unkeyed_draw() {
    closed_is_closed();
    on_gc7000(|| {
        program(false);
        let t = texture(GL_RGBA32F);
        no_error("upload");
        quad();
        let d = finish();
        expect_bound_state(&d[0], "before the delete");
        assert!(all(&d[0], nte_descriptor_addr(0)).iter().any(|a| *a != 0), "slot 0 has a descriptor");

        delete(t);
        assert_eq!(bound_2d(), 0, "the unit falls back to texture 0");
        quad();
        let d = finish();
        assert_eq!(d.len(), 1);
        assert!(!keyed(&d[0]), "after the delete: unkeyed, no companion descriptor");
    });
}

/* ---- (b) delete between two draws of one frame ---------------------------- */

#[test]
fn a_delete_inside_a_frame_keeps_the_first_draws_descriptors() {
    closed_is_closed();
    on_gc7000(|| {
        program(false);
        let t = texture(GL_RGBA32F);
        quad();
        delete(t);
        quad();
        let d = finish();
        assert_eq!(d.len(), 2, "two draws in one frame");
        if expect_bound_state(&d[0], "the first draw") {
            let a0 = *all(&d[0], nte_descriptor_addr(0)).last().unwrap();
            let a1 = *all(&d[0], nte_descriptor_addr(1)).last().unwrap();
            assert_ne!(a0, a1, "two descriptors, not one");
            /* the emitted pair still reads as a pair after the delete */
            let d0 = words_at(u64::from(a0), GC_TEXDESC_BYTES / 4);
            let d1 = words_at(u64::from(a1), GC_TEXDESC_BYTES / 4);
            for w in [GC_TEXDESC_SIZE, GC_TEXDESC_BASELOD, GC_TEXDESC_CONFIG1] {
                assert_eq!(d0[w as usize / 4], d1[w as usize / 4], "word 0x{w:x}: the pair is a copy");
            }
            assert_ne!(d0[texdesc_lod_addr(0) as usize / 4], 0, "plane 0 has an address");
            assert_ne!(d1[texdesc_lod_addr(0) as usize / 4], 0, "plane 1 has an address");
        }
        assert!(!keyed(&d[1]), "the second draw is not keyed");
    });
}

/* ---- (c) a delete on another unit ----------------------------------------- */

#[test]
fn a_texture_deleted_on_another_unit_leaves_no_companion_slot() {
    closed_is_closed();
    on_gc7000(|| {
        /* unit 1 holds the RGBA32F texture, unit 0 an RGBA8 one; the program samples unit 0 */
        active(1);
        let wide = texture(GL_RGBA32F);
        active(0);
        texture(GL_RGBA8);
        program(false);
        quad();
        let d = finish();
        assert!(!keyed(&d[0]), "a program that samples unit 0 alone is not keyed by unit 1's binding");

        active(1);
        delete(wide);
        assert_eq!(bound_2d(), 0, "unit 1 falls back to texture 0");
        active(0);
        quad();
        let d = finish();
        assert!(!keyed(&d[0]), "after the delete there is no companion slot");

        /* the program sampling both units: unit 1 now reads texture 0 */
        program(true);
        quad();
        let d = finish();
        assert!(
            !all(&d[0], nte_descriptor_addr(2)).iter().any(|a| *a != 0),
            "two-unit program: unit 1 is texture 0, so slot 2 holds no companion"
        );
    });
}

/* ---- (d) the name recreated ---------------------------------------------- */

#[test]
fn a_name_recreated_as_rgba8_gives_an_unkeyed_draw() {
    closed_is_closed();
    on_gc7000(|| {
        program(false);
        let t = texture(GL_RGBA32F);
        quad();
        let d = finish();
        expect_bound_state(&d[0], "the RGBA32F texture");

        delete(t);
        let px = [0x40u8; 16];
        // SAFETY: plain GL calls; binding the freed name creates the object anew.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr().cast());
        }
        no_error("recreate as RGBA8");
        quad();
        let d = finish();
        assert!(!keyed(&d[0]), "the same name as RGBA8: unkeyed");
        assert!(all(&d[0], nte_descriptor_addr(0)).iter().any(|a| *a != 0), "slot 0 has the RGBA8 descriptor");
    });
}
