// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): sampler uniforms and units that do not map one to one,
//! on RGBA32F / RGBA32UI / RGBA32I textures (two G32R32 planes, B,A on a
//! companion slot; docs/notes/gc-128bit-texture-sampling.md).
//!
//! The plane key is per binding (the program's n-th sampler), the texture
//! per unit the binding's uniform is set to. tests/es3_wide128_slots_gc.rs
//! pins the slot arithmetic; this file pins what the key, the companion
//! slots and the submitted descriptors do when the two do not line up:
//!
//! - (a) two `sampler2D` uniforms set to the same RGBA32F unit, both read in
//!   one fragment shader (and the same with two `usampler2D` on one RGBA32UI
//!   unit);
//! - (b) a `sampler2D s[3]` array read at constant indices with one RGBA32F
//!   element between RGBA8 elements, and an `isampler2D s[3]` with one
//!   RGBA32I element between RGBA8I elements;
//! - (c) one RGBA32UI unit read by the vertex and the fragment stage of one
//!   draw;
//! - (d) an `isampler2D` and a `usampler2D` on RGBA32I and RGBA32UI units in
//!   one draw.
//!
//! Which bindings are two planes is read from the draw (the G32R32 word in
//! the binding's descriptor), not from the mask, and the expected key and
//! companion slots follow from it: GC_SAMPLED_TEXFMTS holds the three
//! classes, but the GL core still holds GL_RGBA32F as halves (src/gl/texture.rs
//! `sized_format`), so an RGBA32F binding is one descriptor that is no plane
//! and has no key, said on stderr; once the core stores it as planes the
//! same tests check its pair. The integer classes are two planes exactly
//! when the mask holds them.
//!
//! Each case runs twice: with the mask as built, and with the three classes
//! taken out of it (`backend::TEST_HIDDEN_SAMPLED`), the closed state: the
//! integer uploads refused, no key, no G32R32 descriptor in any slot.
//!
//! Not in a `report-es2` build: the ES 2.0 surface has no 128-bit formats
//! and no GLSL ES 3.00.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_uniforms_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::atomic::Ordering;
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

/// The three 128-bit classes, as mask bits.
const WIDE_BITS: u64 =
    gles_texfmt_bit(GLES_TEXFMT_RGBA32F) | gles_texfmt_bit(GLES_TEXFMT_RGBA32UI) | gles_texfmt_bit(GLES_TEXFMT_RGBA32I);

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context, the 128-bit classes out of the sampled mask if `closed`.
fn on_gc7000(closed: bool, case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    TEST_HIDDEN_SAMPLED.store(if closed { WIDE_BITS } else { 0 }, Ordering::Relaxed);
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
    TEST_HIDDEN_SAMPLED.store(0, Ordering::Relaxed);
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
    let src = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [src.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

fn link(vs: &str, fs: &str) -> GLuint {
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

const PLAIN_VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The words of slot `slot`'s descriptor in the last kick, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/// One quad with program `p`, each `(uniform, unit)` set: the last kick's writes.
fn draw(p: GLuint, uniforms: &[(&str, GLint)]) -> Vec<(u32, u32)> {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        for (name, unit) in uniforms {
            let n = CString::new(*name).unwrap();
            let loc = glGetUniformLocation(p, n.as_ptr());
            assert!(loc >= 0, "uniform {name}");
            glUniform1i(loc, *unit);
        }
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

/* ---- the textures ------------------------------------------------------- */

/// A texture class of the cases: its name, the back end's class, the GL
/// internal format, the Mesa word of its planes (if it is a 128-bit class)
/// and the GLSL sampler type that reads it.
struct Class {
    name: &'static str,
    gles: u8,
    internal: GLenum,
    mesa: Option<&'static str>,
    sampler: &'static str,
}

const RGBA8: Class =
    Class { name: "RGBA8", gles: GLES_TEXFMT_RGBA8, internal: GL_RGBA8, mesa: None, sampler: "sampler2D" };
const RGBA8I: Class =
    Class { name: "RGBA8I", gles: GLES_TEXFMT_RGBA8I, internal: GL_RGBA8I, mesa: None, sampler: "isampler2D" };
const RGBA8UI: Class =
    Class { name: "RGBA8UI", gles: GLES_TEXFMT_RGBA8UI, internal: GL_RGBA8UI, mesa: None, sampler: "usampler2D" };
const RGBA32F: Class = Class {
    name: "RGBA32F",
    gles: GLES_TEXFMT_RGBA32F,
    internal: GL_RGBA32F,
    mesa: Some("TEXTURE_FORMAT_EXT_G32R32F"),
    sampler: "sampler2D",
};
const RGBA32UI: Class = Class {
    name: "RGBA32UI",
    gles: GLES_TEXFMT_RGBA32UI,
    internal: GL_RGBA32UI,
    mesa: Some("TEXTURE_FORMAT_EXT_G32R32I"),
    sampler: "usampler2D",
};
const RGBA32I: Class = Class {
    name: "RGBA32I",
    gles: GLES_TEXFMT_RGBA32I,
    internal: GL_RGBA32I,
    mesa: Some("TEXTURE_FORMAT_EXT_G32R32I"),
    sampler: "isampler2D",
};

/// Is the class in the back end's sampled mask (hidden ones taken out)?
fn open(c: &Class) -> bool {
    sampled_texfmts() & gles_texfmt_bit(c.gles) != 0
}

/// A 4x4 level-0-only texture of `c` on unit `unit`, nearest filtering.
/// Texel (x, y) channel k holds `seed + 4 * (y * 4 + x) + k` (as a float
/// of that value for RGBA32F). A closed integer 128-bit class is refused
/// (GL_INVALID_OPERATION) and leaves the unit incomplete.
fn bind_tex(unit: u32, c: &Class, seed: u32) {
    let (fmt, ty) = match c.gles {
        GLES_TEXFMT_RGBA8 => (GL_RGBA, GL_UNSIGNED_BYTE),
        GLES_TEXFMT_RGBA8I => (GL_RGBA_INTEGER, GL_BYTE),
        GLES_TEXFMT_RGBA8UI => (GL_RGBA_INTEGER, GL_UNSIGNED_BYTE),
        GLES_TEXFMT_RGBA32F => (GL_RGBA, GL_FLOAT),
        GLES_TEXFMT_RGBA32I => (GL_RGBA_INTEGER, GL_INT),
        _ => (GL_RGBA_INTEGER, GL_UNSIGNED_INT),
    };
    let bytes: Vec<u8> = if c.mesa.is_none() {
        (0..64u32).map(|i| (seed + i) as u8).collect()
    } else {
        (0..64u32)
            .flat_map(|i| {
                let v = seed + i;
                if c.gles == GLES_TEXFMT_RGBA32F {
                    (v as f32).to_le_bytes()
                } else {
                    v.to_le_bytes()
                }
            })
            .collect()
    };
    // SAFETY: plain GL calls; the image holds 4x4 texels of the class.
    unsafe {
        glActiveTexture(GL_TEXTURE0 + unit);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, c.internal as GLint, 4, 4, 0, fmt, ty, bytes.as_ptr().cast());
    }
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    if c.gles != GLES_TEXFMT_RGBA32F && c.mesa.is_some() && !open(c) {
        assert_eq!(e, GL_INVALID_OPERATION, "{} on unit {unit}, closed: the upload is refused", c.name);
    } else {
        assert_eq!(e, GL_NO_ERROR, "{} on unit {unit}", c.name);
    }
}

/* ---- what a draw is expected to submit ---------------------------------- */

/// The plane key of a stage from `(binding, companion)` pairs; the companion
/// is the slot relative to the stage's first slot.
fn key_of(pairs: &[(u8, u8)]) -> Wide128Planes {
    pairs.iter().fold(Wide128Planes::NONE, |k, &(b, c)| k.with(b, c))
}

/// The plane key of the draw: `keys` = [vertex, fragment] pairs.
fn assert_key(what: &str, p: GLuint, keys: [&[(u8, u8)]; 2]) {
    let (_, planes) = wide128_draw_state(p).unwrap_or_else(|| panic!("{what}: the draw asked for no key"));
    assert_eq!(planes, [key_of(keys[0]), key_of(keys[1])], "{what}: the plane key [vertex, fragment]");
}

/// Whether slot `slot` of the stream holds a G32R32 plane descriptor.
fn is_plane(stream: &[(u32, u32)], slot: u32) -> bool {
    descriptor(stream, slot).is_some_and(|d| {
        let ext = d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f;
        ext == rnndb("TEXTURE_FORMAT_EXT_G32R32F") || ext == rnndb("TEXTURE_FORMAT_EXT_G32R32I")
    })
}

/// The words texel (0, 0) of a `bind_tex` texture holds.
fn first_texel(c: &Class, seed: u32) -> [u32; 4] {
    core::array::from_fn(|k| {
        let v = seed + k as u32;
        if c.gles == GLES_TEXFMT_RGBA32F {
            (v as f32).to_bits()
        } else {
            v
        }
    })
}

/// Slots `own` and `comp` are the two planes of `c`'s texture seeded `seed`:
/// both the G32R32 word of the class, TILED, the companion a copy of the
/// first but for the level address, which is half-way into the 4x4 level
/// (128 bytes a plane), and the texels the planes hold at (0, 0): R,G in
/// plane 0, B,A in plane 1.
fn check_pair(what: &str, stream: &[(u32, u32)], own: u32, comp: u32, c: &Class, seed: u32) {
    let d0 = descriptor(stream, own).unwrap_or_else(|| panic!("{what}: no descriptor in slot {own}"));
    let d1 = descriptor(stream, comp).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot {comp}"));
    let ext = rnndb(c.mesa.expect("a 128-bit class"));
    for (slot, d) in [(own, &d0), (comp, &d1)] {
        let c0 = d[GC_TEXDESC_CONFIG0 as usize / 4];
        assert_eq!((c0 >> 13) & 0x1f, 0, "{what}: slot {slot}: no CONFIG0 format");
        assert_eq!((c0 >> 20) & 3, GC_TEX_ADDRESSING_TILED, "{what}: slot {slot}: TILED");
        assert_eq!(d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f, ext, "{what}: slot {slot}: {}", c.name);
        assert_eq!(d[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(4) | texdesc_size_height(4), "{what}: size");
    }
    for i in 16..d0.len() {
        assert_eq!(d0[i], d1[i], "{what}: slots {own}/{comp}: descriptor word {i}: the companion is a copy");
    }
    let lod0 = texdesc_lod_addr(0) as usize / 4;
    assert_eq!(d1[lod0], d0[lod0] + 4 * 4 * 8, "{what}: slots {own}/{comp}: plane 1 half-way into the level");
    let want = first_texel(c, seed);
    assert_eq!(words_at(u64::from(d0[lod0]), 2), want[..2], "{what}: slot {own}: R,G of (0, 0)");
    assert_eq!(words_at(u64::from(d1[lod0]), 2), want[2..], "{what}: slot {comp}: B,A of (0, 0)");
}

/// Slot `a` and slot `b` hold the very same descriptor (one texture).
fn same_descriptor(what: &str, stream: &[(u32, u32)], a: u32, b: u32) {
    let da = descriptor(stream, a).unwrap_or_else(|| panic!("{what}: no descriptor in slot {a}"));
    let db = descriptor(stream, b).unwrap_or_else(|| panic!("{what}: no descriptor in slot {b}"));
    assert_eq!(da, db, "{what}: slots {a} and {b} read one texture");
}

/// A sampled binding of a stage: the binding (its slot relative to the
/// stage's first), the class and the seed of its unit's texture.
type Binding<'a> = (u8, &'a Class, u32);

/// The bindings of a stage whose first slot is `base` and which owns
/// `slots` slots: which of them the draw sampled as two planes, read from
/// each binding's descriptor and held against the mask, and their
/// `(binding, companion)` pairs. A binding of two planes has its companion at
/// `slots + rank` (relative to `base`), its rank in binding order among the
/// stage's bindings of two planes, and its pair is checked. In the last
/// stage of the slots (`last`), the slot past its last companion holds no
/// plane; past the fragment stage's companions the vertex stage's slots
/// begin.
fn planes_of(what: &str, s: &[(u32, u32)], base: u32, slots: u8, bindings: &[Binding], last: bool) -> Vec<(u8, u8)> {
    let mut pairs = Vec::new();
    for &(b, c, seed) in bindings {
        let slot = base + u32::from(b);
        let planes = is_plane(s, slot);
        let what = format!("{what}: binding {b} ({}, slot {slot})", c.name);
        if c.mesa.is_none() {
            assert!(!planes, "{what}: not a 128-bit class, not a plane");
            assert!(descriptor(s, slot).is_some(), "{what}: has a descriptor");
            continue;
        }
        if c.gles == GLES_TEXFMT_RGBA32F {
            assert!(!planes || open(c), "{what}: planes only when the mask holds the class");
            if open(c) && !planes {
                eprintln!(
                    "{what}: the GL core stores GL_RGBA32F as halves (texture.rs sized_format); no pair to check"
                );
            }
        } else {
            assert_eq!(planes, open(c), "{what}: two planes exactly when the mask holds the class");
        }
        if !open(c) {
            eprintln!("{what}: out of the sampled mask; the closed state is checked");
        }
        if planes {
            let comp = slots + pairs.len() as u8;
            check_pair(&what, s, slot, base + u32::from(comp), c, seed);
            pairs.push((b, comp));
        }
    }
    let past = base + u32::from(slots) + pairs.len() as u32;
    assert!(!last || !is_plane(s, past), "{what}: slot {past} past the {} companion(s) holds no plane", pairs.len());
    pairs
}

/// With the classes out of the mask: no G32R32 descriptor in any slot.
fn assert_no_plane(what: &str, s: &[(u32, u32)]) {
    for slot in 0..16 {
        assert!(!is_plane(s, slot), "{what}: slot {slot} holds a G32R32 plane");
    }
}

const F_FS: &str =
    "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp isampler2D;\nprecision highp usampler2D;\n";

/* ---- (a) two sampler uniforms on one unit ------------------------------- */

/// Two uniforms `a`, `b` of `c`'s sampler type, `a` read at (0, 0) and `b`
/// at (1, 0).
fn two_uniform_fs(c: &Class) -> String {
    let t = c.sampler;
    format!(
        "{F_FS}uniform {t} a;\nuniform {t} b;\nout vec4 o;\n\
         void main() {{ o = vec4(texelFetch(a, ivec2(0), 0)) + vec4(texelFetch(b, ivec2(1, 0), 0)); }}\n"
    )
}

/// `a` and `b` both set to unit 0 holding a 128-bit texture: two bindings,
/// each keyed, each with its own companion (slots 2 and 3 after the stage's
/// two slots), and the descriptors of bindings 0 and 1 (and of the two
/// companions) are the same bytes: one texture.
#[test]
fn two_sampler_uniforms_on_one_rgba32_unit() {
    for closed in [false, true] {
        for c in [&RGBA32F, &RGBA32UI] {
            let what = format!("a {} closed={closed}", c.name);
            on_gc7000(closed, || {
                bind_tex(0, c, 100);
                let p = link(PLAIN_VS, &two_uniform_fs(c));
                let s = draw(p, &[("a", 0), ("b", 0)]);
                let pairs = planes_of(&what, &s, 0, 2, &[(0, c, 100), (1, c, 100)], true);
                assert!(pairs.is_empty() || pairs == [(0, 2), (1, 3)], "{what}: both bindings or neither: {pairs:?}");
                assert_key(&what, p, [&[], &pairs]);
                same_descriptor(&what, &s, 0, 1);
                if !pairs.is_empty() {
                    same_descriptor(&what, &s, 2, 3);
                }
                if closed {
                    assert_no_plane(&what, &s);
                }
            });
        }
    }
}

/// The same program, `b` moved to a narrow unit of the same sampler type:
/// only binding 0 keeps a plane key and its companion is slot 2 (the
/// stage's two slots first); moved back it is the pair again: the key
/// follows the uniforms, not the program.
#[test]
fn moving_one_of_two_uniforms_to_a_narrow_unit_changes_the_key() {
    for closed in [false, true] {
        for (c, narrow) in [(&RGBA32F, &RGBA8), (&RGBA32UI, &RGBA8UI)] {
            let what = format!("a2 {} closed={closed}", c.name);
            on_gc7000(closed, || {
                bind_tex(0, c, 100);
                bind_tex(1, narrow, 7);
                let p = link(PLAIN_VS, &two_uniform_fs(c));
                let s = draw(p, &[("a", 0), ("b", 1)]);
                let one =
                    planes_of(&format!("{what} b on {}", narrow.name), &s, 0, 2, &[(0, c, 100), (1, narrow, 7)], true);
                assert!(one.is_empty() || one == [(0, 2)], "{what}: binding 0 alone, companion slot 2: {one:?}");
                assert_key(&what, p, [&[], &one]);

                let s = draw(p, &[("a", 0), ("b", 0)]);
                let what = format!("{what} b back");
                let both = planes_of(&what, &s, 0, 2, &[(0, c, 100), (1, c, 100)], true);
                assert_key(&what, p, [&[], &both]);
                let want: &[(u8, u8)] = if one.is_empty() { &[] } else { &[(0, 2), (1, 3)] };
                assert_eq!(both, want, "{what}: the same texture on unit 0 in both draws");
                if closed {
                    assert_no_plane(&what, &s);
                }
            });
        }
    }
}

/* ---- (b) a sampler array with one RGBA32 element ------------------------ */

/// `uniform <sampler> s[3]` read at constant indices, units 0 and 2 `side`
/// and unit 1 `wide`: only element 1 may be keyed, its companion the first
/// slot after the array's three; elements 0 and 2 stay ordinary
/// descriptors.
fn array_case(tag: &str, wide: &Class, side: &Class) {
    let t = wide.sampler;
    let fs = format!(
        "{F_FS}uniform {t} s[3];\nout vec4 o;\nvoid main() {{ o = vec4(texelFetch(s[0], ivec2(0), 0)) + \
         vec4(texelFetch(s[1], ivec2(0), 0)) + vec4(texelFetch(s[2], ivec2(0), 0)); }}\n"
    );
    for closed in [false, true] {
        let what = format!("{tag} closed={closed}");
        on_gc7000(closed, || {
            bind_tex(0, side, 1);
            bind_tex(1, wide, 200);
            bind_tex(2, side, 3);
            let p = link(PLAIN_VS, &fs);
            let s = draw(p, &[("s[0]", 0), ("s[1]", 1), ("s[2]", 2)]);
            let pairs = planes_of(&what, &s, 0, 3, &[(0, side, 1), (1, wide, 200), (2, side, 3)], true);
            assert!(pairs.is_empty() || pairs == [(1, 3)], "{what}: element 1 alone, companion slot 3: {pairs:?}");
            assert_key(&what, p, [&[], &pairs]);
            if closed {
                assert_no_plane(&what, &s);
            }
        });
    }
}

#[test]
fn a_float_sampler_array_with_one_rgba32f_element() {
    array_case("b float", &RGBA32F, &RGBA8);
}

#[test]
fn an_integer_sampler_array_with_one_rgba32i_element() {
    array_case("b int", &RGBA32I, &RGBA8I);
}

/* ---- (c) one unit in both stages ---------------------------------------- */

/// One `usampler2D s` (declared in both stages, so one uniform) on an
/// RGBA32UI unit, read by the vertex and the fragment stage: each stage has
/// its own binding 0 and key. The fragment stage owns slot 0 and its
/// companion slot 1; the vertex stage's slots start after them
/// (`VS_SAMPLER_BASE` 2), its own slot first and its companion after,
/// key-relative companion 1.
#[test]
fn one_rgba32ui_unit_in_the_vertex_and_the_fragment_stage() {
    let vs = "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp usampler2D s;\nin vec4 p;\n\
              flat out uvec4 v;\nvoid main() { gl_Position = p; v = texelFetch(s, ivec2(1, 0), 0); }\n";
    let fs = format!(
        "{F_FS}uniform highp usampler2D s;\nflat in uvec4 v;\nout vec4 o;\n\
         void main() {{ o = vec4(texelFetch(s, ivec2(0), 0)) + vec4(v); }}\n"
    );
    for closed in [false, true] {
        let what = format!("c closed={closed}");
        on_gc7000(closed, || {
            bind_tex(0, &RGBA32UI, 400);
            let p = link(vs, &fs);
            let s = draw(p, &[("s", 0)]);
            let fs_pairs = planes_of(&format!("{what} fragment"), &s, 0, 1, &[(0, &RGBA32UI, 400)], false);
            let vs_base = 1 + fs_pairs.len() as u32;
            let vs_pairs = planes_of(&format!("{what} vertex"), &s, vs_base, 1, &[(0, &RGBA32UI, 400)], true);
            assert_eq!(fs_pairs.is_empty(), closed, "{what}: the fragment binding is two planes when open");
            assert_eq!(vs_pairs, fs_pairs, "{what}: both stages key binding 0 with companion 1");
            assert_key(&what, p, [&vs_pairs, &fs_pairs]);
            if closed {
                assert_no_plane(&what, &s);
            } else {
                assert_eq!(vs_base, 2, "{what}: the vertex stage starts after the fragment slot and its companion");
                same_descriptor(&what, &s, 0, 2);
                same_descriptor(&what, &s, 1, 3);
            }
        });
    }
}

/* ---- (d) isampler2D and usampler2D in one draw -------------------------- */

const D_FS: &str = "uniform isampler2D i;\nuniform usampler2D u;\nout vec4 o;\n\
                    void main() { o = vec4(texelFetch(i, ivec2(0), 0)) + vec4(texelFetch(u, ivec2(0), 0)); }\n";

/// An `isampler2D` `i` on RGBA32I unit `iu` and a `usampler2D` `u` on
/// RGBA32UI unit `uu`: two keyed bindings with companions 2 and 3 in
/// binding order whatever the units, each pair carrying its own texture's
/// words; the integer plane word is the same for both classes.
fn int_uint_case(tag: &str, iu: u32, uu: u32) {
    let fs = format!("{F_FS}{D_FS}");
    let (iseed, useed) = (500 + 100 * iu, 500 + 100 * uu);
    for closed in [false, true] {
        let what = format!("{tag} closed={closed}");
        on_gc7000(closed, || {
            bind_tex(iu, &RGBA32I, iseed);
            bind_tex(uu, &RGBA32UI, useed);
            let p = link(PLAIN_VS, &fs);
            let s = draw(p, &[("i", iu as GLint), ("u", uu as GLint)]);
            let pairs = planes_of(&what, &s, 0, 2, &[(0, &RGBA32I, iseed), (1, &RGBA32UI, useed)], true);
            let want: &[(u8, u8)] = if closed { &[] } else { &[(0, 2), (1, 3)] };
            assert_eq!(pairs, want, "{what}: the companions");
            assert_key(&what, p, [&[], &pairs]);
            if closed {
                assert_no_plane(&what, &s);
            }
        });
    }
}

#[test]
fn an_isampler_and_a_usampler_on_rgba32i_and_rgba32ui() {
    int_uint_case("d", 0, 1);
}

/// The same with the units the other way round (`i` on unit 1, `u` on unit
/// 0): the key is the same, the descriptors follow the units' textures.
#[test]
fn the_isampler_and_the_usampler_swap_units() {
    int_uint_case("d swapped", 1, 0);
}
