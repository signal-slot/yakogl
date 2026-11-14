// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): partial uploads, the pixel-unpack state and a
//! `GL_PIXEL_UNPACK_BUFFER` reach both G32R32 planes of a 128-bit texture
//! (docs/notes/gc-128bit-texture-sampling.md; Mesa 26.2.1 etnaviv).
//!
//! RGBA32F, RGBA32UI and RGBA32I are stored as two planes, 4x4-tiled at 8
//! bytes a texel: plane 0 holds R,G and plane 1 holds B,A, plane 1 starting
//! `layers` layer-planes into the level. A `glTexSubImage*` has to land each
//! texel's two halves in the two planes without disturbing any other texel.
//! The draw tests go through the GL API on the stand-in OSAL
//! (tests/es3_wide128_planes_gc.rs, tests/es3_wide128_layers_gc.rs): upload,
//! patch, draw, and compare every texel of every level and layer in both
//! planes, byte for byte, against a model (row-major texels, tiled by the
//! test) that the patch is applied to as well. The padding of a tile that a
//! level does not fill is not specified and not compared.
//!
//! - (a) `glTexSubImage2D` of rectangles that are not tile-aligned, at an
//!   offset, on levels 0 to 3 of a 13x6 texture.
//! - (b) `glTexSubImage3D` into one layer, a run of layers and levels 0 to 2
//!   of a 2D array, and one slice and a run of slices of a 3D texture.
//! - (c) GL_UNPACK_ROW_LENGTH, _SKIP_PIXELS, _SKIP_ROWS, _ALIGNMENT,
//!   _IMAGE_HEIGHT and _SKIP_IMAGES alone and together, on an RGBA32F, an
//!   RGBA32I and an RGBA32UI upload. A 16-byte texel is always a multiple of
//!   the alignment, so GL_UNPACK_ALIGNMENT changes no byte. ES 3.0 applies
//!   _IMAGE_HEIGHT and _SKIP_IMAGES to 3D uploads only; a 2D upload with them
//!   set must ignore them.
//! - (d) the same uploads sourced from a GL_PIXEL_UNPACK_BUFFER at a byte
//!   offset (the core supports it: `pixel_buffer_resolve`), and the error of
//!   an offset that runs past the buffer (GL_INVALID_OPERATION).
//! - (e) negative integers in RGBA32I and values above 2^31 in RGBA32UI land
//!   unchanged in the right plane (a sign or a float conversion would show).
//!
//! The sampled mask hook that opens the 128-bit classes
//! (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so no integration test
//! can add them: until GC_SAMPLED_TEXFMTS holds them (item gc-w128-flip)
//! these find the classes closed, say so on stderr and check the closed
//! state; from the flip on they run for real.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_subimage_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::buffer::*;
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

const WIDE: [(&str, u8, u32, &str); 3] = [
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

/* ---- the textures under test -------------------------------------------- */

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    D2,
    Array,
    Volume,
}

type Class = (&'static str, u8, u32, &'static str);

/// Row-major texels of one layer of one level (four words each).
type Layer = Vec<[u32; 4]>;

/// A texture and its model: `model[level][layer]`.
struct Tex {
    class: &'static Class,
    kind: Kind,
    w: u32,
    h: u32,
    layers: u32,
    levels: u32,
    model: Vec<Vec<Layer>>,
}

fn float(class: &Class) -> bool {
    class.1 == GLES_TEXFMT_RGBA32F
}

/// The words of texel (x, y) in layer `k` of `level`, salted: every word
/// differs from its neighbours; an integer class uses the whole 32 bits
/// (half of them above 2^31 or negative), RGBA32F a finite float.
fn texel(class: &Class, level: u32, k: u32, x: u32, y: u32, salt: u32) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let mut h = (level + 1).wrapping_mul(0x9e37_79b1)
            ^ (k + 1).wrapping_mul(0x85eb_ca6b)
            ^ (x + 1).wrapping_mul(0xc2b2_ae35)
            ^ (y + 1).wrapping_mul(0x27d4_eb2f)
            ^ (salt + 1).wrapping_mul(0x1656_67b1)
            ^ (c as u32 + 1).wrapping_mul(0x9e37_79b9);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 13;
        *o = if float(class) { 0x3f80_0000 | (h & 0x007f_ffff) | (h & 0x8000_0000) } else { h };
    }
    out
}

fn layers_at(t: &Tex, level: u32) -> u32 {
    match t.kind {
        Kind::Volume => (t.layers >> level).max(1),
        Kind::Array => t.layers,
        Kind::D2 => 1,
    }
}

fn new_tex(class: &'static Class, kind: Kind, w: u32, h: u32, layers: u32, levels: u32) -> Tex {
    let mut t = Tex { class, kind, w, h, layers, levels, model: Vec::new() };
    t.model = (0..levels)
        .map(|l| {
            let (lw, lh) = level_dims(w, h, l);
            (0..layers_at(&t, l))
                .map(|k| (0..lh).flat_map(|y| (0..lw).map(move |x| texel(class, l, k, x, y, 0))).collect())
                .collect()
        })
        .collect();
    t
}

fn fmt_type(class: &Class) -> (GLenum, GLenum) {
    match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_RGBA, GL_FLOAT),
        GLES_TEXFMT_RGBA32I => (GL_RGBA_INTEGER, GL_INT),
        _ => (GL_RGBA_INTEGER, GL_UNSIGNED_INT),
    }
}

fn target(kind: Kind) -> GLenum {
    match kind {
        Kind::D2 => GL_TEXTURE_2D,
        Kind::Array => GL_TEXTURE_2D_ARRAY,
        Kind::Volume => GL_TEXTURE_3D,
    }
}

fn bytes_of(texels: &[[u32; 4]]) -> Vec<u8> {
    texels.iter().flatten().flat_map(|w| w.to_le_bytes()).collect()
}

/// Generate and bind the texture and upload every level of the model.
fn upload(t: &Tex) {
    let (fmt, ty) = fmt_type(t.class);
    // SAFETY: plain GL calls; each image holds the texels of the level it names.
    unsafe {
        let mut n: GLuint = 0;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut n);
        glBindTexture(target(t.kind), n);
        glTexParameteri(target(t.kind), GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(target(t.kind), GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        for l in 0..t.levels {
            let (lw, lh) = level_dims(t.w, t.h, l);
            let img: Vec<u8> = t.model[l as usize].iter().flat_map(|k| bytes_of(k)).collect();
            if t.kind == Kind::D2 {
                glTexImage2D(
                    GL_TEXTURE_2D,
                    l as GLint,
                    t.class.2 as GLint,
                    lw as GLsizei,
                    lh as GLsizei,
                    0,
                    fmt,
                    ty,
                    img.as_ptr().cast(),
                );
            } else {
                glTexImage3D(
                    target(t.kind),
                    l as GLint,
                    t.class.2 as GLint,
                    lw as GLsizei,
                    lh as GLsizei,
                    layers_at(t, l) as GLsizei,
                    0,
                    fmt,
                    ty,
                    img.as_ptr().cast(),
                );
            }
        }
    }
    no_error("upload");
}

/// The unpack state of one upload; 0 is the GL default for the counts.
#[derive(Clone, Copy, Default, Debug)]
struct Unpack {
    row_length: u32,
    skip_pixels: u32,
    skip_rows: u32,
    image_height: u32,
    skip_images: u32,
    alignment: u32,
}

const NO_STATE: Unpack =
    Unpack { row_length: 0, skip_pixels: 0, skip_rows: 0, image_height: 0, skip_images: 0, alignment: 0 };

/// One patch: level, origin (x, y, z) and size (w, h, d) of the rectangle.
#[derive(Clone, Copy)]
struct Patch {
    level: u32,
    at: (u32, u32, u32),
    size: (u32, u32, u32),
}

/// Bytes of memory in front of the image: a PBO offset that is not 0.
const PRE: usize = 48;

/// The source memory of a patch under `st`, texels placed where GL reads
/// them (ES 3.0 3.7.1: row stride `align(row_length * 16, alignment)`, image
/// stride `image_height` rows), everything else filler; `new[k][j * w + i]`
/// are the texels. For a 2D target GL does not apply the image height and
/// skip-images, so those are left out of the layout.
fn source(kind: Kind, st: &Unpack, size: (u32, u32, u32), new: &[Layer]) -> Vec<u8> {
    let (rw, rh, rd) = size;
    let row_len = if st.row_length == 0 { rw } else { st.row_length };
    let align = st.alignment.max(1);
    let stride = (row_len * 16).div_ceil(align) * align;
    let (img_h, skip_i) = if kind == Kind::D2 {
        (rh, 0)
    } else {
        (if st.image_height == 0 { rh } else { st.image_height }, st.skip_images)
    };
    let total = (((skip_i + rd - 1) * img_h + st.skip_rows + rh - 1) * stride + (st.skip_pixels + rw) * 16) as usize;
    /* filler: a pattern no model texel is likely to hold */
    let mut buf: Vec<u8> = (0..PRE + total).map(|i| 0xa5 ^ (i as u8).wrapping_mul(31)).collect();
    for k in 0..rd {
        for j in 0..rh {
            for i in 0..rw {
                let at =
                    PRE + (((skip_i + k) * img_h + st.skip_rows + j) * stride + (st.skip_pixels + i) * 16) as usize;
                let w = new[k as usize][(j * rw + i) as usize];
                buf[at..at + 16].copy_from_slice(&bytes_of(&[w]));
            }
        }
    }
    buf
}

fn set_unpack(st: &Unpack) {
    // SAFETY: plain GL calls.
    unsafe {
        glPixelStorei(GL_UNPACK_ROW_LENGTH, st.row_length as GLint);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, st.skip_pixels as GLint);
        glPixelStorei(GL_UNPACK_SKIP_ROWS, st.skip_rows as GLint);
        glPixelStorei(GL_UNPACK_IMAGE_HEIGHT, st.image_height as GLint);
        glPixelStorei(GL_UNPACK_SKIP_IMAGES, st.skip_images as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, st.alignment.max(1) as GLint);
    }
}

/// Issue the sub-image call of `p` from `src` (a pointer, or a byte offset
/// when a PBO is bound).
fn sub_call(t: &Tex, p: &Patch, src: *const c_void) {
    let (fmt, ty) = fmt_type(t.class);
    // SAFETY: the rectangle lies inside the level; `src` holds its texels.
    unsafe {
        if t.kind == Kind::D2 {
            glTexSubImage2D(
                GL_TEXTURE_2D,
                p.level as GLint,
                p.at.0 as GLint,
                p.at.1 as GLint,
                p.size.0 as GLsizei,
                p.size.1 as GLsizei,
                fmt,
                ty,
                src,
            );
        } else {
            glTexSubImage3D(
                target(t.kind),
                p.level as GLint,
                p.at.0 as GLint,
                p.at.1 as GLint,
                p.at.2 as GLint,
                p.size.0 as GLsizei,
                p.size.1 as GLsizei,
                p.size.2 as GLsizei,
                fmt,
                ty,
                src,
            );
        }
    }
}

/// A `glTexSubImage*` of `p` under `st`, from client memory or from a PBO,
/// with texels salted by `salt`; the model gets the same texels.
fn patch(t: &mut Tex, p: Patch, st: &Unpack, pbo: bool, salt: u32) {
    let new: Vec<Layer> = (0..p.size.2)
        .map(|k| {
            let mut v = Vec::new();
            for j in 0..p.size.1 {
                for i in 0..p.size.0 {
                    v.push(texel(t.class, p.level, p.at.2 + k, p.at.0 + i, p.at.1 + j, salt));
                }
            }
            v
        })
        .collect();
    patch_with(t, p, st, pbo, new);
}

/// As [`patch`], with the texels given (`new[k][j * w + i]`).
fn patch_with(t: &mut Tex, p: Patch, st: &Unpack, pbo: bool, new: Vec<Layer>) {
    let (lw, _) = level_dims(t.w, t.h, p.level);
    for (k, layer) in new.iter().enumerate() {
        for j in 0..p.size.1 {
            for i in 0..p.size.0 {
                let at = ((p.at.1 + j) * lw + p.at.0 + i) as usize;
                t.model[p.level as usize][p.at.2 as usize + k][at] = layer[(j * p.size.0 + i) as usize];
            }
        }
    }
    let buf = source(t.kind, st, p.size, &new);
    set_unpack(st);
    if pbo {
        // SAFETY: plain GL calls; the buffer holds `buf`, read from offset PRE.
        unsafe {
            let mut b: GLuint = 0;
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, b);
            glBufferData(GL_PIXEL_UNPACK_BUFFER, buf.len() as GLsizeiptr, buf.as_ptr().cast(), GL_STATIC_DRAW);
            sub_call(t, &p, PRE as *const c_void);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
            glDeleteBuffers(1, &b);
        }
    } else {
        sub_call(t, &p, buf[PRE..].as_ptr().cast());
    }
    set_unpack(&Unpack::default());
    no_error("glTexSubImage");
}

/// Both planes of every level and layer against the model, byte for byte,
/// plus the descriptor pair's format, addressing and plane addresses.
fn check(what: &str, t: &Tex, stream: &[(u32, u32)]) {
    let d0 = descriptor(stream, 0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0"));
    let d1 = descriptor(stream, 1).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot 1"));
    for (p, d) in [&d0, &d1].into_iter().enumerate() {
        assert_eq!(d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f, rnndb(t.class.3), "{what}: plane {p}: {}", t.class.3);
        assert_eq!((d[GC_TEXDESC_CONFIG0 as usize / 4] >> 20) & 3, GC_TEX_ADDRESSING_TILED, "{what}: plane {p}: TILED");
    }
    for l in 0..t.levels {
        let (lw, lh) = level_dims(t.w, t.h, l);
        /* the copy reserves the level-0 layer count at every level */
        let layers = if t.kind == Kind::D2 { 1 } else { t.layers };
        let plane = align4(lw) * align4(lh) * 8;
        let (a0, a1) = (d0[texdesc_lod_addr(l) as usize / 4], d1[texdesc_lod_addr(l) as usize / 4]);
        assert_eq!(a1, a0 + plane * layers, "{what}: level {l}: plane 1 after every layer of plane 0");
        for k in 0..layers_at(t, l) {
            let at = u64::from(plane) * u64::from(k);
            let (p0, p1) =
                (words_at(u64::from(a0) + at, plane as usize / 4), words_at(u64::from(a1) + at, plane as usize / 4));
            for y in 0..lh {
                for x in 0..lw {
                    let tile = (y / 4) * (align4(lw) / 4) + x / 4;
                    let i = (tile * 16 + (y % 4) * 4 + x % 4) as usize * 2;
                    let want = t.model[l as usize][k as usize][(y * lw + x) as usize];
                    assert_eq!([p0[i], p0[i + 1]], want[..2], "{what}: level {l} layer {k} ({x}, {y}): R,G in plane 0");
                    assert_eq!([p1[i], p1[i + 1]], want[2..], "{what}: level {l} layer {k} ({x}, {y}): B,A in plane 1");
                }
            }
        }
    }
}

fn sampler_type(class: &Class, kind: Kind) -> String {
    let prefix = match class.1 {
        GLES_TEXFMT_RGBA32F => "",
        GLES_TEXFMT_RGBA32I => "i",
        _ => "u",
    };
    let k = match kind {
        Kind::D2 => "sampler2D",
        Kind::Array => "sampler2DArray",
        Kind::Volume => "sampler3D",
    };
    format!("{prefix}{k}")
}

fn program(class: &Class, kind: Kind) -> GLuint {
    let ty = sampler_type(class, kind);
    let coord = if kind == Kind::D2 { "vec2(0.5)" } else { "vec3(0.5)" };
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp {ty};\nuniform {ty} s;\nout vec4 o;\nvoid main() {{ o = vec4(texture(s, {coord})); }}\n"
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

type Verify<'a> = &'a dyn Fn(&str, &Tex);

/// Run `body` for every class on a fresh context with the texture `make`
/// builds and uploads, or say that the closed state was checked instead.
/// `body` gets the texture and a `verify(step, tex)` that draws and compares.
fn each_class(what: &str, kind: Kind, make: fn(&'static Class) -> Tex, body: impl Fn(&mut Tex, Verify)) {
    for class in &WIDE {
        if !open(class.1) {
            eprintln!(
                "{what}: {} is not in GC_SAMPLED_TEXFMTS; the sub-image planes are not exercised (gc-w128-flip)",
                class.0
            );
            assert!(!wide128_sampled(&GlesTextureBind { format: class.1, ..Default::default() }));
            continue;
        }
        on_gc7000(|| {
            let mut t = make(class);
            upload(&t);
            let p = program(class, kind);
            let first = draw(p);
            if descriptor(&first, 0).map(|d| d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f) != Some(rnndb(class.3)) {
                eprintln!("{what}: {}: the GL core does not store the class as planes; not checked", class.0);
                return;
            }
            let verify = |step: &str, t: &Tex| check(&format!("{what}: {} {step}", class.0), t, &draw(p));
            verify("after the upload", &t);
            body(&mut t, &verify);
        });
    }
}

fn tex_2d(class: &'static Class) -> Tex {
    new_tex(class, Kind::D2, 13, 6, 1, 4)
}

fn tex_array(class: &'static Class) -> Tex {
    new_tex(class, Kind::Array, 7, 5, 3, 3)
}

fn tex_volume(class: &'static Class) -> Tex {
    new_tex(class, Kind::Volume, 6, 5, 4, 3)
}

type Case = (u32, (u32, u32, u32), (u32, u32, u32));

fn run_cases(t: &mut Tex, verify: Verify, cases: &[Case]) {
    for (n, &(level, at, size)) in cases.iter().enumerate() {
        patch(t, Patch { level, at, size }, &NO_STATE, false, n as u32 + 1);
        verify(&format!("after patch {n} (level {level}, {at:?}, {size:?})"), t);
    }
}

/* ---- (a) glTexSubImage2D ------------------------------------------------ */

/// Rectangles that cross the tile columns at 4 and 8 and the row at 4, a
/// single texel at the last column and row, and levels 1 to 3 (6x3, 3x1,
/// 1x1).
#[test]
fn a_2d_sub_image_off_the_tile_grid_touches_only_its_texels_in_both_planes() {
    each_class("2D sub-image", Kind::D2, tex_2d, |t, verify| {
        run_cases(
            t,
            verify,
            &[
                (0, (3, 1, 0), (7, 4, 1)),
                (0, (12, 5, 0), (1, 1, 1)),
                (0, (5, 3, 0), (4, 1, 1)),
                (0, (0, 0, 0), (13, 6, 1)),
                (1, (1, 0, 0), (4, 3, 1)),
                (1, (5, 2, 0), (1, 1, 1)),
                (2, (1, 0, 0), (2, 1, 1)),
                (3, (0, 0, 0), (1, 1, 1)),
            ],
        );
    });
}

/* ---- (b) glTexSubImage3D ------------------------------------------------ */

/// One layer, a run of layers and layers of levels 1 and 2 of a 2D array.
#[test]
fn a_3d_sub_image_reaches_one_layer_of_a_2d_array() {
    each_class("array sub-image", Kind::Array, tex_array, |t, verify| {
        run_cases(
            t,
            verify,
            &[
                (0, (1, 1, 1), (5, 3, 1)),
                (0, (0, 0, 2), (7, 5, 1)),
                (0, (3, 2, 0), (2, 2, 2)),
                (1, (1, 0, 2), (2, 2, 1)),
                (1, (0, 1, 0), (3, 1, 3)),
                (2, (0, 0, 1), (1, 1, 1)),
            ],
        );
    });
}

/// One slice and a run of slices of a 3D texture; level 1 has two slices and
/// level 2 one.
#[test]
fn a_3d_sub_image_reaches_one_slice_of_a_3d_texture() {
    each_class("3D sub-image", Kind::Volume, tex_volume, |t, verify| {
        run_cases(
            t,
            verify,
            &[
                (0, (1, 1, 2), (4, 3, 1)),
                (0, (0, 0, 0), (6, 5, 1)),
                (0, (2, 1, 1), (3, 3, 3)),
                (1, (1, 0, 1), (2, 2, 1)),
                (1, (0, 0, 0), (3, 2, 2)),
                (2, (0, 0, 0), (1, 1, 1)),
            ],
        );
    });
}

/* ---- (c), (d) the unpack state, client memory and PBO ------------------- */

fn unpack_cases() -> Vec<(&'static str, Unpack)> {
    let n = NO_STATE;
    vec![
        ("ROW_LENGTH", Unpack { row_length: 11, ..n }),
        ("SKIP_PIXELS", Unpack { skip_pixels: 3, ..n }),
        ("SKIP_ROWS", Unpack { skip_rows: 2, ..n }),
        ("ALIGNMENT 8", Unpack { alignment: 8, ..n }),
        ("ALIGNMENT 4 with ROW_LENGTH", Unpack { alignment: 4, row_length: 8, ..n }),
        ("ROW_LENGTH+SKIP_PIXELS+SKIP_ROWS", Unpack { row_length: 11, skip_pixels: 2, skip_rows: 1, ..n }),
        ("IMAGE_HEIGHT", Unpack { image_height: 9, ..n }),
        ("SKIP_IMAGES", Unpack { skip_images: 2, ..n }),
        ("IMAGE_HEIGHT+SKIP_IMAGES", Unpack { image_height: 8, skip_images: 1, ..n }),
        (
            "all six",
            Unpack { row_length: 12, skip_pixels: 3, skip_rows: 2, image_height: 9, skip_images: 1, alignment: 2 },
        ),
    ]
}

/// (c) on a 2D texture: the counts a 2D upload uses; GL_UNPACK_IMAGE_HEIGHT
/// is among them because it only matters between images. With `skip_images`
/// the cases that set GL_UNPACK_SKIP_IMAGES, which a 2D upload must ignore.
fn run_unpack_2d(pbo: bool, skip_images: bool) {
    each_class("2D unpack state", Kind::D2, tex_2d, |t, verify| {
        let cases = unpack_cases().into_iter().filter(|(_, st)| (st.skip_images != 0) == skip_images);
        for (n, (name, st)) in cases.enumerate() {
            let at = (n as u32 % 5, n as u32 % 3, 0);
            patch(t, Patch { level: 0, at, size: (7, 3, 1) }, &st, pbo, 10 + n as u32);
            verify(&format!("after {name} at {at:?} (pbo {pbo})"), t);
        }
        if !skip_images {
            let st = Unpack { row_length: 9, skip_pixels: 1, skip_rows: 1, ..NO_STATE };
            patch(t, Patch { level: 1, at: (1, 0, 0), size: (4, 3, 1) }, &st, pbo, 99);
            verify(&format!("after a level 1 upload (pbo {pbo})"), t);
        }
    });
}

/// (c) on a 2D array and on a 3D texture: all six counts.
fn run_unpack_3d(pbo: bool) {
    for (what, kind, make) in
        [("array", Kind::Array, tex_array as fn(&'static Class) -> Tex), ("3D", Kind::Volume, tex_volume)]
    {
        each_class(&format!("{what} unpack state"), kind, make, |t, verify| {
            for (n, (name, st)) in unpack_cases().into_iter().enumerate() {
                let at = (n as u32 % 3, n as u32 % 2, n as u32 % 2);
                patch(t, Patch { level: 0, at, size: (3, 2, 2) }, &st, pbo, 20 + n as u32);
                verify(&format!("after {name} at {at:?} (pbo {pbo})"), t);
            }
        });
    }
}

#[test]
fn unpack_state_on_a_2d_sub_image_from_client_memory() {
    run_unpack_2d(false, false);
}

#[test]
fn unpack_state_on_a_3d_sub_image_from_client_memory() {
    run_unpack_3d(false);
}

#[test]
fn unpack_state_on_a_2d_sub_image_from_a_pbo() {
    run_unpack_2d(true, false);
}

/// ES 3.0 3.7.1: GL_UNPACK_SKIP_IMAGES (and the image height) apply to 3D
/// and array uploads only (Mesa's `_mesa_image_address2d` takes no image
/// index). The GL core adds the skipped images to a 2D upload's source
/// offset too (`unpack_skip_bytes`, src/gl/texture.rs), which reads the
/// wrong texels, or past the end of a PBO. The class does not matter, the
/// pin is about the 128-bit path only because that is what this file runs.
#[test]
#[ignore = "defect: a 2D glTexSubImage2D applies GL_UNPACK_SKIP_IMAGES (src/gl/texture.rs unpack_skip_bytes)"]
fn a_2d_sub_image_ignores_skip_images_from_client_memory() {
    run_unpack_2d(false, true);
}

#[test]
#[ignore = "defect: a 2D glTexSubImage2D applies GL_UNPACK_SKIP_IMAGES (src/gl/texture.rs unpack_skip_bytes)"]
fn a_2d_sub_image_ignores_skip_images_from_a_pbo() {
    run_unpack_2d(true, true);
}

#[test]
fn unpack_state_on_a_3d_sub_image_from_a_pbo() {
    run_unpack_3d(true);
}

/// With a PBO bound the pointer is a byte offset; one that runs past the
/// buffer is GL_INVALID_OPERATION (ES 3.0 3.7.1) and changes nothing.
#[test]
fn a_pbo_offset_past_the_buffer_is_an_error_and_changes_nothing() {
    each_class("PBO error", Kind::D2, tex_2d, |t, verify| {
        let (fmt, ty) = fmt_type(t.class);
        // SAFETY: plain GL calls; the read starts past what the buffer holds, which the core refuses.
        let e = unsafe {
            let mut b: GLuint = 0;
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, b);
            glBufferData(GL_PIXEL_UNPACK_BUFFER, 16 * 8, [0u8; 16 * 8].as_ptr().cast(), GL_STATIC_DRAW);
            /* 4x2 texels need 128 bytes: from offset 16 only 112 are there */
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 2, fmt, ty, 16 as *const c_void);
            let e = glGetError();
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
            glDeleteBuffers(1, &b);
            e
        };
        assert_eq!(e, GL_INVALID_OPERATION, "a PBO read past the end of the buffer");
        verify("after the refused upload", t);
    });
}

/* ---- (e) values at the edges of the word -------------------------------- */

/// Negative integers and values above 2^31 are the words of the model, so a
/// sign extension, a float conversion or a clamp in the path would show.
#[test]
fn negative_and_large_integers_land_unchanged_in_their_planes() {
    each_class("extreme values", Kind::D2, tex_2d, |t, verify| {
        let words: [[u32; 4]; 4] = match t.class.1 {
            GLES_TEXFMT_RGBA32I => [
                [-1i32 as u32, i32::MIN as u32, i32::MAX as u32, -123_456_789i32 as u32],
                [i32::MIN as u32, -1i32 as u32, 0, 1],
                [1, -2i32 as u32, -3i32 as u32, i32::MIN as u32 + 1],
                [0xffff_ff80, 0x8000_0001, 0x7fff_ffff, 0x8000_0000],
            ],
            GLES_TEXFMT_RGBA32UI => [
                [0x8000_0000, 0xffff_ffff, 0x8000_0001, 0xdead_beef],
                [0xffff_fffe, 0x8000_0000, 0, 1],
                [0xc000_0000, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff],
                [0xffff_ff80, 0x8000_0001, 0x7fff_ffff, 0x0123_4567],
            ],
            /* floats: the extremes that are not NaN, the sign of zero, denormals, infinity */
            _ => [
                [0x0000_0000, 0x8000_0000, 0x7f7f_ffff, 0xff7f_ffff],
                [0x0000_0001, 0x8000_0001, 0x0080_0000, 0x7f80_0000],
                [0xff80_0000, 0x3f80_0000, 0xbf80_0000, 0x4b00_0000],
                [0x3380_0000, 0xc2f6_e979, 0x1234_5678, 0x5678_1234],
            ],
        };
        patch_with(t, Patch { level: 0, at: (3, 1, 0), size: (2, 2, 1) }, &NO_STATE, false, vec![words.to_vec()]);
        verify("after the 2x2 patch", t);
        patch_with(t, Patch { level: 0, at: (4, 4, 0), size: (1, 1, 1) }, &NO_STATE, true, vec![vec![words[3]]]);
        verify("after the 1-texel patch from a PBO", t);
        let st = Unpack { row_length: 3, skip_pixels: 1, ..NO_STATE };
        patch_with(t, Patch { level: 1, at: (0, 0, 0), size: (2, 2, 1) }, &st, false, vec![words.to_vec()]);
        verify("after the level 1 patch", t);
    });
}
