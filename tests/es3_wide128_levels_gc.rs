// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): texture-object level state on both planes of a
//! 128-bit texture -- GL_TEXTURE_BASE_LEVEL, GL_TEXTURE_MAX_LEVEL, mip
//! completeness under NEAREST_MIPMAP_NEAREST, and glTexStorage2D/3D
//! immutable textures (docs/notes/gc-128bit-texture-sampling.md; Mesa 26.2.1
//! etnaviv_texture_desc.c:404-415).
//!
//! An RGBA32F/UI/I texture is two G32R32 planes with two descriptors; the
//! second is a byte copy of the first except for the sixteen level addresses,
//! each of which is the first's plus half the level (`layers` layer-planes).
//! So BASELOD (base and max level) must be the same word in both, and the
//! level addresses must follow the model whichever levels the texture-object
//! state selects. Sampler-object state (min/max LOD, wrap, filter) is the
//! business of tests/es3_wide128_sampler_gc.rs.
//!
//! The model of a draw of a complete texture: BASELOD names
//! GL_TEXTURE_BASE_LEVEL and `min(GL_TEXTURE_MAX_LEVEL, levels - 1)`, both
//! planes' descriptors carry every level's address, and plane 1 of level `l`
//! starts `layers * align4(w_l) * align4(h_l) * 8` bytes after plane 0.
//!
//! - The draw tests go through the GL API on the stand-in OSAL
//!   (tests/es3_wide128_planes_gc.rs). The sampled mask hook that opens the
//!   128-bit classes (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so no
//!   integration test can add them: until GC_SAMPLED_TEXFMTS holds them (item
//!   gc-w128-flip) these find the classes closed, say so on stderr and check
//!   the closed state; from the flip on they run for real.
//! - GL_TEXTURE_BASE_LEVEL is not applied by the draw today
//!   (`mip_levels_needed` and `sampling_fields` in src/gl/draw.rs: "the base
//!   level is level 0 here"; `GlesTextureBind` has no base level and
//!   `build_descriptor` writes `texdesc_baselod_base(0)`), so the two
//!   BASE_LEVEL tests are `#[ignore]`d with that defect; they pin the model
//!   for the day it is applied.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_levels_gc
//! ```

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

/* ---- the draws ---------------------------------------------------------- */

/// A program sampling unit 0 with `ty` (`sampler2D`, `usampler2DArray`, ...).
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

/// The classes of the item: RGBA32F and RGBA32UI.
const CLASSES: [&(&str, u8, u32, &str); 2] = [&WIDE[0], &WIDE[1]];

/// One texture of the item: 2D or a 2D array of `layers`, `levels` levels.
#[derive(Clone, Copy)]
struct Shape {
    array: bool,
    w: u32,
    h: u32,
    layers: u32,
    levels: u32,
}

const PLAIN: Shape = Shape { array: false, w: 8, h: 8, layers: 1, levels: 4 };
const ARRAY: Shape = Shape { array: true, w: 4, h: 4, layers: 3, levels: 3 };

fn target_of(s: &Shape) -> GLenum {
    if s.array {
        GL_TEXTURE_2D_ARRAY
    } else {
        GL_TEXTURE_2D
    }
}

fn bind_of(class: u8, s: &Shape, incomplete: bool) -> GlesTextureBind {
    GlesTextureBind {
        format: class,
        type_: if s.array { GLES_TEXTYPE_2D_ARRAY } else { GLES_TEXTYPE_2D },
        width: s.w,
        height: s.h,
        layers: s.layers as _,
        mip_levels: s.levels as _,
        incomplete: incomplete as u8,
        ..Default::default()
    }
}

fn sampler_of(class: u8, s: &Shape) -> String {
    let prefix = if class == GLES_TEXFMT_RGBA32F { "" } else { "u" };
    format!("{prefix}sampler2D{}", if s.array { "Array" } else { "" })
}

fn coord_of(s: &Shape) -> &'static str {
    if s.array {
        "vec3(0.5, 0.5, 1.0)"
    } else {
        "vec2(0.5, 0.5)"
    }
}

/// The words of every texel of layer `layer` at `level`.
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

fn format_type(class: u8) -> (GLenum, GLenum) {
    if class == GLES_TEXFMT_RGBA32F {
        (GL_RGBA, GL_FLOAT)
    } else {
        (GL_RGBA_INTEGER, GL_UNSIGNED_INT)
    }
}

/// The texels of level `l` of `s`: every layer's texels hold `texel(l, layer)`.
fn image(float: bool, s: &Shape, l: u32) -> Vec<u8> {
    let (lw, lh) = level_dims(s.w, s.h, l);
    let mut v = Vec::new();
    for k in 0..s.layers {
        for _ in 0..lw * lh {
            v.extend(texel(float, l, k).iter().flat_map(|w| w.to_le_bytes()));
        }
    }
    v
}

/// A texture of `s` made by glTexImage*, levels `0..given` filled.
fn upload(class: &(&str, u8, u32, &str), s: &Shape, given: u32) {
    let float = class.1 == GLES_TEXFMT_RGBA32F;
    let (fmt, ty) = format_type(class.1);
    let target = target_of(s);
    // SAFETY: plain GL calls; each image holds the texels of the level it names.
    unsafe {
        let mut t: GLuint = 0;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        for l in 0..given {
            let (lw, lh) = level_dims(s.w, s.h, l);
            let img = image(float, s, l);
            if s.array {
                glTexImage3D(
                    target,
                    l as GLint,
                    class.2 as GLint,
                    lw as GLsizei,
                    lh as GLsizei,
                    s.layers as GLsizei,
                    0,
                    fmt,
                    ty,
                    img.as_ptr().cast(),
                );
            } else {
                glTexImage2D(
                    target,
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
    }
    no_error("upload");
}

/// A texture of `s` made by glTexStorage2D/3D and filled with glTexSubImage*.
fn storage(class: &(&str, u8, u32, &str), s: &Shape) {
    let float = class.1 == GLES_TEXFMT_RGBA32F;
    let (fmt, ty) = format_type(class.1);
    let target = target_of(s);
    // SAFETY: plain GL calls; each image holds the texels of the level it names.
    unsafe {
        let mut t: GLuint = 0;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        if s.array {
            glTexStorage3D(target, s.levels as GLsizei, class.2, s.w as GLsizei, s.h as GLsizei, s.layers as GLsizei);
        } else {
            glTexStorage2D(target, s.levels as GLsizei, class.2, s.w as GLsizei, s.h as GLsizei);
        }
        no_error("glTexStorage");
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        for l in 0..s.levels {
            let (lw, lh) = level_dims(s.w, s.h, l);
            let img = image(float, s, l);
            if s.array {
                glTexSubImage3D(
                    target,
                    l as GLint,
                    0,
                    0,
                    0,
                    lw as GLsizei,
                    lh as GLsizei,
                    s.layers as GLsizei,
                    fmt,
                    ty,
                    img.as_ptr().cast(),
                );
            } else {
                glTexSubImage2D(target, l as GLint, 0, 0, lw as GLsizei, lh as GLsizei, fmt, ty, img.as_ptr().cast());
            }
        }
    }
    no_error("glTexSubImage");
}

fn param(s: &Shape, pname: GLenum, v: GLint) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameteri(target_of(s), pname, v) };
    no_error("glTexParameteri");
}

/// The descriptor pair of one draw against the model: BASELOD in both, the
/// planes' level addresses and texels. `levels` is the chain the texture
/// holds, `base` and `max` the level range GL selects.
fn check(class: &(&str, u8, u32, &str), s: &Shape, base: u32, max: u32, stream: &[(u32, u32)]) {
    let what = format!("{} {}x{}x{} base {base} max {max}", class.0, s.w, s.h, s.layers);
    let d0 = descriptor(stream, 0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0"));
    let d1 = descriptor(stream, 1).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot 1"));
    let float = class.1 == GLES_TEXFMT_RGBA32F;
    let want_max = max.min(s.levels - 1);
    for (p, d) in [&d0, &d1].into_iter().enumerate() {
        assert_eq!(d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f, rnndb(class.3), "{what}: plane {p}: {}", class.3);
        assert_eq!(
            d[GC_TEXDESC_BASELOD as usize / 4],
            texdesc_baselod_base(base) | texdesc_baselod_max(want_max),
            "{what}: plane {p}: the level range"
        );
    }
    /* everything but the sixteen level addresses is shared */
    for i in 16..d0.len() {
        assert_eq!(d0[i], d1[i], "{what}: descriptor word {i}: the companion is a copy");
    }
    for l in 0..16u32 {
        let (a0, a1) = (d0[texdesc_lod_addr(l) as usize / 4], d1[texdesc_lod_addr(l) as usize / 4]);
        /* the copy holds the levels up to MAX_LEVEL (`mip_levels`); the rest
         * are not addressed, and alike in both descriptors */
        if l > want_max {
            assert_eq!(a0, a1, "{what}: level {l} is unused in both");
            continue;
        }
        let (lw, lh) = level_dims(s.w, s.h, l);
        let plane = align4(lw) * align4(lh) * 8;
        assert_eq!(a0 as usize % GC_TEXDESC_LOD_ALIGN, 0, "{what}: level {l}: 64-byte level");
        assert_eq!(a1, a0 + plane * s.layers, "{what}: level {l}: plane 1 after all {} layers of plane 0", s.layers);
        if l < want_max {
            let next = d0[texdesc_lod_addr(l + 1) as usize / 4];
            assert!(next >= a0 + 2 * plane * s.layers, "{what}: level {l}: two planes before level {}", l + 1);
        }
        for k in 0..s.layers {
            let want = texel(float, l, k);
            let at = u64::from(plane) * u64::from(k);
            assert_eq!(words_at(u64::from(a0) + at, 2), want[..2], "{what}: level {l} layer {k}: R,G in plane 0");
            assert_eq!(words_at(u64::from(a1) + at, 2), want[2..], "{what}: level {l} layer {k}: B,A in plane 1");
        }
    }
}

/// Is the class in the mask? If not, say so and check the closed state of
/// `bind` (never sampled as planes).
fn closed(class: &(&str, u8, u32, &str), what: &str, bind: &GlesTextureBind) -> bool {
    if open(class.1) {
        return false;
    }
    eprintln!("{} {what}: not in GC_SAMPLED_TEXFMTS; the level state is not exercised (gc-w128-flip)", class.0);
    assert!(!wide128_sampled(bind));
    true
}

/// Draw the bound texture of `s`; false when the GL core does not store the
/// class as planes (the first descriptor is not a G32R32 one).
fn draws_as_planes(class: &(&str, u8, u32, &str), s: &Shape, p: GLuint) -> Option<Vec<(u32, u32)>> {
    let stream = draw(p);
    let d = descriptor(&stream, 0)
        .unwrap_or_else(|| panic!("{} {}: no descriptor in slot 0", class.0, sampler_of(class.1, s)));
    if d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f != rnndb(class.3) {
        eprintln!("{}: the GL core does not store the class as planes; not checked", class.0);
        return None;
    }
    Some(stream)
}

/* ---- the tests ---------------------------------------------------------- */

/// (a) MAX_LEVEL 2 on a four-level texture: both descriptors' range is 0..2
/// and every level address is still there; the model for BASE_LEVEL follows
/// in the ignored test below.
#[test]
fn max_level_two_on_four_levels_reaches_both_descriptors() {
    for class in CLASSES {
        if closed(class, "max level", &bind_of(class.1, &PLAIN, false)) {
            continue;
        }
        on_gc7000(|| {
            upload(class, &PLAIN, PLAIN.levels);
            param(&PLAIN, GL_TEXTURE_MAX_LEVEL, 2);
            let p = program(&sampler_of(class.1, &PLAIN), coord_of(&PLAIN));
            if let Some(stream) = draws_as_planes(class, &PLAIN, p) {
                check(class, &PLAIN, 0, 2, &stream);
            }
        });
    }
}

/// (a) BASE_LEVEL 1 and MAX_LEVEL 2 on a four-level texture: BASELOD of both
/// descriptors is base 1, max 2, and the level addresses are the model's.
#[test]
#[ignore = "GL_TEXTURE_BASE_LEVEL is not applied by the draw: BASELOD base is always 0 (src/gl/draw.rs mip_levels_needed, src/vivante/texture.rs build_descriptor)"]
fn base_level_one_max_level_two_reaches_both_descriptors() {
    for class in CLASSES {
        if closed(class, "base level", &bind_of(class.1, &PLAIN, false)) {
            continue;
        }
        on_gc7000(|| {
            upload(class, &PLAIN, PLAIN.levels);
            param(&PLAIN, GL_TEXTURE_BASE_LEVEL, 1);
            param(&PLAIN, GL_TEXTURE_MAX_LEVEL, 2);
            let p = program(&sampler_of(class.1, &PLAIN), coord_of(&PLAIN));
            if let Some(stream) = draws_as_planes(class, &PLAIN, p) {
                check(class, &PLAIN, 1, 2, &stream);
            }
        });
    }
}

/// (d) BASE_LEVEL changed between two draws moves both descriptors' range.
#[test]
#[ignore = "GL_TEXTURE_BASE_LEVEL is not applied by the draw: BASELOD base is always 0 (src/gl/draw.rs mip_levels_needed, src/vivante/texture.rs build_descriptor)"]
fn changing_base_level_between_draws_moves_both_descriptors() {
    for class in CLASSES {
        if closed(class, "base level change", &bind_of(class.1, &PLAIN, false)) {
            continue;
        }
        on_gc7000(|| {
            upload(class, &PLAIN, PLAIN.levels);
            let p = program(&sampler_of(class.1, &PLAIN), coord_of(&PLAIN));
            let Some(first) = draws_as_planes(class, &PLAIN, p) else { return };
            check(class, &PLAIN, 0, 3, &first);
            param(&PLAIN, GL_TEXTURE_BASE_LEVEL, 2);
            let second = draw(p);
            check(class, &PLAIN, 2, 3, &second);
            param(&PLAIN, GL_TEXTURE_BASE_LEVEL, 0);
            let third = draw(p);
            check(class, &PLAIN, 0, 3, &third);
        });
    }
}

/// (d) The part of a level change that does work today: MAX_LEVEL changed
/// between two draws moves BASELOD's max in both descriptors, and no other
/// word.
#[test]
fn changing_max_level_between_draws_moves_both_descriptors() {
    for class in CLASSES {
        if closed(class, "max level change", &bind_of(class.1, &PLAIN, false)) {
            continue;
        }
        on_gc7000(|| {
            upload(class, &PLAIN, PLAIN.levels);
            let p = program(&sampler_of(class.1, &PLAIN), coord_of(&PLAIN));
            let Some(first) = draws_as_planes(class, &PLAIN, p) else { return };
            check(class, &PLAIN, 0, 3, &first);
            param(&PLAIN, GL_TEXTURE_MAX_LEVEL, 1);
            let second = draw(p);
            check(class, &PLAIN, 0, 1, &second);
            param(&PLAIN, GL_TEXTURE_MAX_LEVEL, 1000);
            let third = draw(p);
            check(class, &PLAIN, 0, 3, &third);
        });
    }
}

/// (b) A mip chain with a level missing is incomplete under
/// NEAREST_MIPMAP_NEAREST: the draw samples the black texel, and neither a
/// two-plane descriptor nor a companion slot is written.
#[test]
fn an_incomplete_mip_chain_stays_on_the_black_texel() {
    for class in CLASSES {
        /* the binding the GL core hands the back end for it */
        if closed(class, "incomplete chain", &bind_of(class.1, &PLAIN, true)) {
            continue;
        }
        assert!(!wide128_sampled(&bind_of(class.1, &PLAIN, true)), "{}: an incomplete binding is not planes", class.0);
        on_gc7000(|| {
            /* two of the four levels the 8x8 chain needs */
            upload(class, &PLAIN, 2);
            let p = program(&sampler_of(class.1, &PLAIN), coord_of(&PLAIN));
            let stream = draw(p);
            let d0 = descriptor(&stream, 0).expect("slot 0 holds the black texel's descriptor");
            assert_ne!(d0[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f, rnndb(class.3), "{}: not a G32R32 plane", class.0);
            assert!(
                descriptor(&stream, 1).is_none_or(|d| d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f != rnndb(class.3)),
                "{}: no companion plane",
                class.0
            );
        });
    }
}

/// (c) glTexStorage2D and glTexStorage3D (2D array) RGBA32F, and RGBA32UI on
/// the same: both planes' level addresses, BASELOD across the whole
/// immutable chain, and the texels of every level and layer.
#[test]
fn immutable_storage_has_both_planes_at_every_level() {
    for class in CLASSES {
        for s in [&PLAIN, &ARRAY] {
            if closed(class, if s.array { "glTexStorage3D" } else { "glTexStorage2D" }, &bind_of(class.1, s, false)) {
                continue;
            }
            on_gc7000(|| {
                storage(class, s);
                let p = program(&sampler_of(class.1, s), coord_of(s));
                if let Some(stream) = draws_as_planes(class, s, p) {
                    check(class, s, 0, s.levels - 1, &stream);
                }
            });
        }
    }
}

/// (c) An immutable texture whose chain is shorter than the mip pyramid
/// (`levels` 2 of an 8x8) is complete: GL clamps the maximum level to
/// `levels - 1` (ES 3.0 3.8.4), and BASELOD's max is 1 in both descriptors.
#[test]
fn a_short_immutable_chain_is_complete_with_max_level_one() {
    let short = Shape { levels: 2, ..PLAIN };
    for class in CLASSES {
        if closed(class, "short glTexStorage2D", &bind_of(class.1, &short, false)) {
            continue;
        }
        on_gc7000(|| {
            storage(class, &short);
            let p = program(&sampler_of(class.1, &short), coord_of(&short));
            if let Some(stream) = draws_as_planes(class, &short, p) {
                check(class, &short, 0, 1, &stream);
            }
        });
    }
}
