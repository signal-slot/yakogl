// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the lifetime of an RGBA32 texture around a two-plane
//! binding -- deleting or orphaning a bound RGBA32F/UI/I texture (item
//! gc-w128-lifetime; docs/notes/gc-128bit-texture-sampling.md).
//!
//! - (a) `glDeleteTextures` on a bound RGBA32 texture, then a draw: the unit
//!   falls back to texture 0 (`GL_TEXTURE_BINDING_2D` reads 0), the slot
//!   samples the back end's black texel, the draw has no plane key and no
//!   companion descriptor, and `GL_NO_ERROR`.
//! - (b) A draw, the delete and a second draw in one frame (no `glFinish`
//!   between them): the first draw's descriptors in the stream already built
//!   stay what they were -- the G32R32 pair with the texels in both planes
//!   once open -- and none of the memory they point at is given back to the
//!   OSAL before the kick that carries that draw; the second draw is not
//!   keyed.
//! - (c) An RGBA32F texture bound on a unit that is not the active one is
//!   deleted: that unit falls back to black and its companion slot goes; a
//!   second RGBA32F unit's companion moves down to the first companion slot
//!   and the slot the deleted unit's companion had is not written.
//! - (d) A name an RGBA32F texture had, bound again after the delete and
//!   given an RGBA8 image: an unkeyed draw, the RGBA8 descriptor as a fresh
//!   RGBA8 texture's, no companion.
//! - (e) Orphaning: the texture is attached to a framebuffer that is not
//!   bound when it is deleted, so the name goes while the image stays (the
//!   orphan rule in src/gl/texture.rs); the unit falls back as in (a), and
//!   deleting the framebuffer afterwards breaks no OSAL contract.
//!
//! Respecification, unit switches and relinking are item gc-w128-respec's.
//!
//! The sampled mask hook that opens the 128-bit classes
//! (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so no integration test
//! can add them. Which state applies is read at run time, from the class the
//! GL core stored (the descriptor of the bound draw), not from the mask: a
//! binding stored as a G32R32 plane must carry the plane key, a companion
//! and both planes' texels, and its class must be in `sampled_texfmts()`;
//! one that is not must be RGBA32F held as halves (an integer class the GL
//! core stores is always two planes) and has neither. Until the flip (item
//! gc-w128-flip) the GL core stores RGBA32F as RGBA16F and refuses the
//! integer classes: the tests pin that closed state, say so on stderr, and
//! run the lifetime checks on what the GL core stores. They need no edit for
//! the flip, whether or not it keeps GL_RGBA32F as halves (the flip attempt
//! 7a597442 did; with its src/ changes applied locally all five tests pass,
//! the integer classes on the two-plane path).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_lifetime_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
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
use yakogl::shaderc::Wide128Planes;
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

const COMMON_3D: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/common_3d.xml.h"));

/// The value of `#define <name>` in the vendored common_3d.xml.h.
fn rnndb(name: &str) -> u32 {
    let line = COMMON_3D
        .lines()
        .find(|l| l.starts_with("#define") && l.split_whitespace().nth(1) == Some(name))
        .unwrap_or_else(|| panic!("{name} is in common_3d.xml.h"));
    let v = line.split_whitespace().nth(2).expect("a value");
    u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
}

type Class = (&'static str, u8, u32, &'static str);

const WIDE: [Class; 3] = [
    ("RGBA32F", GLES_TEXFMT_RGBA32F, GL_RGBA32F, "TEXTURE_FORMAT_EXT_G32R32F"),
    ("RGBA32UI", GLES_TEXFMT_RGBA32UI, GL_RGBA32UI, "TEXTURE_FORMAT_EXT_G32R32I"),
    ("RGBA32I", GLES_TEXFMT_RGBA32I, GL_RGBA32I, "TEXTURE_FORMAT_EXT_G32R32I"),
];

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

fn align4(v: u32) -> u32 {
    (v + 3) & !3
}

/* ---- the stand-in OSAL (as tests/es3_wide128_mixed_gc.rs) --------------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    /// What the back end gave back: the allocation, kept readable, and how
    /// many kicks had been submitted when it was freed.
    freed: Vec<(u64, Alloc, usize)>,
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
        if m.impl_.is_null() {
            return;
        }
        match p.allocs.remove(&m.gpu) {
            /* addresses are never handed out twice, so the freed bytes stay
             * readable for the model (and the order of frees is kept) */
            Some(a) => {
                let kicks = p.kicks.len();
                p.freed.push((m.gpu, a, kicks));
            }
            None => p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu)),
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

/// `n` bytes of the stand-in's memory at GPU address `gpu`, live or freed,
/// and the kick count at its free (`None`: still live).
fn read_gpu(gpu: u64, n: usize) -> (Vec<u8>, Option<usize>) {
    with_platform(|p| {
        let live = p.allocs.range(..=gpu).next_back().map(|(b, a)| (*b, a, None));
        let dead =
            p.freed.iter().filter(|(b, _, _)| *b <= gpu).max_by_key(|(b, _, _)| *b).map(|(b, a, k)| (*b, a, Some(*k)));
        let (base, a, at) = [live, dead]
            .into_iter()
            .flatten()
            .max_by_key(|(b, _, _)| *b)
            .unwrap_or_else(|| panic!("no allocation at 0x{gpu:x}"));
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x}+{n} runs off the allocation at 0x{base:x} ({} bytes)", a.size);
        // SAFETY: inside the allocation (checked just above).
        let bytes = unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec();
        (bytes, at)
    })
}

fn words_at(gpu: u64, n: usize) -> Vec<u32> {
    let (b, _) = read_gpu(gpu, n * 4);
    let (words, _) = b.as_chunks::<4>();
    words.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// The kick count when the allocation holding `gpu` was freed, if it was.
fn freed_at(gpu: u64) -> Option<usize> {
    read_gpu(gpu, 4).1
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
        for (_, a, _) in core::mem::take(&mut p.freed) {
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

fn link(fs: &str) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
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

fn sampler_of(format: u8) -> &'static str {
    match format {
        GLES_TEXFMT_RGBA32UI => "highp usampler2D",
        GLES_TEXFMT_RGBA32I => "highp isampler2D",
        _ => "sampler2D",
    }
}

/// One sampler `s` of the class's kind, read at texel (0, 0).
fn program(format: u8) -> GLuint {
    let sampler = sampler_of(format);
    link(&format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform {sampler} s;\nout vec4 o;\n\
         void main() {{ o = vec4(texelFetch(s, ivec2(0), 0)); }}\n"
    ))
}

/// Three samplers: `c` (a `sampler2D`, RGBA8 in (c)), `f` and `g` of the class kind.
fn three_program(format: u8) -> GLuint {
    let sampler = sampler_of(format);
    link(&format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n\
         uniform sampler2D c;\nuniform {sampler} f;\nuniform {sampler} g;\nout vec4 o;\n\
         void main() {{ o = texelFetch(c, ivec2(0), 0) + vec4(texelFetch(f, ivec2(0), 0))\n\
           + vec4(texelFetch(g, ivec2(0), 0)); }}\n"
    ))
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// One quad with program `p`, sampler uniforms `names[k]` on their units,
/// not finished: the draw's plane key as the GL core chose it.
fn draw_only(p: GLuint, names: &[(&core::ffi::CStr, i32)]) -> Wide128Planes {
    let refused = draws_refused();
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        for (name, unit) in names {
            glUniform1i(glGetUniformLocation(p, name.as_ptr()), *unit);
        }
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    assert_eq!(draws_refused(), refused, "the draw is not refused");
    let (_, planes) = wide128_draw_state(p).expect("the draw asked the program's sensitivity");
    planes[1]
}

/// `glFinish`: the `(kick, register, value)` writes of every kick since `kicks0`.
fn finish(kicks0: usize) -> Vec<(usize, u32, u32)> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    no_error("glFinish");
    with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the draws produced no kick");
        p.kicks[kicks0..]
            .iter()
            .enumerate()
            .flat_map(|(k, words)| writes(words).into_iter().map(move |(r, v)| (kicks0 + k, r, v)))
            .collect()
    })
}

fn kicks() -> usize {
    with_platform(|p| p.kicks.len())
}

/// One finished quad: its fragment plane key and its writes.
fn draw(p: GLuint, names: &[(&core::ffi::CStr, i32)]) -> (Wide128Planes, Vec<(u32, u32)>) {
    let k0 = kicks();
    let key = draw_only(p, names);
    let s = finish(k0).into_iter().map(|(_, r, v)| (r, v)).collect();
    (key, s)
}

/// The words of the descriptor at `addr`.
fn desc_at(addr: u32) -> Vec<u32> {
    words_at(u64::from(addr), GC_TEXDESC_BYTES / 4)
}

/// The words of slot `slot`'s descriptor in the stream, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| desc_at(a))
}

fn ext(d: &[u32]) -> u32 {
    d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f
}

fn lod0(d: &[u32]) -> u32 {
    d[texdesc_lod_addr(0) as usize / 4]
}

/// Descriptor words 16.. (everything but the sixteen level addresses).
fn but_levels(d: &[u32]) -> &[u32] {
    &d[16..]
}

fn bound_2d(unit: u32) -> GLuint {
    let mut v: GLint = -1;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        glActiveTexture(GL_TEXTURE0 + unit);
        glGetIntegerv(GL_TEXTURE_BINDING_2D, &mut v);
    }
    no_error("GL_TEXTURE_BINDING_2D");
    v as GLuint
}

fn delete(t: GLuint) {
    // SAFETY: plain GL call; one name.
    unsafe { glDeleteTextures(1, &t) };
    no_error("glDeleteTextures");
}

/* ---- the textures ------------------------------------------------------- */

const W: u32 = 8;
const H: u32 = 4;

/// The words of every texel of a class: a finite float in [1, 2) for RGBA32F.
fn texel(float: bool, salt: u32) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let h = (salt + 1).wrapping_mul(0x9e37_79b1) ^ (c as u32 + 1).wrapping_mul(0x27d4_eb2f);
        *o = if float { 0x3f80_0000 | (h & 0x007f_ffff) } else { h };
    }
    out
}

fn value_of(class: &Class, salt: u32) -> [u32; 4] {
    texel(class.1 == GLES_TEXFMT_RGBA32F, salt)
}

/// A new name bound on `unit`, NEAREST-filtered.
fn new_texture(unit: u32) -> GLuint {
    let mut t: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0 + unit);
        glGenTextures(1, &mut t);
    }
    bind(unit, t);
    t
}

/// Bind `t` on `unit` with NEAREST filters.
fn bind(unit: u32, t: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0 + unit);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    no_error("bind");
}

/// A one-level 8x4 image of one value into the texture bound on `unit`:
/// the error the upload left.
fn image_wide(unit: u32, class: &Class, value: [u32; 4]) -> GLenum {
    let (ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGBA),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    let image: Vec<u8> = (0..W * H).flat_map(|_| value.iter().flat_map(|w| w.to_le_bytes())).collect();
    // SAFETY: plain GL calls; the image holds W * H 16-byte texels.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glActiveTexture(GL_TEXTURE0 + unit);
        glTexImage2D(GL_TEXTURE_2D, 0, class.2 as GLint, W as GLsizei, H as GLsizei, 0, fmt, ty, image.as_ptr().cast());
        glGetError()
    }
}

fn image_rgba8(unit: u32) {
    let image: Vec<u8> = (0..W * H).flat_map(|k| [k as u8, 0x40, 0x80, 0xff]).collect();
    // SAFETY: plain GL calls; the image holds W * H 4-byte texels.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glActiveTexture(GL_TEXTURE0 + unit);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8 as GLint,
            W as GLsizei,
            H as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            image.as_ptr().cast(),
        );
    }
    no_error("RGBA8");
}

/// A new RGBA32 texture of `class` on `unit`, or `None` (said on stderr)
/// where the closed state refuses the class (the integer pair).
fn wide_texture(unit: u32, class: &Class, value: [u32; 4]) -> Option<GLuint> {
    let t = new_texture(unit);
    let e = image_wide(unit, class, value);
    if open(class.1) {
        assert_eq!(e, GL_NO_ERROR, "open: {} is stored", class.0);
    } else if class.1 == GLES_TEXFMT_RGBA32F {
        assert_eq!(e, GL_NO_ERROR, "closed: RGBA32F is stored (as RGBA16F)");
    } else {
        assert_eq!(e, GL_INVALID_OPERATION, "closed: {} is refused", class.0);
        eprintln!(
            "{}: not in GC_SAMPLED_TEXFMTS, refused by the GL core; its lifetime case is skipped (gc-w128-flip)",
            class.0
        );
        delete(t);
        return None;
    }
    Some(t)
}

/// The black texel's level address in this context: a draw of `p` with
/// nothing bound on unit 0.
fn black_lod0(p: GLuint) -> u32 {
    bind(0, 0);
    let (key, s) = draw(p, &[(c"s", 0)]);
    assert!(key.is_empty(), "nothing bound: no plane key");
    lod0(&descriptor(&s, 0).expect("slot 0 holds the black texel's descriptor"))
}

/// Does the draw sample `class` as two planes, read from the descriptor of
/// its unit (`d0`), not from the mask? The GL core may keep GL_RGBA32F as
/// halves with the class open; an integer class it stores is always planes.
fn planes_of(class: &Class, d0: &[u32]) -> bool {
    let planes = ext(d0) == rnndb(class.3);
    if planes {
        assert!(open(class.1), "{}: planes only for a class in the sampled mask", class.0);
    } else {
        assert_eq!(class.1, GLES_TEXFMT_RGBA32F, "{}: a stored integer class is two planes", class.0);
    }
    planes
}

/// The first draw on an RGBA32 binding of `class` on unit `unit`, companion
/// slot `companion`, value `value`: the plane key and both planes where the
/// class is stored as planes, nothing of either otherwise; `d0` and `d1` are the descriptors of the
/// unit's slot and of its companion slot.
fn check_bound(
    what: &str,
    class: &Class,
    key: &Wide128Planes,
    (d0, d1): (&[u32], Option<&[u32]>),
    (unit, companion): (u32, u32),
    value: [u32; 4],
) {
    if !planes_of(class, d0) {
        eprintln!("{what}: {} is not stored as two planes; the bound draw is pinned closed (gc-w128-flip)", class.0);
        assert_eq!(key.mask & (1 << unit), 0, "{what}: closed: no plane key on unit {unit}");
        assert!(d1.is_none_or(|d| ext(d) != rnndb(class.3)), "{what}: closed: no companion plane");
        return;
    }
    assert_ne!(key.mask & (1 << unit), 0, "{what}: open: unit {unit} is keyed: {key:?}");
    assert_eq!(key.companion[unit as usize] as u32, companion, "{what}: open: the companion slot");
    let d1 = d1.unwrap_or_else(|| panic!("{what}: open: a companion descriptor in slot {companion}"));
    for (p, d) in [d0, d1].into_iter().enumerate() {
        assert_eq!(ext(d), rnndb(class.3), "{what}: plane {p}: {}", class.3);
    }
    assert_eq!(but_levels(d0), but_levels(d1), "{what}: the companion is a copy but for the level addresses");
    let (a0, a1) = (lod0(d0), lod0(d1));
    assert_eq!(a1, a0 + align4(W) * align4(H) * 8, "{what}: plane 1 half-way into the level");
    assert_eq!(words_at(u64::from(a0), 2), value[..2], "{what}: R,G in plane 0");
    assert_eq!(words_at(u64::from(a1), 2), value[2..], "{what}: B,A in plane 1");
}

/// After the delete: unit `unit` reads texture 0, its slot holds the black
/// texel, no key, no descriptor in `companion`.
fn check_fallen_back(what: &str, key: &Wide128Planes, s: &[(u32, u32)], unit: u32, companion: u32, black: u32) {
    assert_eq!(bound_2d(unit), 0, "{what}: unit {unit} falls back to texture 0");
    assert_eq!(key.mask & (1 << unit), 0, "{what}: no plane key on unit {unit}: {key:?}");
    let d = descriptor(s, unit).unwrap_or_else(|| panic!("{what}: slot {unit} holds the black texel's descriptor"));
    assert_eq!(lod0(&d), black, "{what}: slot {unit} samples the black texel");
    assert!(WIDE.iter().all(|c| ext(&d) != rnndb(c.3)), "{what}: slot {unit} is no G32R32 plane");
    assert!(descriptor(s, companion).is_none(), "{what}: no companion descriptor in slot {companion}");
}

/* ---- the cases ---------------------------------------------------------- */

/// (a) glDeleteTextures on a bound RGBA32 texture, then a draw.
#[test]
fn deleting_a_bound_rgba32_texture_falls_back_to_texture_0() {
    for (k, class) in WIDE.iter().enumerate() {
        on_gc7000(|| {
            let p = program(class.1);
            let black = black_lod0(p);
            let value = value_of(class, k as u32 + 1);
            let Some(t) = wide_texture(0, class, value) else { return };
            let (key, s) = draw(p, &[(c"s", 0)]);
            let d0 = descriptor(&s, 0).expect("slot 0");
            check_bound(class.0, class, &key, (&d0, descriptor(&s, 1).as_deref()), (0, 1), value);
            assert_eq!(bound_2d(0), t);

            delete(t);
            let (key, s) = draw(p, &[(c"s", 0)]);
            assert!(key.is_empty(), "{}: the draw after the delete has no plane key: {key:?}", class.0);
            check_fallen_back(&format!("{} deleted", class.0), &key, &s, 0, 1, black);
        });
    }
}

/// (b) A draw, the delete and a second draw in one frame.
#[test]
fn a_delete_between_two_draws_of_one_frame_keeps_the_first_draws_descriptors() {
    for (k, class) in WIDE.iter().enumerate() {
        on_gc7000(|| {
            let p = program(class.1);
            let black = black_lod0(p);
            let value = value_of(class, k as u32 + 11);
            let Some(t) = wide_texture(0, class, value) else { return };

            let k0 = kicks();
            let key1 = draw_only(p, &[(c"s", 0)]);
            delete(t);
            let key2 = draw_only(p, &[(c"s", 0)]);
            let all = finish(k0);

            /* the two draws' slot 0 and slot 1 addresses, in stream order */
            let addrs = |slot: u32| -> Vec<(usize, u32)> {
                all.iter().filter(|(_, r, _)| *r == nte_descriptor_addr(slot)).map(|(k, _, v)| (*k, *v)).collect()
            };
            let (s0, s1) = (addrs(0), addrs(1));
            assert!(s0.len() >= 2, "{}: both draws wrote slot 0: {s0:x?}", class.0);
            let (kick1, first) = s0[0];
            let (_, second) = *s0.last().unwrap();
            let d0 = desc_at(first);

            /* the first draw, as the stream already holds it */
            let d1 = if planes_of(class, &d0) {
                let (kc, c) = *s1.first().unwrap_or_else(|| panic!("{}: the first draw's companion", class.0));
                assert_eq!(kc, kick1, "{}: the companion travels in the first draw's kick", class.0);
                Some(desc_at(c))
            } else {
                None
            };
            check_bound(&format!("{} first draw", class.0), class, &key1, (&d0, d1.as_deref()), (0, 1), value);
            /* nothing the first draw reads went back to the OSAL before its kick */
            let mut read = vec![("slot 0's descriptor", first), ("plane 0's level", lod0(&d0))];
            if let Some(d1) = &d1 {
                read.push(("plane 1's level", lod0(d1)));
            }
            for (name, a) in read {
                if let Some(at) = freed_at(u64::from(a)) {
                    assert!(
                        at > kick1,
                        "{}: {name} at 0x{a:x} was freed after {at} kicks, before kick {kick1} that reads it",
                        class.0
                    );
                }
            }

            /* the second draw: texture 0, no key, no companion of its own */
            assert!(key2.is_empty(), "{}: the draw after the delete has no plane key: {key2:?}", class.0);
            assert_eq!(bound_2d(0), 0, "{}: unit 0 falls back to texture 0", class.0);
            let d = desc_at(second);
            assert_eq!(lod0(&d), black, "{}: the second draw samples the black texel", class.0);
            assert!(WIDE.iter().all(|c| ext(&d) != rnndb(c.3)), "{}: the second draw has no G32R32 plane", class.0);
            /* slot 1: the first draw's companion at most, nothing after it */
            assert!(s1.len() <= 1, "{}: only the first draw writes a companion: {s1:x?}", class.0);
        });
    }
}

/// (c) An RGBA32 texture deleted while bound on a unit that is not the
/// active one; a second unit of the class keeps its planes.
#[test]
fn deleting_on_another_unit_leaves_no_stale_companion() {
    for (k, class) in WIDE.iter().enumerate() {
        on_gc7000(|| deleting_on_another_unit(class, 21 + 2 * k as u32));
    }
}

fn deleting_on_another_unit(class: &Class, salt: u32) {
    let p = three_program(class.1);
    let names: [(&core::ffi::CStr, i32); 3] = [(c"c", 0), (c"f", 1), (c"g", 2)];
    /* the black texel: nothing bound on units 1 and 2 */
    bind(1, 0);
    bind(2, 0);
    new_texture(0);
    image_rgba8(0);
    let (_, s) = draw(p, &names);
    let black = lod0(&descriptor(&s, 1).expect("slot 1 holds the black texel's descriptor"));
    let rgba8 = descriptor(&s, 0).expect("slot 0");

    let (vf, vg) = (value_of(class, salt), value_of(class, salt + 1));
    let Some(f) = wide_texture(1, class, vf) else { return };
    let g = wide_texture(2, class, vg).expect("stored as f was");
    let (key, s) = draw(p, &names);
    /* three slots, then the companions of units 1 and 2 in unit order */
    let df = descriptor(&s, 1).expect("slot 1");
    let dg = descriptor(&s, 2).expect("slot 2");
    check_bound(&format!("{} f on unit 1", class.0), class, &key, (&df, descriptor(&s, 3).as_deref()), (1, 3), vf);
    check_bound(&format!("{} g on unit 2", class.0), class, &key, (&dg, descriptor(&s, 4).as_deref()), (2, 4), vg);

    /* unit 0 active: f goes from unit 1 without being bound there now */
    // SAFETY: plain GL call.
    unsafe { glActiveTexture(GL_TEXTURE0) };
    delete(f);
    assert_eq!(bound_2d(2), g, "{}: g stays on unit 2", class.0);
    let (key, s) = draw(p, &names);
    check_fallen_back(&format!("{} f deleted", class.0), &key, &s, 1, 4, black);
    let d0 = descriptor(&s, 0).expect("slot 0");
    assert_eq!(but_levels(&d0), but_levels(&rgba8), "{}: the RGBA8 unit is untouched", class.0);
    /* g's companion is now the first one, in slot 3 */
    let dg = descriptor(&s, 2).expect("slot 2");
    let what = format!("{} g after f's delete", class.0);
    check_bound(&what, class, &key, (&dg, descriptor(&s, 3).as_deref()), (2, 3), vg);
    if planes_of(class, &dg) {
        assert_eq!(key.mask, 1 << 2, "{what}: only unit 2 is keyed: {key:?}");
    } else {
        assert!(descriptor(&s, 3).is_none(), "{what}: closed: no companion in slot 3");
    }

    /* and g: no key left at all */
    delete(g);
    let (key, s) = draw(p, &names);
    assert!(key.is_empty(), "{}: both deleted: no plane key: {key:?}", class.0);
    check_fallen_back(&format!("{} g deleted", class.0), &key, &s, 2, 3, black);
    assert!(descriptor(&s, 4).is_none(), "{}: both deleted: no companion in slot 4", class.0);
}

/// (d) The name of a deleted RGBA32 texture bound again with an RGBA8 image.
#[test]
fn a_name_reused_for_rgba8_after_an_rgba32_delete_is_unkeyed() {
    for (k, class) in WIDE.iter().enumerate() {
        on_gc7000(|| name_reused_for_rgba8(class, 31 + k as u32));
    }
}

fn name_reused_for_rgba8(class: &Class, salt: u32) {
    /* the RGBA8 draws go through a float sampler, the RGBA32 one through
     * the class's own */
    let p = program(GLES_TEXFMT_RGBA32F);
    let wide = program(class.1);
    /* a fresh RGBA8 texture's descriptor, for the comparison */
    let r = new_texture(0);
    image_rgba8(0);
    let (_, s) = draw(p, &[(c"s", 0)]);
    let fresh = descriptor(&s, 0).expect("slot 0");
    delete(r);

    let value = value_of(class, salt);
    let Some(t) = wide_texture(0, class, value) else { return };
    let (key, s) = draw(wide, &[(c"s", 0)]);
    let d0 = descriptor(&s, 0).expect("slot 0");
    check_bound(class.0, class, &key, (&d0, descriptor(&s, 1).as_deref()), (0, 1), value);
    delete(t);

    /* the same name again: ES binds a name it has not generated */
    bind(0, t);
    image_rgba8(0);
    assert_eq!(bound_2d(0), t, "{}: the name is bound again", class.0);
    let (key, s) = draw(p, &[(c"s", 0)]);
    assert!(key.is_empty(), "{}: the RGBA8 texture under the old name has no plane key: {key:?}", class.0);
    let d = descriptor(&s, 0).expect("slot 0");
    assert_eq!(ext(&d), ext(&fresh), "{}: the RGBA8 extended format word", class.0);
    assert_eq!(but_levels(&d), but_levels(&fresh), "{}: the descriptor is a fresh RGBA8 texture's", class.0);
    assert!(descriptor(&s, 1).is_none(), "{}: no companion descriptor in slot 1", class.0);
}

/// (e) Orphaning: the texture is attached to a framebuffer that is not bound
/// when it is deleted.
#[test]
fn an_orphaned_rgba32_texture_falls_back_like_a_deleted_one() {
    for (k, class) in WIDE.iter().enumerate() {
        on_gc7000(|| orphaned(class, 41 + k as u32));
    }
}

fn orphaned(class: &Class, salt: u32) {
    let p = program(class.1);
    let black = black_lod0(p);
    let value = value_of(class, salt);
    let Some(t) = wide_texture(0, class, value) else { return };
    let mut fb: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    no_error("attach");
    let (key, s) = draw(p, &[(c"s", 0)]);
    let d0 = descriptor(&s, 0).expect("slot 0");
    check_bound(&format!("{} attached", class.0), class, &key, (&d0, descriptor(&s, 1).as_deref()), (0, 1), value);

    delete(t);
    let (key, s) = draw(p, &[(c"s", 0)]);
    assert!(key.is_empty(), "{}: orphaned: no plane key: {key:?}", class.0);
    check_fallen_back(&format!("{} orphaned", class.0), &key, &s, 0, 1, black);

    /* the framebuffer takes the image with it; the next draw is as before */
    // SAFETY: plain GL call; one name.
    unsafe { glDeleteFramebuffers(1, &fb) };
    no_error("glDeleteFramebuffers");
    let (key, s) = draw(p, &[(c"s", 0)]);
    assert!(key.is_empty(), "{}: after the framebuffer: no plane key: {key:?}", class.0);
    check_fallen_back(&format!("{} framebuffer deleted", class.0), &key, &s, 0, 1, black);
}
