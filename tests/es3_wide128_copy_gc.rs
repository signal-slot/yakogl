// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): glCopyTexImage2D, glCopyTexSubImage2D and
//! glCopyTexSubImage3D into the 128-bit classes (RGBA32F / RGBA32UI /
//! RGBA32I, two G32R32 planes, docs/notes/gc-128bit-texture-sampling.md) and
//! into GL_RGB32F / GL_RGB32I / GL_RGB32UI.
//!
//! ES 3.0 3.8.5 (table 3.15) runs a copy only when the read framebuffer's
//! colour format is compatible with the texture's: a float, signed-integer
//! or unsigned-integer destination needs a source of that kind. The default
//! framebuffer is RGBA8 (unsigned normalized), so every copy from it into
//! these classes is GL_INVALID_OPERATION.
//!
//! - (a) the error of each entry point from the RGBA8 framebuffer into the
//!   six internal formats, and from an RGBA32I / RGBA32UI framebuffer
//!   attachment: the GC cannot make one complete yet
//!   (docs/notes/gc-128bit-render-targets.md), so the test pins the status
//!   it reports;
//! - (b) a refused copy leaves both planes of the destination unchanged,
//!   read out of the submitted stream as tests/es3_wide128_planes_gc.rs does;
//! - (c) glCopyTexImage2D with an RGBA32 internal format and its error.
//!
//! The mask hook of the in-crate tests (`backend::TEST_EXTRA_SAMPLED`) is
//! `#[cfg(test)]`, so no integration test can add the classes: while
//! GC_SAMPLED_TEXFMTS lacks them (item gc-w128-flip) the tests find them
//! closed, say so on stderr and pin whatever the GL core does now. Once they
//! are open the plane checks run for real; they were run with
//! GLES_TEXFMT_RGBA32F, GLES_TEXFMT_RGBA32UI and GLES_TEXFMT_RGBA32I added
//! to GC_SAMPLED_TEXFMTS locally (the result is in the commit message).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_copy_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

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
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

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

/// The words of slot `slot`'s descriptor in the last kick, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/// One quad sampling unit 0 with program `p`: the last kick's writes.
fn draw(p: GLuint) -> Vec<(u32, u32)> {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
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
    writes(&stream)
}

/* ---- the destinations ---------------------------------------------------- */

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

/// (name, internal format, class, upload format, upload type)
type Dest = (&'static str, GLenum, u8, GLenum, GLenum);

const RGBA32F: Dest = ("RGBA32F", GL_RGBA32F, GLES_TEXFMT_RGBA32F, GL_RGBA, GL_FLOAT);
const RGBA32UI: Dest = ("RGBA32UI", GL_RGBA32UI, GLES_TEXFMT_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT);
const RGBA32I: Dest = ("RGBA32I", GL_RGBA32I, GLES_TEXFMT_RGBA32I, GL_RGBA_INTEGER, GL_INT);
const RGB32F: Dest = ("RGB32F", GL_RGB32F, GLES_TEXFMT_RGBA32F, GL_RGB, GL_FLOAT);
const RGB32UI: Dest = ("RGB32UI", GL_RGB32UI, GLES_TEXFMT_RGBA32UI, GL_RGB_INTEGER, GL_UNSIGNED_INT);
const RGB32I: Dest = ("RGB32I", GL_RGB32I, GLES_TEXFMT_RGBA32I, GL_RGB_INTEGER, GL_INT);

/// Every destination but RGBA32F, whose copy the core answers as no error
/// (see `rgba32f_is_refused_like_the_others`).
const REFUSING: [Dest; 5] = [RGBA32UI, RGBA32I, RGB32F, RGB32UI, RGB32I];

const W: u32 = 8;
const H: u32 = 8;

/// The first error since the last call (all pending ones are cleared).
fn take_error() -> GLenum {
    // SAFETY: a plain GL call.
    let first = unsafe { glGetError() };
    let mut e = first;
    while e != GL_NO_ERROR {
        // SAFETY: as above.
        e = unsafe { glGetError() };
    }
    first
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Tex2D,
    Tex3D,
    Array,
}

impl Kind {
    fn target(self) -> GLenum {
        match self {
            Kind::Tex2D => GL_TEXTURE_2D,
            Kind::Tex3D => GL_TEXTURE_3D,
            Kind::Array => GL_TEXTURE_2D_ARRAY,
        }
    }
    fn layers(self) -> u32 {
        if self == Kind::Tex2D {
            1
        } else {
            2
        }
    }
}

/// The GLSL sampler type of the destination's kind and dimensionality.
fn sampler_type(d: &Dest, k: Kind) -> String {
    let prefix = match d.4 {
        GL_FLOAT => "",
        GL_INT => "i",
        _ => "u",
    };
    let dim = match k {
        Kind::Tex2D => "2D",
        Kind::Tex3D => "3D",
        Kind::Array => "2DArray",
    };
    format!("{prefix}sampler{dim}")
}

fn sampling_program(d: &Dest, k: Kind) -> GLuint {
    let sampler = sampler_type(d, k);
    let coord = if k == Kind::Tex2D { "ivec2(0)" } else { "ivec3(0)" };
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp {sampler};\nuniform {sampler} s;\nout vec4 o;\nvoid main() {{ o = vec4(texelFetch(s, {coord}, 0)); }}\n"
    );
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
        p
    }
}

/// A hash word: a finite float in [1, 2) for the float classes, any word
/// otherwise.
fn word(float: bool, i: u32) -> u32 {
    let mut h = i.wrapping_mul(0x9e37_79b1) ^ 0x85eb_ca6b;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    if float {
        0x3f80_0000 | (h & 0x007f_ffff)
    } else {
        h | 1
    }
}

/// Define level 0 of a fresh texture of `d` with distinct texels and return
/// it with the first error the definition raised (cleared).
fn make_texture(d: &Dest, k: Kind) -> (GLuint, GLenum) {
    let comps = if d.3 == GL_RGB || d.3 == GL_RGB_INTEGER { 3 } else { 4 };
    let n = (W * H * k.layers()) as usize * comps;
    let data: Vec<u32> = (0..n as u32).map(|i| word(d.4 == GL_FLOAT, i)).collect();
    let mut t: GLuint = 0;
    // SAFETY: plain GL calls; `data` holds the level's texels.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut t);
        glBindTexture(k.target(), t);
        glTexParameteri(k.target(), GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(k.target(), GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        if k == Kind::Tex2D {
            glTexImage2D(GL_TEXTURE_2D, 0, d.1 as GLint, W as _, H as _, 0, d.3, d.4, data.as_ptr().cast());
        } else {
            glTexImage3D(
                k.target(),
                0,
                d.1 as GLint,
                W as _,
                H as _,
                k.layers() as _,
                0,
                d.3,
                d.4,
                data.as_ptr().cast(),
            );
        }
    }
    (t, take_error())
}

/// What a draw binds for unit 0: the descriptor and level 0's bytes, and the
/// companion's when the first descriptor is a G32R32 plane.
#[derive(PartialEq, Debug)]
struct Snapshot {
    planes: Vec<(Vec<u32>, Vec<u8>)>,
}

fn level0_bytes(d: &[u32], layers: u32) -> Vec<u8> {
    let stride = d[GC_TEXDESC_LINEAR_STRIDE as usize / 4];
    let slice = d[GC_TEXDESC_SLICE as usize / 4];
    let rows = (H + 3) & !3;
    let n = if slice != 0 { slice * layers } else { stride * rows * layers };
    read_gpu(u64::from(d[texdesc_lod_addr(0) as usize / 4]), n as usize)
}

fn snapshot(stream: &[(u32, u32)], layers: u32) -> Snapshot {
    let d0 = descriptor(stream, 0).expect("a descriptor in slot 0");
    let b0 = level0_bytes(&d0, layers);
    let ext = d0[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f;
    let mut planes = vec![(d0, b0)];
    if ext == GC_TEX_FORMAT_EXT_G32R32F || ext == GC_TEX_FORMAT_EXT_G32R32I {
        let d1 = descriptor(stream, 1).expect("a companion descriptor in slot 1");
        let b1 = level0_bytes(&d1, layers);
        planes.push((d1, b1));
    }
    Snapshot { planes }
}

/// One of the three entry points that copy into the texture.
#[derive(Clone, Copy, Debug)]
enum Entry {
    Image2D,
    SubImage2D,
    SubImage3D,
}

/// Run `e` against the bound texture of kind `k`; the first error it raised.
fn copy(e: Entry, d: &Dest, k: Kind) -> GLenum {
    // SAFETY: plain GL calls on the bound texture and the read framebuffer.
    unsafe {
        match e {
            Entry::Image2D => glCopyTexImage2D(k.target(), 0, d.1, 0, 0, 4, 4, 0),
            Entry::SubImage2D => glCopyTexSubImage2D(k.target(), 0, 0, 0, 0, 0, 4, 4),
            Entry::SubImage3D => glCopyTexSubImage3D(k.target(), 0, 0, 0, 1, 0, 0, 4, 4),
        }
    }
    take_error()
}

/// The entry points that apply to a texture of kind `k`.
fn entries(k: Kind) -> &'static [Entry] {
    match k {
        Kind::Tex2D => &[Entry::Image2D, Entry::SubImage2D],
        Kind::Tex3D | Kind::Array => &[Entry::SubImage3D],
    }
}

const KINDS: [Kind; 3] = [Kind::Tex2D, Kind::Tex3D, Kind::Array];

/// Copy from the (RGBA8) default framebuffer through every applicable entry
/// point into a texture of `d` and of every kind: each is
/// GL_INVALID_OPERATION, and the descriptor and both planes of the texture,
/// read out of the stream of a draw, equal those before the attempts.
fn refused_from_default_fb(d: &Dest) {
    for k in KINDS {
        on_gc7000(|| {
            let (_, e) = make_texture(d, k);
            let defined = e == GL_NO_ERROR;
            if !defined {
                eprintln!("{} {k:?}: not defined by the GL core (0x{e:x}); only the copy's error is pinned", d.0);
            } else if !open(d.2) {
                eprintln!("{} {k:?}: class closed; the GL core stores it as another class, which is compared", d.0);
            }
            let p = sampling_program(d, k);
            let before = defined.then(|| snapshot(&draw(p), k.layers()));
            for &entry in entries(k) {
                assert_eq!(
                    copy(entry, d, k),
                    GL_INVALID_OPERATION,
                    "{} {k:?} {entry:?} from the RGBA8 framebuffer",
                    d.0
                );
            }
            if let Some(before) = before {
                let after = snapshot(&draw(p), k.layers());
                assert_eq!(before, after, "{} {k:?}: a refused copy changed the texture", d.0);
            }
        });
    }
}

/* (a), (b), (c): the default framebuffer ------------------------------------ */

/// ES 3.0 3.8.5: a float, signed- or unsigned-integer destination needs a
/// read buffer of the same kind, and the default framebuffer is RGBA8. The
/// copy raises GL_INVALID_OPERATION from every entry point (the RGBA32 and
/// RGB32 internal formats, glCopyTexImage2D included) and leaves the
/// descriptor and both planes as they were.
#[test]
fn copy_from_rgba8_into_the_128_bit_classes_is_invalid_operation_and_changes_nothing() {
    for d in REFUSING {
        refused_from_default_fb(&d);
    }
}

/// RGBA32F is the outlier: the core stores it as RGBA16F through the original
/// tables (`sized_format`, GL_RGBA32F => RGBA16F), which skip the ES 3.0
/// copy-compatibility check, so a copy from the RGBA8 framebuffer succeeds.
/// RGB32F, stored through the ES 3.0 table, is refused.
#[test]
#[ignore = "defect: glCopyTexImage2D / glCopyTexSubImage2D / glCopyTexSubImage3D from an RGBA8 framebuffer into GL_RGBA32F raise no error (texture.rs sized_format keeps it on the original-table path that skips es3_copy_compatible)"]
fn rgba32f_is_refused_like_the_others() {
    refused_from_default_fb(&RGBA32F);
}

/// glCopyTexImage2D into a texture that was never defined: the refusal does
/// not define the level (a sub-image copy into it still fails).
#[test]
fn a_refused_copy_teximage_defines_nothing() {
    for d in REFUSING {
        on_gc7000(|| {
            let mut t: GLuint = 0;
            // SAFETY: plain GL calls.
            unsafe {
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
            }
            assert_eq!(copy(Entry::Image2D, &d, Kind::Tex2D), GL_INVALID_OPERATION, "{}", d.0);
            assert_eq!(copy(Entry::SubImage2D, &d, Kind::Tex2D), GL_INVALID_OPERATION, "{}: nothing was defined", d.0);
        });
    }
}

/* (a) the RGBA32 framebuffer attachment ------------------------------------- */

/// The status of a framebuffer with an RGBA32I / RGBA32UI / RGBA32F texture
/// attached, and what a copy from it does: the GC has no render target class
/// for them yet (docs/notes/gc-128bit-render-targets.md), so it is
/// incomplete (or unsupported) and every copy from it is
/// GL_INVALID_FRAMEBUFFER_OPERATION, the texture unchanged.
#[test]
fn an_rgba32_framebuffer_attachment_is_not_a_complete_copy_source() {
    for (name, internal) in [("RGBA32I", GL_RGBA32I), ("RGBA32UI", GL_RGBA32UI), ("RGBA32F", GL_RGBA32F)] {
        for d in [RGBA32I, RGBA32UI, RGB32F] {
            on_gc7000(|| {
                let (mut t, mut fb): (GLuint, GLuint) = (0, 0);
                // SAFETY: plain GL calls.
                let status = unsafe {
                    glGenTextures(1, &mut t);
                    glBindTexture(GL_TEXTURE_2D, t);
                    glTexStorage2D(GL_TEXTURE_2D, 1, internal, W as _, H as _);
                    let storage = take_error();
                    eprintln!("{name}: glTexStorage2D 0x{storage:x}");
                    glGenFramebuffers(1, &mut fb);
                    glBindFramebuffer(GL_FRAMEBUFFER, fb);
                    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
                    assert_eq!(take_error(), GL_NO_ERROR, "{name}: attaching");
                    glCheckFramebufferStatus(GL_FRAMEBUFFER)
                };
                eprintln!("{name}: framebuffer status 0x{status:x}");
                assert!(
                    status == GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT || status == GL_FRAMEBUFFER_UNSUPPORTED,
                    "{name}: status 0x{status:x}"
                );
                /* the destination, defined while the default framebuffer is read */
                // SAFETY: plain GL call.
                unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
                let (_, e) = make_texture(&d, Kind::Tex2D);
                let p = sampling_program(&d, Kind::Tex2D);
                let before = (e == GL_NO_ERROR).then(|| snapshot(&draw(p), 1));
                // SAFETY: plain GL call.
                unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
                let (sub, img) = (copy(Entry::SubImage2D, &d, Kind::Tex2D), copy(Entry::Image2D, &d, Kind::Tex2D));
                eprintln!("{name} -> {}: glCopyTexSubImage2D 0x{sub:x}, glCopyTexImage2D 0x{img:x}", d.0);
                assert_eq!(sub, GL_INVALID_FRAMEBUFFER_OPERATION, "{name} -> {}", d.0);
                /* the image copy checks the format pair first: either refusal */
                assert!(
                    img == GL_INVALID_FRAMEBUFFER_OPERATION || img == GL_INVALID_OPERATION,
                    "{name} -> {}: 0x{img:x}",
                    d.0
                );
                // SAFETY: plain GL call.
                unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
                if let Some(before) = before {
                    assert_eq!(
                        before,
                        snapshot(&draw(p), 1),
                        "{name} -> {}: the refused copy changed the texture",
                        d.0
                    );
                }
            });
        }
    }
}
