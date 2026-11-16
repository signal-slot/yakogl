// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the other units of a draw are untouched by an RGBA32
//! binding (item gc-w128-neighbours).
//!
//! An RGBA32F/UI/I binding adds a companion sampler slot after its stage's
//! own slots (`companion_slot`, src/vivante/texture.rs; `wide128_companions`
//! in the GL core's draw path). No GL unit lives there, so there is no
//! renumbering: a sampler's own slot is its binding in the program (the
//! fragment stage's base is 0; `GlesTextureBind::unit` is that binding)
//! whatever the other units bind, and only the vertex stage's base
//! (`GC_VS_SAMPLER_BASE`, `fs_count + fs_wide`) moves. This file draws once
//! with six units of the classes the GC samples today and once more with an
//! RGBA32 texture added on another unit, and pins that
//!
//! - the descriptor words of units 0..5 (each the whole descriptor, level
//!   addresses included) and the slots they land in are byte-identical
//!   between the two draws, and between two draws of the same scene (the
//!   control that tells a drift from the added unit apart from one of the
//!   stand-in's);
//! - the RGBA32 unit and its companion take slots no other unit has, and
//!   their level addresses alias no other unit's;
//! - the vertex sampler base moves by the one companion and nothing else does.
//!
//! The six: RGBA8, RGBA16F, R32F, a depth texture with
//! TEXTURE_COMPARE_MODE (a `sampler2DShadow`), a compressed sRGB ETC2 texture
//! (the linear ETC2 classes are decoded at upload, not in the mask) and
//! RG32UI (a `usampler2D`); each is in `GC_SAMPLED_TEXFMTS`
//! (src/backend.rs; docs/es3/gc-formats.md). tests/es3_wide128_slots_gc.rs
//! has the slot arithmetic and its collision matrix; this file is the
//! draw's descriptors.
//!
//! The classes of the added unit are not in `GC_SAMPLED_TEXFMTS` yet (item
//! gc-w128-flip) and the mask hook of the in-crate tests
//! (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so no integration
//! test can open them. As tests/es3_wide128_planes_gc.rs: while they are
//! closed the added unit is whatever the GL core stores for the class (no
//! companion slot is written, said on stderr) and the same pins hold for
//! it; from the flip on the companion checks run for real. Before the flip
//! they were run with GLES_TEXFMT_RGBA32F, GLES_TEXFMT_RGBA32UI and
//! GLES_TEXFMT_RGBA32I added to GC_SAMPLED_TEXFMTS locally (reverted).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_neighbours_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

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
use yakogl::vivante::texture::companion_slot;

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

/// The RGBA32 classes: name, class, internal format, sampler type, upload
/// (format, type) and the rnndb name of the G32R32 word of its planes.
const WIDE: [(&str, u8, u32, &str, u32, u32, &str); 3] = [
    ("RGBA32F", GLES_TEXFMT_RGBA32F, GL_RGBA32F, "sampler2D", GL_RGBA, GL_FLOAT, "TEXTURE_FORMAT_EXT_G32R32F"),
    (
        "RGBA32UI",
        GLES_TEXFMT_RGBA32UI,
        GL_RGBA32UI,
        "usampler2D",
        GL_RGBA_INTEGER,
        GL_UNSIGNED_INT,
        "TEXTURE_FORMAT_EXT_G32R32I",
    ),
    ("RGBA32I", GLES_TEXFMT_RGBA32I, GL_RGBA32I, "isampler2D", GL_RGBA_INTEGER, GL_INT, "TEXTURE_FORMAT_EXT_G32R32I"),
];

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

/* ---- the stand-in OSAL (as tests/es3_wide128_planes_gc.rs) -------------- */

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
        if ok != 1 {
            let mut log = [0 as c_char; 512];
            glGetShaderInfoLog(s, 512, core::ptr::null_mut(), log.as_mut_ptr());
            let log = core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned();
            panic!("{text:?} did not compile: {log}");
        }
        s
    }
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The words of slot `slot`'s descriptor in the last kick, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/* ---- the scene ---------------------------------------------------------- */

/// What a unit binds.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Rgba8,
    Rgba16F,
    R32F,
    /// DEPTH_COMPONENT24 with TEXTURE_COMPARE_MODE: a `sampler2DShadow`.
    Shadow,
    /// COMPRESSED_SRGB8_ETC2.
    Etc2,
    Rg32Ui,
    /// A declared sampler with no texture on its unit (the black texel).
    Unbound,
    /// An index into [`WIDE`].
    Wide(usize),
}

const SIX: [Kind; 6] = [Kind::Rgba8, Kind::Rgba16F, Kind::R32F, Kind::Shadow, Kind::Etc2, Kind::Rg32Ui];

impl Kind {
    /// Every unit is read through a `sampler2D`: the online compiler does not
    /// select a shadow sampler or an integer result yet ("an ES 3.00 texture
    /// look-up (a shadow sampler, an integer result, ...) is not selected
    /// yet"), so a program with one is refused at compile time. The binding
    /// does not depend on it: the descriptor of a unit comes from the
    /// texture bound to it, whatever the program's sampler type (a mismatch
    /// reads undefined texels, which the descriptor pins do not look at).
    fn sampler(self) -> &'static str {
        "sampler2D"
    }

    /// An expression of type vec4 sampling the sampler named `n`.
    fn fetch(self, n: &str) -> String {
        format!("texelFetch({n}, ivec2(1), 0)")
    }
}

/// Make a 4x4 texture of `kind` on `unit`, its texels told apart by `unit`.
fn make_texture(kind: Kind, unit: u32) {
    let e = try_texture(kind, unit);
    assert_eq!(e, GL_NO_ERROR, "{kind:?} on unit {unit}: 0x{e:x}");
}

/// [`make_texture`], returning the GL error instead of asserting there is none.
fn try_texture(kind: Kind, unit: u32) -> GLenum {
    let salt = (unit * 17 + 3) as u8;
    let bytes = |n: usize| -> Vec<u8> { (0..n).map(|i| salt.wrapping_add((i as u8).wrapping_mul(7))).collect() };
    let floats = |n: usize| -> Vec<f32> { (0..n).map(|i| 1.0 + unit as f32 + i as f32 / 64.0).collect() };
    if kind == Kind::Unbound {
        return GL_NO_ERROR;
    }
    // SAFETY: plain GL calls; each image holds 4x4 texels of its class.
    unsafe {
        glActiveTexture(GL_TEXTURE0 + unit);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        let img = |ifmt: u32, fmt: u32, ty: u32, p: *const c_void| {
            glTexImage2D(GL_TEXTURE_2D, 0, ifmt as GLint, 4, 4, 0, fmt, ty, p);
        };
        match kind {
            Kind::Rgba8 => img(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, bytes(64).as_ptr().cast()),
            Kind::Rgba16F => img(GL_RGBA16F, GL_RGBA, GL_FLOAT, floats(64).as_ptr().cast()),
            Kind::R32F => img(GL_R32F, GL_RED, GL_FLOAT, floats(16).as_ptr().cast()),
            Kind::Shadow => {
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
                let d: Vec<u32> = (0..16).map(|i| 0x1000_0000u32 * (i % 8) + unit).collect();
                img(GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, d.as_ptr().cast());
            }
            Kind::Etc2 => {
                let blk = bytes(8);
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_SRGB8_ETC2, 4, 4, 0, 8, blk.as_ptr().cast());
            }
            Kind::Rg32Ui => {
                let d: Vec<u32> = (0..32).map(|i| 0x0101_0101 * unit + i).collect();
                img(GL_RG32UI, GL_RG_INTEGER, GL_UNSIGNED_INT, d.as_ptr().cast());
            }
            Kind::Unbound => {}
            Kind::Wide(i) => {
                let d: Vec<u32> =
                    (0..64)
                        .map(|k| {
                            if i == 0 {
                                (1.0 + unit as f32 + k as f32 / 64.0).to_bits()
                            } else {
                                0x0101_0101 * unit + k
                            }
                        })
                        .collect();
                img(WIDE[i].2, WIDE[i].4, WIDE[i].5, d.as_ptr().cast());
            }
        }
    }
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// A program sampling every `(unit, kind)`, one uniform `s<unit>` each.
fn program(units: &[(u32, Kind)]) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (u, k) in units {
        fs += &format!("uniform {} s{u};\n", k.sampler());
    }
    fs += "out vec4 o;\nvoid main() {\n  vec4 r = vec4(0.0);\n";
    for (u, k) in units {
        fs += &format!("  r += {};\n", k.fetch(&format!("s{u}")));
    }
    fs += "  o = r;\n}\n";
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

/// One quad with `p` sampling the units: the last kick's writes.
fn draw(p: GLuint, units: &[(u32, Kind)]) -> Vec<(u32, u32)> {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        for (u, _) in units {
            let name = CString::new(format!("s{u}")).unwrap();
            glUniform1i(glGetUniformLocation(p, name.as_ptr()), *u as GLint);
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

/// The 32 slots of the draw: the descriptor each wrote, if it wrote one.
fn slots(stream: &[(u32, u32)]) -> Vec<Option<Vec<u32>>> {
    (0..32).map(|s| descriptor(stream, s)).collect()
}

fn lod0(d: &[u32]) -> u32 {
    d[texdesc_lod_addr(0) as usize / 4]
}

fn vs_base(stream: &[(u32, u32)]) -> Option<u32> {
    stream.iter().rev().find(|(r, _)| *r == GC_VS_SAMPLER_BASE).map(|w| w.1)
}

/* ---- the pins ----------------------------------------------------------- */

/// The six units bind textures of their own: each of the six slots is
/// written, with a distinct level-0 address (none is the shared black
/// texel), and the stand-in's draws are repeatable (the control).
#[test]
fn the_six_units_are_six_descriptors_and_a_draw_repeats() {
    on_gc7000(|| {
        let six: Vec<(u32, Kind)> = SIX.iter().enumerate().map(|(u, k)| (u as u32, *k)).collect();
        for (u, k) in &six {
            make_texture(*k, *u);
        }
        let p = program(&six);
        let a = slots(&draw(p, &six));
        let b = slots(&draw(p, &six));
        assert_eq!(a, b, "two draws of one scene");
        let mut addrs = Vec::new();
        for (u, k) in &six {
            let d = a[*u as usize].as_ref().unwrap_or_else(|| panic!("{k:?}: no descriptor in slot {u}"));
            addrs.push(lod0(d));
        }
        let mut sorted = addrs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 6, "six textures, six level-0 addresses: {addrs:x?}");
        assert!(a[6..].iter().all(Option::is_none), "no descriptor past the six units");
    });
}

/// Draw the six, then the six and an RGBA32 texture on `unit`. Slots 0..5 are
/// byte-identical; the added unit and its companion are slots of their own
/// and alias no other unit's level addresses.
fn neighbours_case(w: usize, unit: u32) {
    let class = &WIDE[w];
    on_gc7000(|| {
        /* the six, then `unit - 6` declared samplers with no texture bound
         * (sampled black, a descriptor each) up to the added sampler */
        let mut six: Vec<(u32, Kind)> = SIX.iter().enumerate().map(|(u, k)| (u as u32, *k)).collect();
        six.extend((6..unit).map(|u| (u, Kind::Unbound)));
        for (u, k) in &six {
            make_texture(*k, *u);
        }
        let pa = program(&six);
        let sa = draw(pa, &six);
        let before = slots(&sa);

        let e = try_texture(Kind::Wide(w), unit);
        if e != GL_NO_ERROR {
            /* closed integer class: the GL core stores no 16-byte integer class
             * for this back end, so the upload is refused by name and nothing
             * is bound; the six draw as before */
            assert!(!open(class.1) && w != 0, "{}: 0x{e:x} with the class open", class.0);
            assert_eq!(e, GL_INVALID_OPERATION, "{}: the closed class is refused as INVALID_OPERATION", class.0);
            eprintln!("{}: not in GC_SAMPLED_TEXFMTS; the upload is refused, nothing to add (wide128-flip)", class.0);
            assert_eq!(slots(&draw(pa, &six)), before, "{}: the six after a refused upload", class.0);
            return;
        }
        let mut seven = six.clone();
        seven.push((unit, Kind::Wide(w)));
        let sb = draw(program(&seven), &seven);
        let after = slots(&sb);

        let what = |s: &str| format!("{} on unit {unit}: {s}", class.0);
        for s in 0..unit as usize {
            assert!(before[s].is_some(), "{}", what(&format!("slot {s} written before")));
            assert_eq!(after[s], before[s], "{}", what(&format!("slot {s}'s descriptor moved")));
        }
        /* the vertex stage's base: after the fragment units, and the companion if any */
        assert_eq!(vs_base(&sa), Some(unit), "{}", what("base without it"));
        let own = after[unit as usize].as_ref().unwrap_or_else(|| panic!("{}", what("no descriptor in its slot")));
        /* Open is not enough: the GL core stores RGBA32F as halves whatever the
         * mask says (texture.rs `sized_format`, the planes test's
         * `stored_as_planes`), so the descriptor says whether it is planes. */
        let wide = open(class.1) && own[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f == rnndb(class.6);
        if open(class.1) && !wide {
            eprintln!("{}: in the mask but not stored as planes (texture.rs sized_format); treated as closed", class.0);
        }
        let companion = companion_slot(0, unit + 1, 0).and_then(|s| after[s as usize].as_ref());
        if wide {
            assert_eq!(vs_base(&sb), Some(unit + 2), "{}", what("one companion after the units"));
            assert_eq!(
                own[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f,
                rnndb(class.6),
                "{}",
                what("plane 0 is the G32R32 word")
            );
            let c = companion.unwrap_or_else(|| panic!("{}", what("no companion descriptor")));
            assert_eq!(c[GC_TEXDESC_CONFIG1 as usize / 4], own[GC_TEXDESC_CONFIG1 as usize / 4]);
            assert_ne!(lod0(c), lod0(own), "{}", what("the planes are two addresses"));
            /* 4x4 texels, 8 bytes each: plane 1 half-way into the level */
            assert_eq!(lod0(c), lod0(own) + 4 * 4 * 8, "{}", what("plane 1 half-way into the level"));
        } else {
            eprintln!("{}: no planes; the companion is not exercised (wide128-flip)", class.0);
            assert_eq!(vs_base(&sb), Some(unit + 1), "{}", what("no companion, no shift"));
            assert!(companion.is_none(), "{}", what("a companion slot while the class is closed"));
        }
        /* nothing but the added unit (and its companion) is new */
        let new = (0..32usize).filter(|&s| after[s].is_some() && before[s].is_none()).count();
        assert_eq!(new, if wide { 2 } else { 1 }, "{}", what("descriptors added"));
        /* no level address of the added unit is another unit's */
        let mut others: Vec<u32> = (0..6usize).map(|s| lod0(before[s].as_ref().unwrap())).collect();
        let mine = [Some(lod0(own)), companion.map(|c| lod0(c))];
        for a in mine.into_iter().flatten() {
            assert!(!others.contains(&a), "{}", what(&format!("0x{a:x} aliases a neighbour: {others:x?}")));
            others.push(a);
        }
    });
}

#[test]
fn rgba32f_beside_the_six_leaves_them_alone() {
    neighbours_case(0, 6);
}

#[test]
fn rgba32ui_beside_the_six_leaves_them_alone() {
    neighbours_case(1, 6);
}

#[test]
fn rgba32i_beside_the_six_leaves_them_alone() {
    neighbours_case(2, 6);
}

/// The sampler past a gap. A unit's slot is its sampler's binding in the
/// program (`GlesTextureBind::unit` is the shader's binding, draw.rs
/// `tb.unit = binding`), not the GL texture unit number; here samplers 6 to 8
/// declare nothing bound (the black texel, a descriptor each), so the stage
/// has ten slots and the companion is slot 10; the six are still untouched.
#[test]
fn an_rgba32_unit_past_a_gap_leaves_the_six_alone() {
    for w in 0..3 {
        neighbours_case(w, 9);
    }
}
