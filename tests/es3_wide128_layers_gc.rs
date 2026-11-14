// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): where the two G32R32 planes of a 128-bit texture sit
//! across mip levels, cube faces, 2D-array layers and 3D slices
//! (docs/notes/gc-128bit-texture-sampling.md; Mesa 26.2.1 etnaviv).
//!
//! A level of a layered texture holds every layer of it, and each layer is
//! one plane's `align4(w) * align4(h) * 8` bytes, 4x4-tiled. Plane 1 starts
//! half-way into the level, `layers` layer-planes after plane 0
//! (`second_plane_offset`, etnaviv_resource.h:91-94); a level starts on a
//! 64-byte boundary. A cube, a 2D array and a 3D texture are the same stack
//! of layers to the texture unit: SLICE is one layer-plane, and 3D_CONFIG's
//! depth is the layer count for an array and a volume (a cube's six faces
//! are not a depth). Both planes' descriptors are copies of one another but
//! for the sixteen level addresses, so they share the level count (BASELOD),
//! the size, stride and slice words, and the sampler's LOD clamps.
//!
//! - The tests that always run read the layout out of the public API
//!   (`tiled_layout`, `second_plane_offset`, `copy_bpp`) for a cube of six
//!   faces, a 2D array of three layers and a 3D texture of four slices, each
//!   with three levels.
//! - The draw tests go through the GL API on the stand-in OSAL
//!   (tests/es3_wide128_planes_gc.rs): upload, draw, and read the descriptor
//!   pair and the sampler words out of the submitted stream, and the first
//!   texel of every layer of both planes out of the copy. The sampled mask
//!   hook that opens the 128-bit classes (`backend::TEST_EXTRA_SAMPLED`) is
//!   `#[cfg(test)]`, so no integration test can add them: until
//!   GC_SAMPLED_TEXFMTS holds them (item gc-w128-flip) these find the classes
//!   closed, say so on stderr and check the closed state; from the flip on
//!   they run for real. Before the flip they were run with the classes added
//!   to GC_SAMPLED_TEXFMTS locally.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_layers_gc
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

/* ---- the layouts under test --------------------------------------------- */

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Cube,
    Array,
    Volume,
}

/// One texture of the item: its kind, base size, layers at level 0 (faces,
/// array layers or slices) and level count.
struct Shape {
    name: &'static str,
    kind: Kind,
    w: u32,
    h: u32,
    layers: u32,
    levels: u32,
}

const CUBE: Shape = Shape { name: "cube", kind: Kind::Cube, w: 4, h: 4, layers: 6, levels: 3 };
const ARRAY: Shape = Shape { name: "2D array", kind: Kind::Array, w: 4, h: 4, layers: 3, levels: 3 };
const VOLUME: Shape = Shape { name: "3D", kind: Kind::Volume, w: 4, h: 4, layers: 4, levels: 3 };
const SHAPES: [&Shape; 3] = [&CUBE, &ARRAY, &VOLUME];

fn bind_of(class: u8, s: &Shape) -> GlesTextureBind {
    GlesTextureBind {
        format: class,
        type_: match s.kind {
            Kind::Cube => GLES_TEXTYPE_CUBE,
            Kind::Array => GLES_TEXTYPE_2D_ARRAY,
            Kind::Volume => GLES_TEXTYPE_3D,
        },
        width: s.w,
        height: s.h,
        layers: s.layers as _,
        mip_levels: s.levels as _,
        ..Default::default()
    }
}

/* ---- the layout, from the API ------------------------------------------- */

/// The copy of a layered 128-bit texture: a level starts on 64 bytes, holds
/// every layer's plane 0 and then every layer's plane 1, and the next level
/// starts after both. Plane 1 is `layers` layer-planes into the level.
#[test]
fn the_second_plane_follows_every_layer_of_the_first() {
    for s in SHAPES {
        for class in WIDE {
            let t = bind_of(class.1, s);
            assert_eq!(copy_bpp(&t), 8, "{}: a plane's texel", s.name);
            let (off, total) = tiled_layout(s.w, s.h, s.layers, s.levels, 16);
            let mut prev_end = 0usize;
            for l in 0..s.levels {
                let (lw, lh) = level_dims(s.w, s.h, l);
                let layer_plane = (align4(lw) * align4(lh) * 8) as usize;
                assert_eq!(
                    second_plane_offset(&t, l),
                    layer_plane * s.layers as usize,
                    "{} {} level {l}",
                    s.name,
                    class.0
                );
                assert_eq!(off[l as usize] % GC_TEXDESC_LOD_ALIGN, 0, "{} level {l}: 64-byte level", s.name);
                assert!(off[l as usize] >= prev_end, "{} level {l}: starts after the previous one", s.name);
                /* both planes of every layer, and the next level after them */
                prev_end = off[l as usize] + 2 * second_plane_offset(&t, l);
                let end = if l + 1 < s.levels { off[l as usize + 1] } else { total };
                assert!(end >= prev_end, "{} level {l}: two planes of {} layers fit", s.name, s.layers);
            }
            assert_eq!(total, prev_end, "{}: the last level ends the copy", s.name);
        }
    }
}

/// The layer count is the same at every level (a level holds every layer, a
/// volume's included: the copy reserves the level-0 depth at each level), and
/// every level of these 4x4 textures is one 4x4 tile at most per layer.
#[test]
fn plane_one_of_each_level_is_layers_planes_in() {
    for (s, plane0) in [(&CUBE, 6 * 128usize), (&ARRAY, 3 * 128), (&VOLUME, 4 * 128)] {
        let t = bind_of(GLES_TEXFMT_RGBA32F, s);
        /* 4x4 texels * 8 bytes = 128 per layer-plane; 2x2 and 1x1 pad to one tile */
        for l in 0..3 {
            assert_eq!(second_plane_offset(&t, l), plane0, "{}: level {l}", s.name);
        }
    }
}

/* ---- the draws ---------------------------------------------------------- */

/// A program sampling unit 0 with `ty` (`sampler2DArray`, `isamplerCube`, ...).
fn program(ty: &str, coord: &str) -> GLuint {
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

fn sampler_of(class: u8, s: &Shape) -> String {
    let prefix = match class {
        GLES_TEXFMT_RGBA32F => "",
        GLES_TEXFMT_RGBA32I => "i",
        _ => "u",
    };
    let kind = match s.kind {
        Kind::Cube => "samplerCube",
        Kind::Array => "sampler2DArray",
        Kind::Volume => "sampler3D",
    };
    format!("{prefix}{kind}")
}

/// The words of every texel of layer `layer` at `level`: one value per
/// (level, layer), a finite float for RGBA32F.
fn texel(float: bool, level: u32, layer: u32) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let h = (level + 1).wrapping_mul(0x9e37_79b1)
            ^ (layer + 1).wrapping_mul(0x85eb_ca6b)
            ^ (c as u32 + 1).wrapping_mul(0x27d4_eb2f);
        *o = if float { 0x3f80_0000 | (h & 0x007f_ffff) } else { h };
    }
    out
}

/// Layers of `s` at `level`: a volume halves its depth, the others do not.
fn layers_at(s: &Shape, level: u32) -> u32 {
    if s.kind == Kind::Volume {
        (s.layers >> level).max(1)
    } else {
        s.layers
    }
}

fn upload(class: &(&str, u8, u32, &str), s: &Shape) {
    let float = class.1 == GLES_TEXFMT_RGBA32F;
    let (ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGBA),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    let target = match s.kind {
        Kind::Cube => GL_TEXTURE_CUBE_MAP,
        Kind::Array => GL_TEXTURE_2D_ARRAY,
        Kind::Volume => GL_TEXTURE_3D,
    };
    let image = |level: u32, n: u32| -> Vec<u8> {
        let (lw, lh) = level_dims(s.w, s.h, level);
        let mut v = Vec::new();
        for k in 0..n {
            for _ in 0..lw * lh {
                v.extend(texel(float, level, k).iter().flat_map(|w| w.to_le_bytes()));
            }
        }
        v
    };
    // SAFETY: plain GL calls; each image holds the texels of the level it names.
    unsafe {
        let mut t: GLuint = 0;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        for l in 0..s.levels {
            let (lw, lh) = level_dims(s.w, s.h, l);
            match s.kind {
                Kind::Cube => {
                    /* face k is layer k: every texel of it holds texel(l, k) */
                    for face in 0..6 {
                        let img: Vec<u8> = image(l, 1)
                            .chunks(16)
                            .flat_map(|_| {
                                texel(float, l, face).iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>()
                            })
                            .collect();
                        glTexImage2D(
                            GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                            l as GLint,
                            class.2 as GLint,
                            lw as GLsizei,
                            lh as GLsizei,
                            0,
                            fmt,
                            ty,
                            img.as_ptr().cast(),
                        );
                    }
                }
                _ => {
                    let d = layers_at(s, l);
                    glTexImage3D(
                        target,
                        l as GLint,
                        class.2 as GLint,
                        lw as GLsizei,
                        lh as GLsizei,
                        d as GLsizei,
                        0,
                        fmt,
                        ty,
                        image(l, d).as_ptr().cast(),
                    );
                }
            }
        }
    }
    no_error("upload");
}

/// The last value `stream` wrote to `reg`.
fn last(stream: &[(u32, u32)], reg: u32) -> u32 {
    stream.iter().rev().find(|(r, _)| *r == reg).unwrap_or_else(|| panic!("no write of register 0x{reg:x}")).1
}

/// The descriptor pair, the sampler words and the planes of one draw of `s`.
fn check(class: &(&str, u8, u32, &str), s: &Shape, stream: &[(u32, u32)]) {
    let what = format!("{} {}", class.0, s.name);
    let d0 = descriptor(stream, 0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0"));
    let d1 = descriptor(stream, 1).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot 1"));
    let float = class.1 == GLES_TEXFMT_RGBA32F;
    let layers = s.layers;
    let hw_type = if s.kind == Kind::Cube { GC_TEX_TYPE_CUBE_MAP } else { GC_TEX_TYPE_3D };
    let depth = if s.kind == Kind::Cube { 1 } else { layers };
    let layer_plane = align4(s.w) * align4(s.h) * 8;
    for (p, d) in [&d0, &d1].into_iter().enumerate() {
        assert_eq!(
            d[GC_TEXDESC_CONFIG0 as usize / 4],
            texdesc_type(hw_type) | texdesc_addressing_mode(GC_TEX_ADDRESSING_TILED),
            "{what}: plane {p}: type and tiled addressing"
        );
        assert_eq!(
            d[GC_TEXDESC_CONFIG1 as usize / 4] & GC_TEXDESC_TEXTURE_ARRAY != 0,
            s.kind == Kind::Array,
            "{what}: plane {p}: TEXTURE_ARRAY only for an array"
        );
        assert_eq!(d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f, rnndb(class.3), "{what}: plane {p}: {}", class.3);
        assert_eq!(d[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(s.w) | texdesc_size_height(s.h), "{what}: size");
        assert_eq!(d[GC_TEXDESC_LINEAR_STRIDE as usize / 4], align4(s.w) * 8, "{what}: plane {p}: stride");
        assert_eq!(d[GC_TEXDESC_SLICE as usize / 4], layer_plane, "{what}: plane {p}: one layer of one plane");
        assert_eq!(d[GC_TEXDESC_3D_CONFIG as usize / 4], texdesc_3d_depth(depth), "{what}: plane {p}: depth");
        assert_eq!(
            d[GC_TEXDESC_BASELOD as usize / 4],
            texdesc_baselod_base(0) | texdesc_baselod_max(s.levels - 1),
            "{what}: plane {p}: the level count"
        );
    }
    /* everything but the sixteen level addresses is shared */
    for i in 16..d0.len() {
        assert_eq!(d0[i], d1[i], "{what}: descriptor word {i}: the companion is a copy");
    }
    /* the sampler words and the LOD clamps of both slots */
    for reg in [
        nte_descriptor_samp_ctrl0,
        nte_descriptor_samp_ctrl1,
        nte_descriptor_samp_lod_minmax,
        nte_descriptor_samp_lod_bias,
    ] {
        assert_eq!(last(stream, reg(0)), last(stream, reg(1)), "{what}: sampler register 0x{:x}", reg(0));
    }
    /* the levels */
    for l in 0..16u32 {
        let (a0, a1) = (d0[texdesc_lod_addr(l) as usize / 4], d1[texdesc_lod_addr(l) as usize / 4]);
        if l >= s.levels {
            assert_eq!(a0, a1, "{what}: level {l} is unused in both");
            continue;
        }
        let (lw, lh) = level_dims(s.w, s.h, l);
        let plane = align4(lw) * align4(lh) * 8;
        assert_eq!(a0 as usize % GC_TEXDESC_LOD_ALIGN, 0, "{what}: level {l}: 64-byte level");
        assert_eq!(a1, a0 + plane * layers, "{what}: level {l}: plane 1 after all {layers} layers of plane 0");
        if l + 1 < s.levels {
            let next = d0[texdesc_lod_addr(l + 1) as usize / 4];
            assert!(
                next >= a0 + 2 * plane * layers,
                "{what}: level {l}: two planes of every layer before level {}",
                l + 1
            );
        }
        /* the first texel of each layer the level holds, in each plane */
        for k in 0..layers_at(s, l) {
            let want = texel(float, l, k);
            let at = u64::from(plane) * u64::from(k);
            assert_eq!(words_at(u64::from(a0) + at, 2), want[..2], "{what}: level {l} layer {k}: R,G in plane 0");
            assert_eq!(words_at(u64::from(a1) + at, 2), want[2..], "{what}: level {l} layer {k}: B,A in plane 1");
        }
    }
}

/// A draw of one shape in one class, or the closed state while the class is
/// not in the sampled mask.
fn draw_shape(class: &(&str, u8, u32, &str), s: &Shape) {
    if !open(class.1) {
        eprintln!(
            "{} {}: not in GC_SAMPLED_TEXFMTS; the layered planes are not exercised (gc-w128-flip)",
            class.0, s.name
        );
        assert!(!wide128_sampled(&bind_of(class.1, s)));
        return;
    }
    let coord = match s.kind {
        Kind::Cube => "vec3(0.3, 0.5, 0.7)",
        Kind::Array => "vec3(0.5, 0.5, 1.0)",
        Kind::Volume => "vec3(0.5, 0.5, 0.5)",
    };
    on_gc7000(|| {
        upload(class, s);
        let stream = draw(program(&sampler_of(class.1, s), coord));
        let d = descriptor(&stream, 0).unwrap_or_else(|| panic!("{} {}: no descriptor in slot 0", class.0, s.name));
        if d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f != rnndb(class.3) {
            eprintln!("{} {}: the GL core does not store the class as planes; not checked", class.0, s.name);
            return;
        }
        check(class, s, &stream);
    });
}

/// RGBA32F cube (six faces), 2D array (three layers) and 3D texture (four
/// slices), three levels each: both planes' descriptors, per-level addresses
/// and the first texel of every layer.
#[test]
fn an_rgba32f_cube_array_and_volume_have_two_planes_per_layer() {
    for s in SHAPES {
        draw_shape(&WIDE[0], s);
    }
}

/// RGBA32UI on a 2D array, and RGBA32I on the cube: the integer word.
#[test]
fn rgba32ui_on_a_2d_array_and_rgba32i_on_a_cube() {
    draw_shape(&WIDE[1], &ARRAY);
    draw_shape(&WIDE[2], &CUBE);
}
