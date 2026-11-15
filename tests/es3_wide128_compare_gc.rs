// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): `TEXTURE_COMPARE_MODE = COMPARE_REF_TO_TEXTURE` on an
//! RGBA32F, RGBA32UI or RGBA32I texture, set on the texture and on a sampler
//! object (docs/notes/gc-128bit-texture-sampling.md,
//! docs/notes/gc-shadow-compare.md, docs/notes/gc-depth-sample.md).
//!
//! ES 3.0 (section 3.8.14, "Texture Comparison Modes") applies the compare
//! state only to a depth-format texture read through a shadow sampler; on a
//! texture of any other base format it is ignored. A shadow sampler on a
//! non-depth texture returns an undefined value and raises no error. What
//! the tests pin on this back end:
//!
//! - (a) `sampler2D` on RGBA32F with the compare state set on the texture or
//!   on a bound sampler object: both plane descriptors are byte-identical to
//!   the ones of a draw without it.
//! - (b) `sampler2DShadow` on an RGBA32F texture: no panic, GL_NO_ERROR; today the
//!   shader is refused at compile time by name (no shadow look-up is selected), and
//!   should that change the draw is either drawn (a kick) or dropped; which of the
//!   two is printed on stderr.
//! - (c) `usampler2D` / `isampler2D` on RGBA32UI / RGBA32I as (a).
//!
//! The sampled mask hook that opens the 128-bit classes
//! (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so no integration test
//! can add them: until GC_SAMPLED_TEXFMTS holds them (item gc-w128-flip) the
//! GL core stores the class as RGBA16F; these find the classes closed, say so
//! on stderr and pin the closed state (the descriptor is not a G32R32 plane
//! and the compare state still changes none of the texture's words). From the
//! flip on the same code checks the planes, unedited.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_compare_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
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

const WIDE: [(&str, u8, u32, &str); 3] = [
    ("RGBA32F", GLES_TEXFMT_RGBA32F, GL_RGBA32F, "TEXTURE_FORMAT_EXT_G32R32F"),
    ("RGBA32UI", GLES_TEXFMT_RGBA32UI, GL_RGBA32UI, "TEXTURE_FORMAT_EXT_G32R32I"),
    ("RGBA32I", GLES_TEXFMT_RGBA32I, GL_RGBA32I, "TEXTURE_FORMAT_EXT_G32R32I"),
];

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
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

/// The compiled shader, or the info log of the compiler's refusal.
fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 512];
            glGetShaderInfoLog(s, 512, core::ptr::null_mut(), log.as_mut_ptr());
            let log = core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned();
            return Err(log);
        }
        Ok(s)
    }
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The words of slot `slot`'s descriptor in the last kick, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/* ---- one draw ------------------------------------------------------------ */

/// Where the compare state is set.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Compare {
    /// nothing: the default GL_NONE
    Off,
    /// GL_TEXTURE_COMPARE_MODE on the texture object
    Texture,
    /// GL_TEXTURE_COMPARE_MODE on a sampler object bound to unit 0
    Sampler,
}

/// What one draw left: the GL error of the upload and of the draw, the writes
/// of the kick it produced, if it produced one, and the words of the
/// descriptors of slots 0 and 1 (read while the memory was live).
struct Drawn {
    /// the info log, if the shader did not compile or the program did not link
    refused: Option<String>,
    upload: GLenum,
    error: GLenum,
    stream: Option<Vec<(u32, u32)>>,
    desc: [Option<Vec<u32>>; 2],
}

fn program(ty: &str, coord: &str) -> Result<GLuint, String> {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {ty} s;\nout vec4 o;\nvoid main() {{ o = vec4(texture(s, {coord})); }}\n"
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs).expect("the vertex shader compiles"));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs)?);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        Ok(p)
    }
}

/// A 4x4 RGBA32 texture of the class on unit 0 (nearest, one level, texel
/// words unique per component), the compare state set as `compare` says, and
/// one quad drawn with `ty` and `coord`.
fn draw_once(class: &(&str, u8, u32, &str), ty: &str, coord: &str, compare: Compare) -> Drawn {
    let (gl_ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGBA),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    let texel = |i: u32| -> [u8; 16] {
        let mut b = [0u8; 16];
        for c in 0..4u32 {
            let w =
                if class.1 == GLES_TEXFMT_RGBA32F { 0x3f80_0000 | (i * 4 + c + 1) } else { 0x1000_0000 + i * 4 + c };
            b[c as usize * 4..c as usize * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        b
    };
    let image: Vec<u8> = (0..16).flat_map(texel).collect();
    let mut out = None;
    on_gc7000(|| {
        let p = match program(ty, coord) {
            Ok(p) => p,
            Err(log) => {
                out = Some(Drawn {
                    refused: Some(log),
                    upload: GL_NO_ERROR,
                    error: GL_NO_ERROR,
                    stream: None,
                    desc: [None, None],
                });
                return;
            }
        };
        let kicks0 = with_platform(|p| p.kicks.len());
        // SAFETY: plain GL calls; the image is alive across them.
        let upload = unsafe {
            let mut t: GLuint = 0;
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(GL_TEXTURE_2D, 0, class.2 as GLint, 4, 4, 0, fmt, gl_ty, image.as_ptr().cast());
            glGetError()
        };
        if upload != GL_NO_ERROR {
            out = Some(Drawn { refused: None, upload, error: GL_NO_ERROR, stream: None, desc: [None, None] });
            return;
        }
        // SAFETY: plain GL calls; QUAD is a static.
        let error = unsafe {
            match compare {
                Compare::Off => {}
                Compare::Texture => {
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_FUNC, GL_LEQUAL as GLint);
                }
                Compare::Sampler => {
                    let mut s: GLuint = 0;
                    glGenSamplers(1, &mut s);
                    glSamplerParameteri(s, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                    glSamplerParameteri(s, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
                    glSamplerParameteri(s, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
                    glSamplerParameteri(s, GL_TEXTURE_COMPARE_FUNC, GL_LEQUAL as GLint);
                    glBindSampler(0, s);
                }
            }
            no_error("compare state");
            glViewport(0, 0, 64, 64);
            glUseProgram(p);
            glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            let e = glGetError();
            glFinish();
            e
        };
        let stream = with_platform(|p| (p.kicks.len() > kicks0).then(|| p.kicks.last().cloned().unwrap()));
        let stream = stream.map(|s| writes(&s));
        let desc = [0, 1].map(|slot| stream.as_ref().and_then(|s| descriptor(s, slot)));
        out = Some(Drawn { refused: None, upload, error, stream, desc });
    });
    out.expect("the case ran")
}

/// A descriptor's words but for the sixteen level addresses (each run
/// allocates its own memory, so the addresses differ).
fn shared_words(d: &Option<Vec<u32>>) -> Option<&[u32]> {
    d.as_ref().map(|d| &d[16..])
}

/// The sampler registers of slots 0 and 1, if the kick wrote them.
fn sampler_words(stream: &[(u32, u32)]) -> Vec<Option<u32>> {
    let regs: [fn(u32) -> u32; 4] = [
        nte_descriptor_samp_ctrl0,
        nte_descriptor_samp_ctrl1,
        nte_descriptor_samp_lod_minmax,
        nte_descriptor_samp_lod_bias,
    ];
    let mut v = Vec::new();
    for slot in 0..2 {
        for r in regs {
            v.push(stream.iter().rev().find(|(a, _)| *a == r(slot)).map(|(_, x)| *x));
        }
    }
    v
}

/// The class's draw with a non-shadow sampler: the compare state set on the
/// texture or on a sampler object leaves the descriptors alone.
fn compare_is_ignored(class: &(&str, u8, u32, &str), ty: &str) {
    let what = format!("{} through {ty}", class.0);
    let base = draw_once(class, ty, "vec2(0.5, 0.5)", Compare::Off);
    assert_eq!(base.refused, None, "{what}: the shader compiles");
    if !open(class.1) && base.upload != GL_NO_ERROR {
        /* closed: the GL core does not store the class, so there is no texture to
        carry the compare state; the refusal is the same with it set */
        eprintln!(
            "{what}: not in GC_SAMPLED_TEXFMTS; the upload is refused (0x{:x}) and the planes are not exercised (gc-w128-flip)",
            base.upload
        );
        for mode in [Compare::Texture, Compare::Sampler] {
            assert_eq!(draw_once(class, ty, "vec2(0.5, 0.5)", mode).upload, base.upload, "{what}: {mode:?}");
        }
        return;
    }
    assert_eq!(base.upload, GL_NO_ERROR, "{what}: the upload");
    assert_eq!(base.error, GL_NO_ERROR, "{what}: the plain draw");
    let base_stream = base.stream.as_ref().expect("the plain draw produced a kick");
    let planes = base.desc[0].as_ref().is_some_and(|d| d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f == rnndb(class.3));
    if open(class.1) {
        assert!(planes, "{what}: the open class is a G32R32 plane in slot 0");
        assert!(base.desc[1].is_some(), "{what}: and has a companion in slot 1");
    } else {
        eprintln!("{what}: not in GC_SAMPLED_TEXFMTS; the planes are not exercised (gc-w128-flip)");
        assert!(!planes, "{what}: the closed class is not stored as a G32R32 plane");
    }
    for mode in [Compare::Texture, Compare::Sampler] {
        let got = draw_once(class, ty, "vec2(0.5, 0.5)", mode);
        assert_eq!(got.error, GL_NO_ERROR, "{what}: compare on {mode:?}");
        let stream = got.stream.as_ref().unwrap_or_else(|| panic!("{what}: compare on {mode:?}: no kick"));
        for slot in 0..2 {
            assert_eq!(
                shared_words(&got.desc[slot]),
                shared_words(&base.desc[slot]),
                "{what}: compare on {mode:?}: slot {slot}'s descriptor words are those of the draw without it"
            );
        }
        assert_eq!(
            sampler_words(stream),
            sampler_words(base_stream),
            "{what}: compare on {mode:?}: the sampler words (both slots) are unchanged"
        );
    }
}

/// (a) RGBA32F through `sampler2D`.
#[test]
fn compare_state_on_an_rgba32f_texture_is_ignored_by_a_plain_sampler() {
    compare_is_ignored(&WIDE[0], "sampler2D");
}

/// (c) RGBA32UI through `usampler2D`, RGBA32I through `isampler2D`.
#[test]
fn compare_state_on_the_integer_classes_is_ignored_by_integer_samplers() {
    compare_is_ignored(&WIDE[1], "usampler2D");
    compare_is_ignored(&WIDE[2], "isampler2D");
}

/// (b) `sampler2DShadow` on an RGBA32F texture (undefined result per ES 3.0
/// section 3.8.14, no error): no panic and GL_NO_ERROR whether the texture
/// carries the compare state, a sampler object does, or neither does. Today
/// the code generator refuses every shadow look-up by name before any texture
/// is involved ("a shadow sampler ... is not selected yet"): the shader does
/// not compile and the info log says so. Should a later change select shadow
/// look-ups, the draw is drawn or dropped, the same each time, and the test
/// checks that instead.
#[test]
fn a_shadow_sampler_on_an_rgba32f_texture_raises_no_error() {
    let class = &WIDE[0];
    let mut seen = Vec::new();
    for mode in [Compare::Off, Compare::Texture, Compare::Sampler] {
        let a = draw_once(class, "sampler2DShadow", "vec3(0.5, 0.5, 0.5)", mode);
        let b = draw_once(class, "sampler2DShadow", "vec3(0.5, 0.5, 0.5)", mode);
        assert_eq!(a.refused.is_some(), b.refused.is_some(), "compare on {mode:?}: refused the same each time");
        if let Some(log) = &a.refused {
            assert!(log.contains("shadow sampler"), "compare on {mode:?}: the refusal names the shadow sampler: {log}");
            seen.push("the shader is refused by name");
            continue;
        }
        for d in [&a, &b] {
            assert_eq!(d.upload, GL_NO_ERROR, "compare on {mode:?}: the upload");
            assert_eq!(d.error, GL_NO_ERROR, "RGBA32F through sampler2DShadow, compare on {mode:?}");
        }
        assert_eq!(a.stream.is_some(), b.stream.is_some(), "compare on {mode:?}: drawn or dropped, the same each time");
        seen.push(if a.stream.is_some() { "drawn" } else { "dropped" });
    }
    eprintln!("RGBA32F through sampler2DShadow (compare Off, Texture, Sampler): {}", seen.join(", "));
}
