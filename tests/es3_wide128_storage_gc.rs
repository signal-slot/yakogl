// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): what the GL core stores for GL_RGBA32F / RGB32F /
//! RG32F / R32F and the RGBA32 / RGB32 integer formats once the back end
//! samples the 16-byte classes (docs/notes/gc-128bit-texture-sampling.md).
//!
//! Through the public GL entry points on the stand-in OSAL, the tests upload
//! (glTexImage2D, glTexStorage2D + glTexSubImage2D, glTexImage3D for a 2D
//! array), draw, and read the descriptor pair and both G32R32 planes out of
//! the submitted stream (as tests/es3_wide128_planes_gc.rs). Pinned:
//!
//! - the uploaded words are stored bit-exact, the channels the format lacks
//!   as 0 and the alpha of an RGB32* or RG32F texture as 1.0f / integer 1;
//! - level addresses, sizes and strides of a 3-level NPOT chain, plane 1
//!   half-way into each level, `layers` slices to a plane for an array;
//! - a glTexSubImage2D into the middle of a level touches only its texels;
//! - GL_R32F keeps its own 4-byte class (it is no 128-bit class).
//!
//! What it cannot see: the sampling itself (the texel fetch is the
//! compiler's, tests/es3_wide128_gc.rs), the stored bytes of the padding of
//! a partial tile, and the board. The mask hook of the in-crate tests
//! (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so an integration test
//! cannot add the classes: while GC_SAMPLED_TEXFMTS lacks them (item
//! gc-w128-flip) each test says so on stderr and checks nothing; from the
//! flip on they run for real. They were run with the three classes added to
//! GC_SAMPLED_TEXFMTS locally (not committed).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_storage_gc
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
use yakogl::vivante::tiled_copy::tiled_layout;

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

fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

fn align4(v: u32) -> u32 {
    (v + 3) & !3
}

fn level_dims(w: u32, h: u32, l: u32) -> (u32, u32) {
    ((w >> l).max(1), (h >> l).max(1))
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

fn program(sampler: &str, array: bool) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let coord = if array { "ivec3(0)" } else { "ivec2(0)" };
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

/* ---- the formats, the texels and the model ------------------------------ */

#[derive(Clone, Copy)]
struct Fmt {
    name: &'static str,
    internal: GLenum,
    /// The upload's format and type.
    format: GLenum,
    ty: GLenum,
    /// Channels the format has; the others read 0, and alpha 1.
    channels: usize,
    float: bool,
    unsigned: bool,
    /// The class the core must store it in.
    class: u8,
    ext: &'static str,
}

const fn f(
    name: &'static str,
    internal: GLenum,
    format: GLenum,
    ty: GLenum,
    channels: usize,
    float: bool,
    unsigned: bool,
    class: u8,
) -> Fmt {
    let ext = if float { "TEXTURE_FORMAT_EXT_G32R32F" } else { "TEXTURE_FORMAT_EXT_G32R32I" };
    Fmt { name, internal, format, ty, channels, float, unsigned, class, ext }
}

const FORMATS: [Fmt; 7] = [
    f("RGBA32F", GL_RGBA32F, GL_RGBA, GL_FLOAT, 4, true, false, GLES_TEXFMT_RGBA32F),
    f("RGB32F", GL_RGB32F, GL_RGB, GL_FLOAT, 3, true, false, GLES_TEXFMT_RGBA32F),
    f("RG32F", GL_RG32F, GL_RG, GL_FLOAT, 2, true, false, GLES_TEXFMT_RGBA32F),
    f("RGBA32UI", GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, 4, false, true, GLES_TEXFMT_RGBA32UI),
    f("RGBA32I", GL_RGBA32I, GL_RGBA_INTEGER, GL_INT, 4, false, false, GLES_TEXFMT_RGBA32I),
    f("RGB32UI", GL_RGB32UI, GL_RGB_INTEGER, GL_UNSIGNED_INT, 3, false, true, GLES_TEXFMT_RGBA32UI),
    f("RGB32I", GL_RGB32I, GL_RGB_INTEGER, GL_INT, 3, false, false, GLES_TEXFMT_RGBA32I),
];

impl Fmt {
    fn sampler(&self, array: bool) -> &'static str {
        match (self.float, self.unsigned, array) {
            (true, _, false) => "sampler2D",
            (true, _, true) => "sampler2DArray",
            (false, true, false) => "usampler2D",
            (false, true, true) => "usampler2DArray",
            (false, false, false) => "isampler2D",
            (false, false, true) => "isampler2DArray",
        }
    }
    /// The alpha a format without one stores.
    fn one(&self) -> u32 {
        if self.float {
            0x3f80_0000
        } else {
            1
        }
    }
}

/// The words of texel (x, y) of `level`, `layer`: a hash; finite floats of
/// either sign in [1, 2) for the float formats, any word otherwise. `salt`
/// tells updates apart. Only the format's own channels are the upload's;
/// the rest are what the store must show.
fn texel(fmt: &Fmt, level: u32, layer: u32, x: u32, y: u32, salt: u32) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        if c >= fmt.channels {
            *o = if c == 3 { fmt.one() } else { 0 };
            continue;
        }
        let mut h = level.wrapping_mul(0x9e37_79b1)
            ^ x.wrapping_mul(0x85eb_ca6b)
            ^ y.wrapping_mul(0xc2b2_ae35)
            ^ layer.wrapping_mul(0x1b87_3593)
            ^ (c as u32).wrapping_mul(0x27d4_eb2f)
            ^ salt.wrapping_mul(0x1656_67b1);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        *o = if fmt.float { (h & 0x8000_0000) | 0x3f80_0000 | (h & 0x007f_ffff) } else { h };
    }
    out
}

/// One level of the model: `layers` images of row-major texels.
type Level = Vec<[u32; 4]>;

fn model(fmt: &Fmt, w: u32, h: u32, layers: u32, levels: u32) -> Vec<Level> {
    (0..levels)
        .map(|l| {
            let (lw, lh) = level_dims(w, h, l);
            (0..layers)
                .flat_map(|z| (0..lh).flat_map(move |y| (0..lw).map(move |x| texel(fmt, l, z, x, y, 0))))
                .collect()
        })
        .collect()
}

/// The upload's bytes for texels of the model: only the format's channels.
fn bytes_of(fmt: &Fmt, texels: &[[u32; 4]]) -> Vec<u8> {
    texels.iter().flat_map(|t| t[..fmt.channels].iter()).flat_map(|w| w.to_le_bytes()).collect()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Path {
    /// glTexImage2D per level.
    Image2D,
    /// glTexStorage2D, then glTexSubImage2D per level.
    Storage2D,
    /// glTexImage3D of GL_TEXTURE_2D_ARRAY per level.
    Array,
}

fn upload(fmt: &Fmt, path: Path, (w, h, layers): (u32, u32, u32), m: &[Level]) {
    let target = if path == Path::Array { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_2D };
    // SAFETY: plain GL calls; each level's image holds `lw * lh * depth` texels of the format.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        if path == Path::Storage2D {
            glTexStorage2D(target, m.len() as GLsizei, fmt.internal, w as GLsizei, h as GLsizei);
            no_error("glTexStorage2D");
        }
        for (l, texels) in m.iter().enumerate() {
            let (lw, lh) = level_dims(w, h, l as u32);
            let data = bytes_of(fmt, texels);
            match path {
                Path::Image2D => glTexImage2D(
                    target,
                    l as GLint,
                    fmt.internal as GLint,
                    lw as GLsizei,
                    lh as GLsizei,
                    0,
                    fmt.format,
                    fmt.ty,
                    data.as_ptr().cast(),
                ),
                Path::Storage2D => glTexSubImage2D(
                    target,
                    l as GLint,
                    0,
                    0,
                    lw as GLsizei,
                    lh as GLsizei,
                    fmt.format,
                    fmt.ty,
                    data.as_ptr().cast(),
                ),
                Path::Array => glTexImage3D(
                    target,
                    l as GLint,
                    fmt.internal as GLint,
                    lw as GLsizei,
                    lh as GLsizei,
                    layers as GLsizei,
                    0,
                    fmt.format,
                    fmt.ty,
                    data.as_ptr().cast(),
                ),
            }
            no_error(&format!("{} level {l} upload", fmt.name));
        }
    }
}

/// A `glTexSubImage2D` of `rect` = (x, y, w, h) at `level`, texels salted
/// with `salt`, applied to the model too.
fn patch(fmt: &Fmt, m: &mut [Level], (w, h): (u32, u32), level: u32, rect: (u32, u32, u32, u32), salt: u32) {
    let (x0, y0, rw, rh) = rect;
    let (lw, _) = level_dims(w, h, level);
    let mut tex = Vec::new();
    for y in y0..y0 + rh {
        for x in x0..x0 + rw {
            let t = texel(fmt, level, 0, x, y, salt);
            m[level as usize][(y * lw + x) as usize] = t;
            tex.push(t);
        }
    }
    let data = bytes_of(fmt, &tex);
    // SAFETY: the rectangle lies inside the level; `data` holds its texels.
    unsafe {
        glTexSubImage2D(
            GL_TEXTURE_2D,
            level as GLint,
            x0 as GLint,
            y0 as GLint,
            rw as GLsizei,
            rh as GLsizei,
            fmt.format,
            fmt.ty,
            data.as_ptr().cast(),
        )
    };
    no_error("glTexSubImage2D");
}

/// The draw's descriptor pair and both planes of every level and layer
/// against the model, byte-exact for every texel of the image (the padding
/// of a partial tile is not specified).
fn check_planes(what: &str, fmt: &Fmt, (w, h, layers): (u32, u32, u32), m: &[Level], stream: &[(u32, u32)]) {
    let levels = m.len() as u32;
    let d0 = descriptor(stream, 0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0"));
    let d1 = descriptor(stream, 1).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot 1"));
    let ext = rnndb(fmt.ext);
    for (p, d) in [&d0, &d1].into_iter().enumerate() {
        let c0 = d[GC_TEXDESC_CONFIG0 as usize / 4];
        let c1 = d[GC_TEXDESC_CONFIG1 as usize / 4];
        assert_eq!((c0 >> 20) & 3, GC_TEX_ADDRESSING_TILED, "{what}: plane {p}: TILED");
        assert_eq!(c1 & 0x3f, ext, "{what}: plane {p}: stored as {} planes, not another class", fmt.ext);
        assert_eq!(d[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(w) | texdesc_size_height(h), "{what}: size");
        assert_eq!(
            d[GC_TEXDESC_LINEAR_STRIDE as usize / 4],
            align4(w) * 8,
            "{what}: plane {p}: a row of G32R32 texels"
        );
        assert_eq!(d[GC_TEXDESC_SLICE as usize / 4], align4(w) * align4(h) * 8, "{what}: plane {p}: one plane's layer");
    }
    let (off, _) = tiled_layout(w, h, layers, levels, 16);
    for l in 0..levels {
        let (a0, a1) = (d0[texdesc_lod_addr(l) as usize / 4], d1[texdesc_lod_addr(l) as usize / 4]);
        let (lw, lh) = level_dims(w, h, l);
        let plane = align4(lw) * align4(lh) * 8;
        assert_eq!(
            a0 - d0[texdesc_lod_addr(0) as usize / 4],
            off[l as usize] as u32,
            "{what}: level {l}: the 16-byte layout's offset"
        );
        assert_eq!(a1, a0 + plane * layers, "{what}: level {l}: plane 1 half-way into the level");
        for z in 0..layers {
            let p0 = words_at(u64::from(a0) + u64::from(plane * z), plane as usize / 4);
            let p1 = words_at(u64::from(a1) + u64::from(plane * z), plane as usize / 4);
            for y in 0..lh {
                for x in 0..lw {
                    let tile = (y / 4) * (align4(lw) / 4) + x / 4;
                    let at = (tile * 16 + (y % 4) * 4 + x % 4) as usize * 2;
                    let want = m[l as usize][((z * lh + y) * lw + x) as usize];
                    assert_eq!([p0[at], p0[at + 1]], want[..2], "{what}: level {l} layer {z}: R,G of ({x}, {y})");
                    assert_eq!([p1[at], p1[at + 1]], want[2..], "{what}: level {l} layer {z}: B,A of ({x}, {y})");
                }
            }
        }
    }
}

fn levels_of(w: u32, h: u32) -> u32 {
    32 - w.max(h).leading_zeros()
}

/// Upload `fmt` by `path`, draw, check the planes against the model, then
/// `after` (more updates, each re-checked with the closure it is given).
/// False when the class is not in the sampled mask (nothing checked, said so).
fn run(
    fmt: &Fmt,
    path: Path,
    size: (u32, u32, u32),
    levels: u32,
    after: impl FnOnce(&mut [Level], &dyn Fn(&str, &[Level])),
) -> bool {
    if !open(fmt.class) {
        eprintln!("{}: not in GC_SAMPLED_TEXFMTS; the storage is not exercised (gc-w128-flip)", fmt.name);
        return false;
    }
    on_gc7000(|| {
        let (w, h, layers) = size;
        let mut m = model(fmt, w, h, layers, levels);
        upload(fmt, path, size, &m);
        let p = program(fmt.sampler(path == Path::Array), path == Path::Array);
        let what = format!("{} {path:?} {w}x{h}x{layers}", fmt.name);
        check_planes(&what, fmt, size, &m, &draw(p));
        let again = |s: &str, m: &[Level]| check_planes(&format!("{what} {s}"), fmt, size, m, &draw(p));
        after(&mut m, &again);
    });
    true
}

/// Four tests of one format of `FORMATS` (by index), each format its own
/// module so a defect of one format is ignored alone.
macro_rules! format_tests {
    ($($mod:ident: $idx:expr $(, $why:literal)?;)*) => {$(
        mod $mod {
            use super::*;

            /// The format stores the uploaded words bit-exact through
            /// glTexImage2D and through glTexStorage2D + glTexSubImage2D, the
            /// channels it lacks as 0 and alpha as 1.0f / 1, at sizes that are
            /// not tile-aligned.
            #[test]
            $(#[ignore = $why])?
            fn stores_16_byte_texels_bit_exact() {
                let fmt = &FORMATS[$idx];
                for path in [Path::Image2D, Path::Storage2D] {
                    for (w, h) in [(13u32, 6u32), (5, 9), (16, 4), (1, 1)] {
                        if !run(fmt, path, (w, h, 1), levels_of(w, h), |_, _| {}) {
                            return;
                        }
                    }
                }
            }

            /// A 3-level NPOT chain (7x5, 3x2, 1x1): level offsets are the
            /// 16-byte layout's, plane 1 half-way into each level, stride and
            /// slice of level 0.
            #[test]
            $(#[ignore = $why])?
            fn three_level_npot_chain_has_the_16_byte_layout() {
                let fmt = &FORMATS[$idx];
                for path in [Path::Image2D, Path::Storage2D] {
                    if !run(fmt, path, (7, 5, 1), 3, |_, _| {}) {
                        return;
                    }
                }
            }

            /// A glTexSubImage2D into the middle of a level (across the tile
            /// column at 4) changes exactly its texels in both planes.
            #[test]
            $(#[ignore = $why])?
            fn sub_image_into_the_middle_of_a_level_touches_only_its_texels() {
                let fmt = &FORMATS[$idx];
                run(fmt, Path::Image2D, (7, 5, 1), 3, |m, again| {
                    patch(fmt, m, (7, 5), 0, (2, 1, 4, 3), 1);
                    again("after a level 0 patch", m);
                    patch(fmt, m, (7, 5), 1, (1, 0, 2, 1), 2);
                    again("after a level 1 patch", m);
                });
            }

            /// glTexImage3D of a 2D array: `layers` slices to each plane,
            /// layer z at `z * align4(w) * align4(h) * 8` in both planes of
            /// every level.
            #[test]
            $(#[ignore = $why])?
            fn a_2d_array_stores_its_layers_in_both_planes() {
                let fmt = &FORMATS[$idx];
                for (w, h, layers) in [(7u32, 5u32, 3u32), (4, 4, 2)] {
                    if !run(fmt, Path::Array, (w, h, layers), levels_of(w, h), |_, _| {}) {
                        return;
                    }
                }
            }
        }
    )*};
}

format_tests! {
    rgba32f: 0, "GL_RGBA32F is stored as RGBA16F whatever the sampled mask says: sized_format (src/gl/texture.rs) hard-codes GL_RGBA32F => GLES_TEXFMT_RGBA16F";
    rgb32f: 1;
    rg32f: 2;
    rgba32ui: 3;
    rgba32i: 4;
    rgb32ui: 5;
    rgb32i: 6;
}

/// GL_R32F is no 128-bit class: the core keeps its own 4-byte class for it,
/// whatever the 16-byte classes in the mask.
#[test]
fn r32f_keeps_its_own_class() {
    if !open(GLES_TEXFMT_RGBA32F) {
        eprintln!("RGBA32F: not in GC_SAMPLED_TEXFMTS; not exercised (gc-w128-flip)");
        return;
    }
    on_gc7000(|| {
        let data: Vec<u8> = (0..4u32).flat_map(|i| (1.0f32 + i as f32).to_le_bytes()).collect();
        // SAFETY: plain GL calls; 2x2 floats.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            let mut t: GLuint = 0;
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_R32F as GLint, 2, 2, 0, GL_RED, GL_FLOAT, data.as_ptr().cast());
        }
        no_error("glTexImage2D R32F");
        let s = draw(program("sampler2D", false));
        let d = descriptor(&s, 0).expect("a descriptor in slot 0");
        assert_ne!(
            d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f,
            rnndb("TEXTURE_FORMAT_EXT_G32R32F"),
            "R32F is not G32R32F planes"
        );
        assert!(descriptor(&s, 1).is_none(), "no companion descriptor for a one-plane class");
    });
}
