// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the four 32-bit integer classes R32UI, R32I, RG32UI
//! and RG32I as sampled textures (item gc-r32-decode of plan #335;
//! docs/es3/gc-formats.md sections 2-4).
//!
//! - The tests that always run read the descriptor words the back end writes
//!   for the four classes (`format_words`: EXT_R32I 0x23 for R32, EXT_G32R32I
//!   0x24 for RG32, both read out of the rnndb header by name, no CONFIG2
//!   sign bit for 32-bit channels, `etnaviv_texture_desc.c:187-189`, no
//!   INT_FILTER, linear or tiled addressing as given) and the copy a texel
//!   size picks: an RG32 texel is 8 bytes and is taken by `wide_texels`
//!   (a page-aligned 4x4-tiled copy) as RGBA16F's is; an R32 texel is 4 and
//!   is sampled as the GL core stores it.
//! - The upload tests go through the GL API on the stand-in OSAL: upload,
//!   draw an `isampler2D` / `usampler2D` look-up, and read the descriptor and
//!   the texels out of the submitted stream (R32: the linear storage at its
//!   stride; RG32: the tiled copy, byte-exact against a model).
//!
//! The four classes are in `GC_SAMPLED_TEXFMTS` (item gc-r32-flip), so the
//! upload tests run for real, and `the_gl_core_stores_the_32_bit_integer_formats_here`
//! checks what the GL core does with the eight 32-bit integer formats.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_r32_gc
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
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;
use yakogl::vivante::texture::{format_words, gc_texfmt, is_wide128};
use yakogl::vivante::tiled_copy::copy_bpp;

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

/// (name, class, sized internal format, GL format, GL type, words a texel, sampler, TE format).
type Class = (&'static str, u8, u32, u32, u32, usize, &'static str, &'static str);
const CLASSES: [Class; 4] = [
    ("R32UI", GLES_TEXFMT_R32UI, GL_R32UI, GL_RED_INTEGER, GL_UNSIGNED_INT, 1, "usampler2D", "TEXTURE_FORMAT_EXT_R32I"),
    ("R32I", GLES_TEXFMT_R32I, GL_R32I, GL_RED_INTEGER, GL_INT, 1, "isampler2D", "TEXTURE_FORMAT_EXT_R32I"),
    (
        "RG32UI",
        GLES_TEXFMT_RG32UI,
        GL_RG32UI,
        GL_RG_INTEGER,
        GL_UNSIGNED_INT,
        2,
        "usampler2D",
        "TEXTURE_FORMAT_EXT_G32R32I",
    ),
    ("RG32I", GLES_TEXFMT_RG32I, GL_RG32I, GL_RG_INTEGER, GL_INT, 2, "isampler2D", "TEXTURE_FORMAT_EXT_G32R32I"),
];

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

fn align4(v: u32) -> u32 {
    (v + 3) & !3
}

fn bind(format: u8) -> GlesTextureBind {
    GlesTextureBind { format, width: 8, height: 4, ..Default::default() }
}

/// The words of the four classes: Mesa's HALTI5 override, the rnndb values
/// by name, no CONFIG0 format, no sign bit, no INT_FILTER.
#[test]
fn the_descriptor_words_are_mesas_ext_r32i_and_g32r32i() {
    assert_eq!(rnndb("TEXTURE_FORMAT_EXT_R32I"), GC_TEX_FORMAT_EXT_R32I);
    assert_eq!(rnndb("TEXTURE_FORMAT_EXT_G32R32I"), GC_TEX_FORMAT_EXT_G32R32I);
    for tiled in [false, true] {
        for c in &CLASSES {
            let f = gc_texfmt(c.1);
            let ext = rnndb(c.7);
            assert_eq!((f.format, f.format_ext, f.config2), (0, ext, 0), "{}", c.0);
            assert!(f.decoded && !f.compressed && !f.no_alpha && !f.int_filter, "{}", c.0);
            assert!(!is_wide128(c.1), "{}: not a plane class", c.0);
            let [c0, c1, c2] = format_words(&bind(c.1), tiled);
            assert_eq!((c0 >> 13) & 0x1f, 0, "{}: no CONFIG0 format", c.0);
            let want = if tiled { GC_TEX_ADDRESSING_TILED } else { GC_TEX_ADDRESSING_LINEAR };
            assert_eq!((c0 >> 20) & 3, want, "{}: addressing", c.0);
            assert_eq!(c1 & 0x3f, ext, "{}: FORMAT_EXT", c.0);
            assert_eq!(c2, GC_TEXDESC_CONFIG2_BASE, "{}: no sign bit for 32-bit channels", c.0);
        }
    }
    /* one word for both signs */
    assert_eq!(gc_texfmt(GLES_TEXFMT_R32UI).format_ext, gc_texfmt(GLES_TEXFMT_R32I).format_ext);
    assert_eq!(gc_texfmt(GLES_TEXFMT_RG32UI).format_ext, gc_texfmt(GLES_TEXFMT_RG32I).format_ext);
}

/// The GL swizzle composes as for every class: the format's own is the
/// identity, and a binding without alpha reads A as 1.
#[test]
fn the_gl_swizzle_composes_with_the_identity() {
    for c in &CLASSES {
        let id = format_words(&bind(c.1), false)[1];
        let na = GlesTextureBind { no_alpha: 1, ..bind(c.1) };
        assert_ne!(format_words(&na, false)[1], id, "{}: no_alpha changes the alpha swizzle", c.0);
        /* green read as red, through the GL swizzle */
        let mut g = bind(c.1);
        g.swizzle[0] = GL_GREEN as _;
        assert_ne!(format_words(&g, false)[1], id, "{}: the GL swizzle reaches the descriptor", c.0);
    }
}

/// An RG32 texel is 8 bytes, the size `wide_texels` takes as RGBA16F's
/// (src/vivante/tiled_copy.rs): the copy's texel is the texel. An R32 texel
/// is 4 bytes and is not copied for its size.
#[test]
fn an_rg32_texel_is_an_eight_byte_wide_texel_and_an_r32_one_is_not() {
    for c in &CLASSES {
        assert_eq!(gles_texfmt_bytes(c.1), 4 * c.5 as u32, "{}: bytes a texel", c.0);
        assert_eq!(copy_bpp(&bind(c.1)), gles_texfmt_bytes(c.1), "{}: the copy's texel", c.0);
    }
    assert_eq!(copy_bpp(&bind(GLES_TEXFMT_RGBA16F)), 8);
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

fn program(sampler: &str) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp {sampler};\nuniform {sampler} s;\nout vec4 o;\nvoid main() {{ o = vec4(texelFetch(s, ivec2(0), 0)); }}\n"
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

/* ---- the texels and the model ------------------------------------------- */

fn word(x: u32, y: u32, c: u32) -> u32 {
    let mut h = x.wrapping_mul(0x85eb_ca6b) ^ y.wrapping_mul(0xc2b2_ae35) ^ c.wrapping_mul(0x27d4_eb2f) ^ 0x9e37_79b1;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^ (h >> 12)
}

/// Row-major words of a `w` x `h` level, `ch` words a texel.
fn model(ch: usize, w: u32, h: u32) -> Vec<u32> {
    (0..h).flat_map(|y| (0..w).flat_map(move |x| (0..ch as u32).map(move |c| word(x, y, c)))).collect()
}

/// glTexImage2D of the model at level 0; the GL error it left.
fn upload(c: &Class, w: u32, h: u32, m: &[u32]) -> GLenum {
    let bytes: Vec<u8> = m.iter().flat_map(|w| w.to_le_bytes()).collect();
    // SAFETY: plain GL calls; the image holds `w * h` texels of `c.5` words.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, c.2 as GLint, w as GLsizei, h as GLsizei, 0, c.3, c.4, bytes.as_ptr().cast());
        glGetError()
    }
}

/// The draw links with the integer sampler, binds a descriptor
/// with the class's words, and the texels are where the descriptor says.
#[test]
fn an_uploaded_texture_is_described_and_laid_out() {
    for c in &CLASSES {
        for (w, h) in [(13u32, 6u32), (5, 9), (16, 4), (8, 8)] {
            assert!(open(c.1), "{}: in GC_SAMPLED_TEXFMTS", c.0);
            on_gc7000(|| {
                let m = model(c.5, w, h);
                assert_eq!(upload(c, w, h, &m), GL_NO_ERROR, "{} {w}x{h}: glTexImage2D", c.0);
                let s = draw(program(c.6));
                let what = format!("{} {w}x{h}", c.0);
                let d = descriptor(&s, 0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0"));
                let c0 = d[GC_TEXDESC_CONFIG0 as usize / 4];
                let c1 = d[GC_TEXDESC_CONFIG1 as usize / 4];
                assert_eq!((c0 >> 13) & 0x1f, 0, "{what}: no CONFIG0 format");
                assert_eq!(c1 & 0x3f, rnndb(c.7), "{what}: {}", c.7);
                assert_eq!(d[GC_TEXDESC_CONFIG2 as usize / 4], GC_TEXDESC_CONFIG2_BASE, "{what}: no sign bit");
                assert_eq!(d[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(w) | texdesc_size_height(h), "{what}");
                let base = u64::from(d[texdesc_lod_addr(0) as usize / 4]);
                if c.5 == 2 {
                    /* 8-byte texels: the page-aligned 4x4-tiled copy */
                    assert_eq!((c0 >> 20) & 3, GC_TEX_ADDRESSING_TILED, "{what}: tiled copy");
                    assert_eq!(base % 4096, 0, "{what}: the copy starts on a page");
                    let tiles = words_at(base, (align4(w) * align4(h) * 2) as usize);
                    for y in 0..h {
                        for x in 0..w {
                            let tile = (y / 4) * (align4(w) / 4) + x / 4;
                            let at = ((tile * 16 + (y % 4) * 4 + x % 4) * 2) as usize;
                            let i = ((y * w + x) * 2) as usize;
                            assert_eq!(tiles[at..at + 2], m[i..i + 2], "{what}: texel ({x}, {y})");
                        }
                    }
                } else {
                    /* 4-byte texels: the GL core's linear storage at its stride */
                    assert_eq!((c0 >> 20) & 3, GC_TEX_ADDRESSING_LINEAR, "{what}: linear");
                    let stride = d[GC_TEXDESC_LINEAR_STRIDE as usize / 4];
                    assert!(stride >= w * 4, "{what}: stride {stride}");
                    for y in 0..h {
                        let row = words_at(base + u64::from(y * stride), w as usize);
                        assert_eq!(row[..], m[(y * w) as usize..((y + 1) * w) as usize], "{what}: row {y}");
                    }
                }
            });
        }
    }
}

/// RC17 (dEQP fbo.completeness `renderable.texture.*32*`): the GL core no
/// longer refuses the 32-bit integer formats of one and two channels as
/// textures, and still refuses the three- and four-channel ones (the
/// two-plane path of docs/es3/gc-formats.md section 3). The mask holds the
/// four classes, `storage_in` answers each format's own class, glTexImage2D
/// and glTexStorage2D raise nothing, and a texture colour attachment is made
/// without error; its status is COMPLETE (the GL core's
/// `integer_texture_surface` has rows for the classes, RC17; gl-r32-surface,
/// `6c5b004f`).
#[test]
fn the_gl_core_stores_the_32_bit_integer_formats_here() {
    use yakogl::gl::es3_formats::find;
    let own = [
        (GL_R32UI, GL_RED_INTEGER, GL_UNSIGNED_INT, GLES_TEXFMT_R32UI),
        (GL_R32I, GL_RED_INTEGER, GL_INT, GLES_TEXFMT_R32I),
        (GL_RG32UI, GL_RG_INTEGER, GL_UNSIGNED_INT, GLES_TEXFMT_RG32UI),
        (GL_RG32I, GL_RG_INTEGER, GL_INT, GLES_TEXFMT_RG32I),
    ];
    let wide = [
        (GL_RGB32UI, GL_RGB_INTEGER, GL_UNSIGNED_INT),
        (GL_RGB32I, GL_RGB_INTEGER, GL_INT),
        (GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT),
        (GL_RGBA32I, GL_RGBA_INTEGER, GL_INT),
    ];
    for (internal, format, ty, class) in own {
        assert!(open(class), "class {class}: in the mask");
        assert_eq!(find(internal).expect("an ES 3.0 format").1.storage_in(GC_SAMPLED_TEXFMTS), Some(class));
        on_gc7000(|| {
            // SAFETY: plain GL calls; no pixels.
            unsafe {
                let mut tex = [0; 2];
                glGenTextures(2, tex.as_mut_ptr());
                glBindTexture(GL_TEXTURE_2D, tex[0]);
                glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 16, 16, 0, format, ty, core::ptr::null());
                assert_eq!(glGetError(), GL_NO_ERROR, "0x{internal:x}: glTexImage2D");
                glBindTexture(GL_TEXTURE_2D, tex[1]);
                glTexStorage2D(GL_TEXTURE_2D, 1, internal, 16, 16);
                assert_eq!(glGetError(), GL_NO_ERROR, "0x{internal:x}: glTexStorage2D");
                let mut fb = 0;
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex[1], 0);
                assert_eq!(glGetError(), GL_NO_ERROR, "0x{internal:x}: the attachment");
                assert_eq!(
                    glCheckFramebufferStatus(GL_FRAMEBUFFER),
                    GL_FRAMEBUFFER_COMPLETE,
                    "0x{internal:x}: its own surface class (RC17)"
                );
            }
        });
    }
    for (internal, format, ty) in wide {
        assert_eq!(find(internal).expect("an ES 3.0 format").1.storage_in(GC_SAMPLED_TEXFMTS), None);
        on_gc7000(|| {
            // SAFETY: a plain GL call; no pixels.
            unsafe {
                let mut t = 0;
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
                glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 16, 16, 0, format, ty, core::ptr::null());
                assert_eq!(glGetError(), GL_INVALID_OPERATION, "0x{internal:x}: still refused");
            }
        });
    }
}
