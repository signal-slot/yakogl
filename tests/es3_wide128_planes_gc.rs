// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): RGBA32F / RGBA32UI / RGBA32I textures are two G32R32
//! planes (docs/notes/gc-128bit-texture-sampling.md; Mesa 26.2.1 etnaviv).
//!
//! Plane 0 holds each texel's R,G words and plane 1 its B,A words, both
//! 4x4-tiled at 8 bytes a texel, plane 1 half-way into the level
//! (`etna_resource_level_second_plane_offset`, etnaviv_resource.h:91-94). Both
//! planes' descriptors carry the G32R32 word of Mesa's emulated 128-bit rows:
//! `EXT_G32R32F` for RGBA32F, `EXT_G32R32I` for the two integer classes
//! (etnaviv_format.c:229-233), no CONFIG2 sign bit and no INT_FILTER
//! (etnaviv_texture_desc.c:187-189); the second descriptor is a byte copy of
//! the first but for the level addresses (etnaviv_texture_desc.c:404-415).
//!
//! - The tests that always run read the word and the plane offset out of the
//!   public API against the rnndb header's values by name.
//! - The upload tests go through the GL API on the stand-in OSAL (as
//!   tests/gc_tiled_copy_reuse.rs): upload, draw, and read the descriptor pair
//!   and both planes out of the submitted stream, byte-exact against a model
//!   of the tiled layout, for non-square sizes, sizes that are not
//!   tile-aligned, and mip levels 1 and up; then a `glTexSubImage2D` of
//!   level 0 and of level 1 against the model.
//!
//! The upload tests need the three classes in the sampled mask. The mask
//! hook of the in-crate tests (`backend::TEST_EXTRA_SAMPLED`) is
//! `#[cfg(test)]`, so no integration test can add them: until
//! GC_SAMPLED_TEXFMTS holds them (item wide128-flip) the upload tests find
//! the classes closed, say so on stderr and check the closed state instead
//! (the GL core stores no 128-bit class for this back end). From the flip on
//! they run for real. Before the flip they were run with the three classes
//! added to GC_SAMPLED_TEXFMTS locally.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_planes_gc
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
use yakogl::vivante::texture::{is_wide128, wide128_plane_format};
use yakogl::vivante::tiled_copy::{copy_bpp, second_plane_offset, tiled_layout};

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

/* ---- the word and the plane offsets, from the API ---------------------- */

/// Both planes take the G32R32 word of Mesa's table, read by name from the
/// rnndb header: EXT_G32R32F for RGBA32F, EXT_G32R32I for the integer pair
/// (etnaviv_format.c:229-233); the plain format field is 0 (the extended
/// field names the format), and nothing else is a 128-bit class.
#[test]
fn the_plane_word_is_mesas_g32r32() {
    assert_eq!(rnndb("TEXTURE_FORMAT_EXT_G32R32F"), GC_TEX_FORMAT_EXT_G32R32F);
    assert_eq!(rnndb("TEXTURE_FORMAT_EXT_G32R32I"), GC_TEX_FORMAT_EXT_G32R32I);
    for (name, class, _, mesa) in WIDE {
        assert!(is_wide128(class), "{name}");
        let f = wide128_plane_format(class).unwrap_or_else(|| panic!("{name}: a plane format"));
        assert_eq!(f.format, 0, "{name}: no CONFIG0 format");
        assert_eq!(f.format_ext, rnndb(mesa), "{name}: {mesa}");
        assert_eq!(f.config2, 0, "{name}: no CONFIG2 sign bit (etnaviv_texture_desc.c:187-189)");
        assert!(!f.int_filter, "{name}: no INT_FILTER");
        assert!(!f.compressed && !f.no_alpha, "{name}");
    }
    for class in [
        GLES_TEXFMT_RGBA8,
        GLES_TEXFMT_RGBA16F,
        GLES_TEXFMT_RGBA16UI,
        GLES_TEXFMT_RGBA16I,
        GLES_TEXFMT_R32F,
        GLES_TEXFMT_RGBA8UI,
    ] {
        assert!(!is_wide128(class), "class {class}");
        assert!(wide128_plane_format(class).is_none(), "class {class}");
    }
}

/// A level is two planes (Mesa sizes it as its 64-bit twin, twice,
/// etnaviv_resource.c:310-338): the chain is the 16-byte-texel layout, plane 1
/// starts at `layers * align4(w) * align4(h) * 8` from the level's start, and
/// the copy's texel is the plane's 8 bytes.
#[test]
fn the_second_plane_is_half_way_into_each_level() {
    for (w, h, layers) in [(13u32, 6u32, 1u32), (5, 9, 1), (16, 4, 1), (7, 7, 3), (1, 1, 1)] {
        let levels = 32 - w.max(h).leading_zeros();
        let t = GlesTextureBind {
            format: GLES_TEXFMT_RGBA32F,
            width: w,
            height: h,
            layers: layers as _,
            mip_levels: levels as _,
            ..Default::default()
        };
        assert_eq!(copy_bpp(&t), 8, "a plane's texel");
        let (off, total) = tiled_layout(w, h, layers, levels, 16);
        for l in 0..levels {
            let (lw, lh) = level_dims(w, h, l);
            let plane = (align4(lw) * align4(lh) * 8 * layers) as usize;
            assert_eq!(second_plane_offset(&t, l), plane, "{w}x{h}x{layers} level {l}");
            let end = if l + 1 < levels { off[l as usize + 1] } else { total };
            assert!(end >= off[l as usize] + 2 * plane, "{w}x{h} level {l}: two planes fit before the next level");
            assert_eq!(off[l as usize] % GC_TEXDESC_LOD_ALIGN, 0, "{w}x{h} level {l}: 64-byte level");
        }
    }
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

/// The four words of texel (x, y) of `level`: a hash, the finite floats
/// [1, 2) for RGBA32F, any word otherwise. `salt` tells updates apart.
fn texel(float: bool, level: u32, x: u32, y: u32, salt: u32) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let mut h = level.wrapping_mul(0x9e37_79b1)
            ^ x.wrapping_mul(0x85eb_ca6b)
            ^ y.wrapping_mul(0xc2b2_ae35)
            ^ (c as u32).wrapping_mul(0x27d4_eb2f)
            ^ salt.wrapping_mul(0x1656_67b1);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        *o = if float { 0x3f80_0000 | (h & 0x007f_ffff) } else { h };
    }
    out
}

/// The model of one level: row-major texels (16 bytes each as four words).
type Level = Vec<[u32; 4]>;

fn model(float: bool, w: u32, h: u32, levels: u32) -> Vec<Level> {
    (0..levels)
        .map(|l| {
            let (lw, lh) = level_dims(w, h, l);
            (0..lh).flat_map(|y| (0..lw).map(move |x| texel(float, l, x, y, 0))).collect()
        })
        .collect()
}

fn bytes_of(texels: &[[u32; 4]]) -> Vec<u8> {
    texels.iter().flatten().flat_map(|w| w.to_le_bytes()).collect()
}

fn upload(class: &(&str, u8, u32, &str), w: u32, h: u32, m: &[Level]) {
    let (ty, fmt) = if class.1 == GLES_TEXFMT_RGBA32F {
        (GL_FLOAT, GL_RGBA)
    } else if class.1 == GLES_TEXFMT_RGBA32I {
        (GL_INT, GL_RGBA_INTEGER)
    } else {
        (GL_UNSIGNED_INT, GL_RGBA_INTEGER)
    };
    // SAFETY: plain GL calls; each level's image holds `lw * lh` 16-byte texels.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        for (l, texels) in m.iter().enumerate() {
            let (lw, lh) = level_dims(w, h, l as u32);
            glTexImage2D(
                GL_TEXTURE_2D,
                l as GLint,
                class.2 as GLint,
                lw as GLsizei,
                lh as GLsizei,
                0,
                fmt,
                ty,
                bytes_of(texels).as_ptr().cast(),
            );
        }
    }
    no_error("glTexImage2D");
}

/// A `glTexSubImage2D` of `rect` = (x, y, w, h) at `level`, texels salted
/// with `salt`, applied to the model too.
fn patch(
    class: &(&str, u8, u32, &str),
    m: &mut [Level],
    (w, h): (u32, u32),
    level: u32,
    rect: (u32, u32, u32, u32),
    salt: u32,
) {
    let (x0, y0, rw, rh) = rect;
    let (lw, _) = level_dims(w, h, level);
    let float = class.1 == GLES_TEXFMT_RGBA32F;
    let mut tex = Vec::new();
    for y in y0..y0 + rh {
        for x in x0..x0 + rw {
            let t = texel(float, level, x, y, salt);
            m[level as usize][(y * lw + x) as usize] = t;
            tex.push(t);
        }
    }
    let (ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGBA),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    // SAFETY: the rectangle lies inside the level; `tex` holds its texels.
    unsafe {
        glTexSubImage2D(
            GL_TEXTURE_2D,
            level as GLint,
            x0 as GLint,
            y0 as GLint,
            rw as GLsizei,
            rh as GLsizei,
            fmt,
            ty,
            bytes_of(&tex).as_ptr().cast(),
        )
    };
    no_error("glTexSubImage2D");
}

/// The draw's descriptor pair against Mesa's, and both planes of every level
/// against the model, byte-exact for every texel of the image (the padding
/// of a tile that a level does not fill is not specified).
fn check_planes(what: &str, class: &(&str, u8, u32, &str), (w, h): (u32, u32), m: &[Level], stream: &[(u32, u32)]) {
    let levels = m.len() as u32;
    let d0 = descriptor(stream, 0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0"));
    let d1 = descriptor(stream, 1).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot 1"));
    let ext = rnndb(class.3);
    for (p, d) in [&d0, &d1].into_iter().enumerate() {
        let c0 = d[GC_TEXDESC_CONFIG0 as usize / 4];
        let c1 = d[GC_TEXDESC_CONFIG1 as usize / 4];
        assert_eq!((c0 >> 13) & 0x1f, 0, "{what}: plane {p}: no CONFIG0 format");
        assert_eq!((c0 >> 20) & 3, GC_TEX_ADDRESSING_TILED, "{what}: plane {p}: TILED");
        assert_eq!(c1 & 0x3f, ext, "{what}: plane {p}: {}", class.3);
        assert_eq!(d[GC_TEXDESC_CONFIG2 as usize / 4], GC_TEXDESC_CONFIG2_BASE, "{what}: plane {p}: no sign bit");
        assert_eq!(d[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(w) | texdesc_size_height(h), "{what}: size");
        assert_eq!(
            d[GC_TEXDESC_LINEAR_STRIDE as usize / 4],
            align4(w) * 8,
            "{what}: plane {p}: a row of G32R32 texels"
        );
        assert_eq!(d[GC_TEXDESC_SLICE as usize / 4], align4(w) * align4(h) * 8, "{what}: plane {p}: one plane's layer");
    }
    /* the companion is a copy but for the level addresses (the sixteen words at 0) */
    for i in 16..d0.len() {
        assert_eq!(d0[i], d1[i], "{what}: descriptor word {i}: the companion is a copy");
    }
    assert_eq!(d0[0] as u64 % 4096, 0, "{what}: the copy starts on a page");
    for l in 0..16u32 {
        let (a0, a1) = (d0[texdesc_lod_addr(l) as usize / 4], d1[texdesc_lod_addr(l) as usize / 4]);
        if l >= levels {
            assert_eq!(a0, a1, "{what}: level {l} is unused in both");
            continue;
        }
        let (lw, lh) = level_dims(w, h, l);
        let plane = align4(lw) * align4(lh) * 8;
        assert_eq!(a1, a0 + plane, "{what}: level {l}: plane 1 half-way into the level");
        assert_eq!(a0 as usize % GC_TEXDESC_LOD_ALIGN, 0, "{what}: level {l}: 64-byte level");
        if l + 1 < levels {
            assert!(d0[texdesc_lod_addr(l + 1) as usize / 4] >= a0 + 2 * plane, "{what}: level {l}: overlaps the next");
        }
        let (p0, p1) = (words_at(u64::from(a0), plane as usize / 4), words_at(u64::from(a1), plane as usize / 4));
        for y in 0..lh {
            for x in 0..lw {
                let tile = (y / 4) * (align4(lw) / 4) + x / 4;
                let at = (tile * 16 + (y % 4) * 4 + x % 4) as usize * 2;
                let want = m[l as usize][(y * lw + x) as usize];
                assert_eq!([p0[at], p0[at + 1]], want[..2], "{what}: level {l}: R,G of ({x}, {y}) in plane 0");
                assert_eq!([p1[at], p1[at + 1]], want[2..], "{what}: level {l}: B,A of ({x}, {y}) in plane 1");
            }
        }
    }
}

/// Did the GL core store the class as planes? RGBA32F is held as halves by
/// `sized_format` (src/gl/texture.rs, `GL_RGBA32F => ... GLES_TEXFMT_RGBA16F`)
/// whatever the mask says, so its descriptor is not a G32R32 one even with
/// the class open. The test says so and checks nothing for it until it is.
fn stored_as_planes(class: &(&str, u8, u32, &str), stream: &[(u32, u32)]) -> bool {
    let d = descriptor(stream, 0).unwrap_or_else(|| panic!("{}: no descriptor in slot 0", class.0));
    let planes = d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f == rnndb(class.3);
    if !planes {
        eprintln!("{}: the GL core does not store the class as planes (texture.rs sized_format); not checked", class.0);
    }
    planes
}

/// The sizes: non-square, not tile-aligned, with mips down to 1x1.
const SIZES: [(u32, u32); 4] = [(13, 6), (5, 9), (16, 4), (8, 8)];

fn levels_of(w: u32, h: u32) -> u32 {
    32 - w.max(h).leading_zeros()
}

/// RGBA32F, RGBA32UI and RGBA32I uploads write plane 0 = (R,G) and plane 1 =
/// (B,A) in the tiled layout the descriptors address, for every size and
/// level. While the classes are closed, the GL core stores no 128-bit class
/// for this back end, which this checks instead.
#[test]
fn an_uploaded_128_bit_texture_is_two_tiled_planes() {
    for class in &WIDE {
        if !open(class.1) {
            eprintln!("{}: not in GC_SAMPLED_TEXFMTS; the plane upload is not exercised (wide128-flip)", class.0);
            assert!(!yakogl::vivante::texture::wide128_sampled(&GlesTextureBind {
                format: class.1,
                ..Default::default()
            }));
            continue;
        }
        let sampler = match class.1 {
            GLES_TEXFMT_RGBA32F => "sampler2D",
            GLES_TEXFMT_RGBA32I => "isampler2D",
            _ => "usampler2D",
        };
        for (w, h) in SIZES {
            on_gc7000(|| {
                let m = model(class.1 == GLES_TEXFMT_RGBA32F, w, h, levels_of(w, h));
                upload(class, w, h, &m);
                let s = draw(program(sampler));
                if !stored_as_planes(class, &s) {
                    return;
                }
                check_planes(&format!("{} {w}x{h}", class.0), class, (w, h), &m, &s);
            });
        }
    }
}

/// A sub-image update changes exactly its texels, in both planes, at level 0
/// and at level 1; every other texel of every level still equals the model.
#[test]
fn a_sub_image_update_touches_only_its_texels_in_both_planes() {
    for class in &WIDE {
        if !open(class.1) {
            eprintln!("{}: not in GC_SAMPLED_TEXFMTS; the sub-image update is not exercised (wide128-flip)", class.0);
            continue;
        }
        let sampler = match class.1 {
            GLES_TEXFMT_RGBA32F => "sampler2D",
            GLES_TEXFMT_RGBA32I => "isampler2D",
            _ => "usampler2D",
        };
        let (w, h) = (13u32, 6u32);
        on_gc7000(|| {
            let mut m = model(class.1 == GLES_TEXFMT_RGBA32F, w, h, levels_of(w, h));
            upload(class, w, h, &m);
            let p = program(sampler);
            let what = |s: &str| format!("{} {s}", class.0);
            let s = draw(p);
            if !stored_as_planes(class, &s) {
                return;
            }
            check_planes(&what("before"), class, (w, h), &m, &s);

            /* crosses the tile columns at 4 and 8 and the row at 4 */
            patch(class, &mut m, (w, h), 0, (3, 1, 7, 4), 1);
            check_planes(&what("after a level 0 patch"), class, (w, h), &m, &draw(p));
            /* a single texel at the last column and row */
            patch(class, &mut m, (w, h), 0, (12, 5, 1, 1), 2);
            check_planes(&what("after a corner texel"), class, (w, h), &m, &draw(p));
            /* level 1 is 6x3 */
            patch(class, &mut m, (w, h), 1, (1, 0, 4, 3), 3);
            check_planes(&what("after a level 1 patch"), class, (w, h), &m, &draw(p));
        });
    }
}
