// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the tiled copy of a texture the vertex stage samples is
//! made once while the texture is unchanged and made again after a write
//! (docs/notes/gc-terrain-tiled-copy.md).
//!
//! glmark2's terrain samples two 256x256 textures in its vertex shader (the
//! height map and the normal map) and renders into both every frame, so all
//! its copies are needed. This test pins the other half: `tiled_copy.rs` keys
//! a copy by the storage's `content_gen` and `target_kick`, so an unchanged
//! texture keeps its copy and a changed one gets a new one.
//!
//! The copy is seen through the submitted stream: the texture descriptor's
//! level 0 address of the draw is the copy's GPU address, and the stand-in
//! OSAL never reuses an address, so the same address is the same copy. The
//! texels are read from that address in the 4x4-tiled layout.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_tiled_copy_reuse
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
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

/// Texel (x, y) read from the 4x4-tiled copy at `gpu`: 4x4 tiles row-major,
/// texels row-major inside a tile.
fn tiled_texel(gpu: u64, x: u32, y: u32) -> [u8; 4] {
    let tile = (y / 4) * (SIZE / 4) + x / 4;
    let at = (tile * 16 + (y % 4) * 4 + x % 4) as u64 * 4;
    read_gpu(gpu + at, 4).try_into().unwrap()
}

/// Every texel of the copy at `gpu` against `want(x, y)`.
fn assert_copy(what: &str, gpu: u64, want: impl Fn(u32, u32) -> [u8; 4]) {
    for y in 0..SIZE {
        for x in 0..SIZE {
            assert_eq!(tiled_texel(gpu, x, y), want(x, y), "{what}: texel ({x}, {y}) of the copy at 0x{gpu:x}");
        }
    }
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

#[test]
fn an_unchanged_texture_keeps_its_copy_across_draws() {
    on_gc7000(|| {
        make_texture(1);
        let first = draw_sampling();
        assert_copy("first draw", first, |x, y| texel(x, y, 1));
        for n in 2..=4 {
            let again = draw_sampling();
            assert_eq!(again, first, "draw {n} of an unchanged texture made a new copy");
        }
        assert_copy("after four draws", first, |x, y| texel(x, y, 1));
    });
}

#[test]
fn a_texsubimage_write_makes_a_new_copy_with_the_new_data() {
    on_gc7000(|| {
        make_texture(1);
        let before = draw_sampling();
        assert_eq!(draw_sampling(), before, "the unchanged texture made a new copy");
        let patch = [0xaau8, 0xbb, 0xcc, 0xff];
        // SAFETY: a 1x1 RGBA texel at (5, 6) of the bound texture.
        unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 5, 6, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, patch.as_ptr().cast()) };
        no_error("glTexSubImage2D");
        let after = draw_sampling();
        assert_ne!(after, before, "a CPU write kept the old copy");
        assert_copy("after the write", after, |x, y| if (x, y) == (5, 6) { patch } else { texel(x, y, 1) });
        assert_eq!(draw_sampling(), after, "the copy of the written texture was made again with no new write");
    });
}

#[test]
fn a_render_into_the_texture_makes_a_new_copy() {
    on_gc7000(|| {
        let t = make_texture(1);
        let before = draw_sampling();
        assert_copy("before the render", before, |x, y| texel(x, y, 1));
        let mut fb: GLuint = 0;
        // SAFETY: plain GL calls; `t` is complete and renderable (RGBA8).
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glViewport(0, 0, SIZE as GLsizei, SIZE as GLsizei);
            glClearColor(0.5, 0.25, 0.125, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glFinish();
        }
        no_error("render into the texture");
        let after = draw_sampling();
        assert_ne!(after, before, "a render into the texture kept the old copy");
        /* The stand-in OSAL executes no stream, so the clear writes nothing and
         * the storage's bytes after the attach mean nothing: only the new copy
         * is checked here. The data of a new copy is checked by the TexSubImage
         * case. */
        assert_eq!(draw_sampling(), after, "the copy was made again with no new render");
    });
}
