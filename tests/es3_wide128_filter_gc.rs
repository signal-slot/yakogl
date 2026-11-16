// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): RGBA32F / RGBA32UI / RGBA32I textures under a linear
//! filter are incomplete and stay on the black-texel path (item
//! gc-w128-filter; docs/notes/gc-128bit-texture-sampling.md).
//!
//! ES 3.0 3.8.13: the three 128-bit classes are not filterable (the
//! extension OES_texture_float_linear is not exposed), so a texture of one
//! with a LINEAR mag filter, or a min filter other than NEAREST /
//! NEAREST_MIPMAP_NEAREST, is incomplete and samples (0,0,0,1). The two-plane
//! path (`wide128_sampled` requires `incomplete == 0`) must not give such a
//! binding planes: no 128-bit descriptor in slot 0, no companion in slot 1.
//!
//! - Incompleteness is read from `incomplete_texture_draws()`, which counts
//!   the draws that sampled an incomplete texture; it works whatever the
//!   mask holds.
//! - The plane checks (a G32R32 descriptor in slot 0 and a companion in
//!   slot 1) need the classes in the sampled mask. The mask hook of the
//!   in-crate tests (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so no
//!   integration test can set it (as tests/es3_wide128_planes_gc.rs): until
//!   GC_SAMPLED_TEXFMTS holds the classes (item gc-w128-flip) the plane
//!   checks are skipped with a note on stderr and the completeness checks run
//!   alone; from the flip on they all run (GC_SAMPLED_TEXFMTS holds them
//!   since gc-wide128-flip). The float class is reached through GL_RGB32F,
//!   since GL_RGBA32F itself is still stored as halves.
//! - An isampler/usampler draw runs no shader here (the stand-in OSAL
//!   executes nothing), so the integer (0,0,0,1) result is not read: the
//!   integer classes are pinned by the incomplete flag and the descriptors.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_filter_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::{glBindSampler, glGenSamplers, glSamplerParameteri};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
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

/// The float row is a GL_RGB32F texture: the GL core stores it in the
/// RGBA32F class (two planes, alpha 1), while GL_RGBA32F itself is still
/// held as halves (src/gl/texture.rs `sized_format`), a filterable class.
const WIDE: [(&str, u8, u32, &str); 3] = [
    ("RGBA32F", GLES_TEXFMT_RGBA32F, GL_RGB32F, "TEXTURE_FORMAT_EXT_G32R32F"),
    ("RGBA32UI", GLES_TEXFMT_RGBA32UI, GL_RGBA32UI, "TEXTURE_FORMAT_EXT_G32R32I"),
    ("RGBA32I", GLES_TEXFMT_RGBA32I, GL_RGBA32I, "TEXTURE_FORMAT_EXT_G32R32I"),
];

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    sampled_texfmts() & gles_texfmt_bit(format) != 0
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

/* ---- the probes --------------------------------------------------------- */

const SIDE: u32 = 4;
const LEVELS: u32 = 3;

fn sampler_of(class: u8) -> &'static str {
    match class {
        GLES_TEXFMT_RGBA32F => "sampler2D",
        GLES_TEXFMT_RGBA32I => "isampler2D",
        _ => "usampler2D",
    }
}

/// A 4x4 texture of `class` with a full three-level chain (so a mipmap min
/// filter is complete on the chain alone), bound to unit 0 with the filters
/// given.
fn texture(class: &(&str, u8, u32, &str), min: GLenum, mag: GLenum) {
    let (ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGB),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    let texels = [0u32; (SIDE * SIDE * 4) as usize];
    // SAFETY: plain GL calls; the image of a level is at most 4x4 16-byte texels.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        for l in 0..LEVELS {
            let s = (SIDE >> l) as GLsizei;
            glTexImage2D(GL_TEXTURE_2D, l as GLint, class.2 as GLint, s, s, 0, fmt, ty, texels.as_ptr().cast());
        }
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, mag as GLint);
    }
    no_error(&format!("{} texture", class.0));
}

/// What one draw of unit 0 showed.
#[derive(Debug, PartialEq)]
struct Seen {
    /// the draw sampled an incomplete texture (`incomplete_texture_draws`)
    incomplete: bool,
    /// slot 0 holds the class's G32R32 descriptor
    planes: bool,
    /// slot 1 holds a descriptor (the companion)
    companion: bool,
}

fn probe(class: &(&str, u8, u32, &str), p: GLuint) -> Seen {
    let before = incomplete_texture_draws();
    let s = draw(p);
    let incomplete = incomplete_texture_draws() > before;
    let planes = descriptor(&s, 0).is_some_and(|d| d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f == rnndb(class.3));
    Seen { incomplete, planes, companion: descriptor(&s, 1).is_some() }
}

/// Can the GL core store the class here? RGB32F always (as halves while
/// closed), the integer pair only once open.
fn usable(class: &(&str, u8, u32, &str)) -> bool {
    if open(class.1) {
        return true;
    }
    eprintln!("{}: not in GC_SAMPLED_TEXFMTS; the plane checks are skipped (gc-w128-flip)", class.0);
    class.1 == GLES_TEXFMT_RGBA32F
}

/// While RGBA32F is not in GC_SAMPLED_TEXFMTS the GL core stores GL_RGB32F
/// as RGBA16F, a filterable format, so incomplete_reason() cannot see the
/// non-filterable class and the RGBA32F completeness checks are skipped
/// with a note (item gc-w128-flip); from the flip on they all run.
fn float_filter_visible() -> bool {
    if open(GLES_TEXFMT_RGBA32F) {
        return true;
    }
    eprintln!("RGBA32F: stored as RGBA16F until gc-w128-flip; the linear-filter completeness checks are skipped");
    false
}

/// The verdict on a draw: an incomplete (black) binding never has planes nor
/// a companion; a complete one has two planes when the class is open.
fn verdict(what: &str, incomplete: bool, class: &(&str, u8, u32, &str), seen: Seen) {
    assert_eq!(seen.incomplete, incomplete, "{what}: {seen:?}");
    if incomplete {
        assert!(!seen.planes && !seen.companion, "{what}: no planes for an incomplete texture: {seen:?}");
    } else if open(class.1) {
        assert!(seen.planes && seen.companion, "{what}: two planes for a complete texture: {seen:?}");
    }
}

fn expect(class: &(&str, u8, u32, &str), min: GLenum, mag: GLenum, incomplete: bool) {
    texture(class, min, mag);
    let seen = probe(class, program(sampler_of(class.1)));
    verdict(&format!("{} min 0x{min:x} mag 0x{mag:x}", class.0), incomplete, class, seen);
}

/// (a) RGBA32F: a LINEAR mag filter, or any min filter but the two NEAREST
/// ones, is incomplete.
#[test]
fn rgba32f_under_a_linear_filter_is_incomplete() {
    if !float_filter_visible() {
        return;
    }
    let class = &WIDE[0];
    for (min, mag) in [
        (GL_NEAREST, GL_LINEAR),
        (GL_LINEAR_MIPMAP_NEAREST, GL_NEAREST),
        (GL_LINEAR, GL_NEAREST),
        (GL_NEAREST_MIPMAP_LINEAR, GL_NEAREST),
        (GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR),
    ] {
        on_gc7000(|| expect(class, min, mag, true));
    }
}

/// (b) RGBA32F with NEAREST/NEAREST and NEAREST_MIPMAP_NEAREST is complete
/// and, with the class open, two planes.
#[test]
fn rgba32f_under_nearest_is_complete_with_two_planes() {
    let class = &WIDE[0];
    for min in [GL_NEAREST, GL_NEAREST_MIPMAP_NEAREST] {
        on_gc7000(|| expect(class, min, GL_NEAREST, false));
    }
}

/// (c) RGBA32I / RGBA32UI: the same pins. The (0,0,0,1) an isampler/usampler
/// reads from an incomplete one is not read here (no shader runs on the
/// stand-in OSAL).
#[test]
fn the_integer_classes_follow_the_same_rule() {
    for class in &WIDE[1..] {
        if !usable(class) {
            continue;
        }
        for (min, mag, incomplete) in [
            (GL_NEAREST, GL_NEAREST, false),
            (GL_NEAREST_MIPMAP_NEAREST, GL_NEAREST, false),
            (GL_NEAREST, GL_LINEAR, true),
            (GL_LINEAR, GL_NEAREST, true),
            (GL_LINEAR_MIPMAP_NEAREST, GL_NEAREST, true),
            (GL_NEAREST_MIPMAP_LINEAR, GL_NEAREST, true),
            (GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR, true),
        ] {
            on_gc7000(|| expect(class, min, mag, incomplete));
        }
    }
}

/// (d) A sampler object overrides the texture's filters on its unit only: a
/// LINEAR sampler makes a NEAREST texture incomplete there, a NEAREST
/// sampler makes a LINEAR texture complete, a sampler on another unit
/// changes nothing, and unbinding it restores the texture's own state.
fn sampler_case(class: &(&str, u8, u32, &str)) {
    if !usable(class) {
        return;
    }
    on_gc7000(|| {
        let sampler = |min: GLenum, mag: GLenum| -> GLuint {
            let mut s: GLuint = 0;
            // SAFETY: plain GL calls.
            unsafe {
                glGenSamplers(1, &mut s);
                glSamplerParameteri(s, GL_TEXTURE_MIN_FILTER, min as GLint);
                glSamplerParameteri(s, GL_TEXTURE_MAG_FILTER, mag as GLint);
            }
            s
        };
        let bind = |unit: GLuint, s: GLuint| {
            // SAFETY: plain GL call.
            unsafe { glBindSampler(unit, s) }
        };
        let p = program(sampler_of(class.1));
        let check = |what: &str, incomplete: bool| {
            verdict(&format!("{} {what}", class.0), incomplete, class, probe(class, p));
        };

        texture(class, GL_NEAREST, GL_NEAREST);
        check("texture only", false);
        let linear = sampler(GL_LINEAR, GL_LINEAR);
        bind(0, linear);
        check("LINEAR sampler on unit 0", true);
        bind(0, 0);
        bind(1, linear);
        check("LINEAR sampler on unit 1", false);
        bind(1, 0);
        check("sampler unbound", false);

        /* the texture itself LINEAR: incomplete, until a NEAREST sampler overrides it */
        texture(class, GL_LINEAR, GL_LINEAR);
        check("LINEAR texture", true);
        bind(0, sampler(GL_NEAREST, GL_NEAREST));
        check("NEAREST sampler over a LINEAR texture", false);
        bind(0, 0);
        check("LINEAR texture again", true);
    });
}

#[test]
fn a_sampler_object_changes_completeness_for_its_unit_only() {
    for class in &WIDE[1..] {
        sampler_case(class);
    }
}

#[test]
fn a_sampler_object_changes_completeness_for_its_unit_only_rgba32f() {
    if !float_filter_visible() {
        return;
    }
    sampler_case(&WIDE[0]);
}

/// OES_texture_float_linear is not exposed, so nothing makes the float class
/// filterable.
#[test]
fn oes_texture_float_linear_is_not_exposed() {
    on_gc7000(|| {
        // SAFETY: glGetString returns a NUL-terminated string of the context.
        let ext = unsafe { core::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS).cast()) };
        let ext = ext.to_string_lossy();
        assert!(!ext.contains("OES_texture_float_linear"), "{ext}");
    });
}
