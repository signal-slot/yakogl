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
//!   one fragment shader;
//! - (b) a `sampler2D s[3]` array read at constant indices, with one RGBA32I
//!   element between RGBA8 elements;
//! - (c) one RGBA32UI unit read by the vertex and the fragment stage of one
//!   draw;
//! - (d) an `isampler2D` and a `usampler2D` on RGBA32I and RGBA32UI units in
//!   one draw.
//!
//! The mask hook of the in-crate tests (`backend::TEST_EXTRA_SAMPLED`) is
//! `#[cfg(test)]`, so no integration test can open the 128-bit classes:
//! until GC_SAMPLED_TEXFMTS holds them (item gc-w128-flip) every case
//! checks the closed state instead (no plane key, no G32R32 descriptor) and
//! says so on stderr; from the flip on it checks the planes. Before the flip
//! the open expectations were run with the three classes added to
//! GC_SAMPLED_TEXFMTS locally (not committed).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_uniforms_gc
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
/// internal format, the Mesa word of its planes (if it is a 128-bit class).
struct Class {
    name: &'static str,
    gles: u8,
    internal: GLenum,
    mesa: Option<&'static str>,
}

const RGBA8: Class = Class { name: "RGBA8", gles: GLES_TEXFMT_RGBA8, internal: GL_RGBA8, mesa: None };
const RGBA8I: Class = Class { name: "RGBA8I", gles: GLES_TEXFMT_RGBA8I, internal: GL_RGBA8I, mesa: None };
const RGBA32F: Class = Class {
    name: "RGBA32F",
    gles: GLES_TEXFMT_RGBA32F,
    internal: GL_RGBA32F,
    mesa: Some("TEXTURE_FORMAT_EXT_G32R32F"),
};
const RGBA32UI: Class = Class {
    name: "RGBA32UI",
    gles: GLES_TEXFMT_RGBA32UI,
    internal: GL_RGBA32UI,
    mesa: Some("TEXTURE_FORMAT_EXT_G32R32I"),
};
const RGBA32I: Class = Class {
    name: "RGBA32I",
    gles: GLES_TEXFMT_RGBA32I,
    internal: GL_RGBA32I,
    mesa: Some("TEXTURE_FORMAT_EXT_G32R32I"),
};

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(c: &Class) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(c.gles) as u64) != 0
}

/// A 4x4 level-0-only texture of `c` on unit `unit`, nearest filtering.
/// Texel (x, y) channel k holds `seed + 4 * (y * 4 + x) + k` (as a float
/// of that value for RGBA32F).
fn bind_tex(unit: u32, c: &Class, seed: u32) {
    let (fmt, ty) = match c.gles {
        GLES_TEXFMT_RGBA8 => (GL_RGBA, GL_UNSIGNED_BYTE),
        GLES_TEXFMT_RGBA8I => (GL_RGBA_INTEGER, GL_BYTE),
        GLES_TEXFMT_RGBA32F => (GL_RGBA, GL_FLOAT),
        GLES_TEXFMT_RGBA32I => (GL_RGBA_INTEGER, GL_INT),
        _ => (GL_RGBA_INTEGER, GL_UNSIGNED_INT),
    };
    let bytes: Vec<u8> = if c.gles == GLES_TEXFMT_RGBA8 || c.gles == GLES_TEXFMT_RGBA8I {
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
    if c.gles != GLES_TEXFMT_RGBA32F && c.mesa.is_some() && !open(c) {
        /* a closed integer 128-bit class is not stored: the upload is refused */
        // SAFETY: a plain GL call.
        let e = unsafe { glGetError() };
        assert_eq!(e, GL_INVALID_OPERATION, "{} on unit {unit}, closed", c.name);
        return;
    }
    no_error(&format!("{} on unit {unit}", c.name));
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

/// Closed state: no key, no G32R32 descriptor in any slot.
fn assert_closed(what: &str, p: GLuint, stream: &[(u32, u32)]) {
    assert_key(what, p, [&[], &[]]);
    for slot in 0..16 {
        assert!(!is_plane(stream, slot), "{what}: slot {slot} holds a G32R32 plane of a closed class");
    }
}

fn note_closed(what: &str, classes: &[&Class]) {
    for c in classes.iter().filter(|c| !open(c)) {
        eprintln!("{what}: {} is not in GC_SAMPLED_TEXFMTS; the closed state is checked (gc-w128-flip)", c.name);
    }
}

const F_FS: &str =
    "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp isampler2D;\nprecision highp usampler2D;\n";

/* ---- (a) two sampler uniforms on one unit ------------------------------- */

/// `uniform sampler2D a, b` both set to unit 0 holding an RGBA32F texture:
/// two bindings, each keyed, each with its own companion (slots 2 and 3 after
/// the stage's two slots), and the descriptors of bindings 0 and 1 (and of
/// the two companions) are the same bytes: one texture.
#[test]
fn two_sampler_uniforms_on_one_rgba32f_unit() {
    let fs = format!(
        "{F_FS}uniform sampler2D a;\nuniform sampler2D b;\nout vec4 o;\nvoid main() {{ o = texelFetch(a, ivec2(0), 0) + texelFetch(b, ivec2(1, 0), 0); }}\n"
    );
    note_closed("a", &[&RGBA32F]);
    on_gc7000(|| {
        bind_tex(0, &RGBA32F, 100);
        let p = link(PLAIN_VS, &fs);
        let s = draw(p, &[("a", 0), ("b", 0)]);
        if !open(&RGBA32F) {
            assert_closed("a", p, &s);
            same_descriptor("a closed", &s, 0, 1);
            return;
        }
        assert_key("a", p, [&[], &[(0, 2), (1, 3)]]);
        check_pair("a", &s, 0, 2, &RGBA32F, 100);
        check_pair("a", &s, 1, 3, &RGBA32F, 100);
        same_descriptor("a", &s, 0, 1);
        same_descriptor("a", &s, 2, 3);
    });
}

/// The same program, `b` moved to an RGBA8 unit: only binding 0 keeps a
/// plane key and its companion is slot 2 (the stage's two slots first), and
/// moved back it is the pair again; the key follows the uniforms, not the
/// program.
#[test]
fn moving_one_of_two_uniforms_to_an_rgba8_unit_changes_the_key() {
    let fs = format!(
        "{F_FS}uniform sampler2D a;\nuniform sampler2D b;\nout vec4 o;\nvoid main() {{ o = texelFetch(a, ivec2(0), 0) + texelFetch(b, ivec2(1, 0), 0); }}\n"
    );
    note_closed("a2", &[&RGBA32F]);
    on_gc7000(|| {
        bind_tex(0, &RGBA32F, 100);
        bind_tex(1, &RGBA8, 7);
        let p = link(PLAIN_VS, &fs);
        let s = draw(p, &[("a", 0), ("b", 1)]);
        if !open(&RGBA32F) {
            assert_closed("a2", p, &s);
            return;
        }
        assert_key("a2 b on RGBA8", p, [&[], &[(0, 2)]]);
        check_pair("a2", &s, 0, 2, &RGBA32F, 100);
        assert!(!is_plane(&s, 1), "a2: binding 1 is an RGBA8 texture");
        assert!(descriptor(&s, 3).is_none_or(|_| !is_plane(&s, 3)), "a2: no second companion");

        let s = draw(p, &[("a", 0), ("b", 0)]);
        assert_key("a2 b back on RGBA32F", p, [&[], &[(0, 2), (1, 3)]]);
        check_pair("a2 again", &s, 1, 3, &RGBA32F, 100);
    });
}

/* ---- (b) a sampler array with one RGBA32 element ------------------------ */

/// `uniform sampler2D s[3]` read at constant indices, units 0 and 2 RGBA8
/// and unit 1 RGBA32F: only element 1 is keyed, its companion the first slot
/// after the array's three; elements 0 and 2 stay ordinary descriptors.
#[test]
fn a_float_sampler_array_with_one_rgba32f_element() {
    let fs = format!(
        "{F_FS}uniform sampler2D s[3];\nout vec4 o;\nvoid main() {{ o = texelFetch(s[0], ivec2(0), 0) + texelFetch(s[1], ivec2(0), 0) + texelFetch(s[2], ivec2(0), 0); }}\n"
    );
    note_closed("b", &[&RGBA32F]);
    on_gc7000(|| {
        bind_tex(0, &RGBA8, 1);
        bind_tex(1, &RGBA32F, 200);
        bind_tex(2, &RGBA8, 3);
        let p = link(PLAIN_VS, &fs);
        let s = draw(p, &[("s[0]", 0), ("s[1]", 1), ("s[2]", 2)]);
        if !open(&RGBA32F) {
            assert_closed("b", p, &s);
            return;
        }
        assert_key("b", p, [&[], &[(1, 3)]]);
        check_pair("b", &s, 1, 3, &RGBA32F, 200);
        assert!(!is_plane(&s, 0) && !is_plane(&s, 2), "b: elements 0 and 2 are RGBA8");
        assert!(descriptor(&s, 0).is_some() && descriptor(&s, 2).is_some(), "b: elements 0 and 2 have descriptors");
    });
}

/// The integer array: `isampler2D s[3]`, RGBA8I around an RGBA32I element.
#[test]
fn an_integer_sampler_array_with_one_rgba32i_element() {
    let fs = format!(
        "{F_FS}uniform isampler2D s[3];\nout vec4 o;\nvoid main() {{ o = vec4(texelFetch(s[0], ivec2(0), 0) + texelFetch(s[1], ivec2(0), 0) + texelFetch(s[2], ivec2(0), 0)); }}\n"
    );
    note_closed("b int", &[&RGBA32I]);
    on_gc7000(|| {
        bind_tex(0, &RGBA8I, 1);
        bind_tex(1, &RGBA32I, 300);
        bind_tex(2, &RGBA8I, 3);
        let p = link(PLAIN_VS, &fs);
        let s = draw(p, &[("s[0]", 0), ("s[1]", 1), ("s[2]", 2)]);
        if !open(&RGBA32I) {
            assert_closed("b int", p, &s);
            return;
        }
        assert_key("b int", p, [&[], &[(1, 3)]]);
        check_pair("b int", &s, 1, 3, &RGBA32I, 300);
        assert!(!is_plane(&s, 0) && !is_plane(&s, 2), "b int: elements 0 and 2 are RGBA8I");
    });
}

/* ---- (c) one unit in both stages ---------------------------------------- */

/// One `usampler2D s` (declared in both stages, so one uniform) on an
/// RGBA32UI unit, read by the vertex and the fragment stage: each stage has
/// its own binding 0 and key. The fragment stage owns slots 0 and 1 (its
/// companion); the vertex stage's start at `VS_SAMPLER_BASE` = 2, its own
/// slot first and its companion after, key-relative companion 1.
#[test]
fn one_rgba32ui_unit_in_the_vertex_and_the_fragment_stage() {
    let vs = "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp usampler2D s;\nin vec4 p;\nflat out uvec4 v;\nvoid main() { gl_Position = p; v = texelFetch(s, ivec2(1, 0), 0); }\n";
    let fs = format!(
        "{F_FS}uniform highp usampler2D s;\nflat in uvec4 v;\nout vec4 o;\nvoid main() {{ o = vec4(texelFetch(s, ivec2(0), 0)) + vec4(v); }}\n"
    );
    note_closed("c", &[&RGBA32UI]);
    on_gc7000(|| {
        bind_tex(0, &RGBA32UI, 400);
        let p = link(vs, &fs);
        let s = draw(p, &[("s", 0)]);
        if !open(&RGBA32UI) {
            assert_closed("c", p, &s);
            return;
        }
        assert_key("c", p, [&[(0, 1)], &[(0, 1)]]);
        check_pair("c fragment", &s, 0, 1, &RGBA32UI, 400);
        check_pair("c vertex", &s, 2, 3, &RGBA32UI, 400);
        same_descriptor("c", &s, 0, 2);
        same_descriptor("c", &s, 1, 3);
    });
}

/* ---- (d) isampler2D and usampler2D in one draw -------------------------- */

const D_FS: &str = "uniform isampler2D i;\nuniform usampler2D u;\nout vec4 o;\nvoid main() { o = vec4(texelFetch(i, ivec2(0), 0)) + vec4(texelFetch(u, ivec2(0), 0)); }\n";

/// An `isampler2D` on an RGBA32I unit and a `usampler2D` on an RGBA32UI
/// unit: two keyed bindings with companions 2 and 3, each pair carrying its
/// own texture's words; the integer word is the same for both classes.
#[test]
fn an_isampler_and_a_usampler_on_rgba32i_and_rgba32ui() {
    let fs = format!("{F_FS}{D_FS}");
    note_closed("d", &[&RGBA32I, &RGBA32UI]);
    on_gc7000(|| {
        bind_tex(0, &RGBA32I, 500);
        bind_tex(1, &RGBA32UI, 600);
        let p = link(PLAIN_VS, &fs);
        let s = draw(p, &[("i", 0), ("u", 1)]);
        if !open(&RGBA32I) || !open(&RGBA32UI) {
            assert_closed("d", p, &s);
            return;
        }
        assert_key("d", p, [&[], &[(0, 2), (1, 3)]]);
        check_pair("d i", &s, 0, 2, &RGBA32I, 500);
        check_pair("d u", &s, 1, 3, &RGBA32UI, 600);
    });
}

/// The same program with the units the other way round (`i` on unit 1, `u`
/// on unit 0, fresh textures): the key is the same, the descriptors follow
/// the units' textures.
#[test]
fn the_isampler_and_the_usampler_swap_units() {
    let fs = format!("{F_FS}{D_FS}");
    note_closed("d swapped", &[&RGBA32I, &RGBA32UI]);
    on_gc7000(|| {
        bind_tex(1, &RGBA32I, 700);
        bind_tex(0, &RGBA32UI, 800);
        let p = link(PLAIN_VS, &fs);
        let s = draw(p, &[("i", 1), ("u", 0)]);
        if !open(&RGBA32I) || !open(&RGBA32UI) {
            assert_closed("d swapped", p, &s);
            return;
        }
        assert_key("d swapped", p, [&[], &[(0, 2), (1, 3)]]);
        check_pair("d swapped i", &s, 0, 2, &RGBA32I, 700);
        check_pair("d swapped u", &s, 1, 3, &RGBA32UI, 800);
    });
}
