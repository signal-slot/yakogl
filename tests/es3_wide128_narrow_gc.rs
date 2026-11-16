// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the per-draw plane key of the 32-bit textures that
//! are narrower than RGBA32 (item gc-w128-narrow of plan #366;
//! docs/notes/gc-128bit-swizzle.md sections 3, 5 and 6).
//!
//! - GL_RGB32F, GL_RGB32I and GL_RGB32UI: held in the 128-bit class where
//!   the back end samples it, they are drawn as two planes, keyed like
//!   RGBA32 on the binding with its B,A plane on the companion slot, and the
//!   joined `.w` is 1 (1.0, or integer 1): the key's `no_alpha` makes the
//!   join write the constant, and the core stores alpha 1 in the class
//!   (tests/es3_rgb32_alpha.rs), so `.w` is 1 whichever of the two the tree
//!   has. Which one it is, is said on stderr.
//! - GL_RG32F and GL_R32F: R32F is its own one-channel class and never
//!   keyed; RG32F takes the 128-bit class where the mask has it and RGBA16F
//!   otherwise. Either way blue reads 0 and alpha 1 (ES 3.0.6 3.8.15, table
//!   3.24): stored (r, g, 0, 1) where the store has the lanes, the format's
//!   absent channel and the descriptor's A = ONE where it has not.
//! - R32I/UI and RG32I/UI beside an RGBA32 unit in one draw: their own
//!   hardware classes (docs/notes/gc-r32-int-sampling.md), never keyed, even
//!   where the RGBA32 unit beside them is; and an R32F unit beside an RGB32F
//!   one the same.
//!
//! Every expectation follows from the class the core stored and the sampled
//! mask, read at run time (no class is pinned here: that is
//! tests/es3_wide128_storage_gc.rs). The mask hook of the in-crate tests
//! (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so until
//! GC_SAMPLED_TEXFMTS holds the 128-bit classes (item gc-w128-flip) the
//! tests find them closed, say so on stderr and check the closed state: the
//! integer RGB32 formats are refused by name, the float ones held as
//! RGBA16F, and no draw is keyed. From the flip on they check the open
//! behaviour without edits.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_narrow_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3_formats::half_to_f32;
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
use yakogl::vivante::texture::{gc_texfmt, is_wide128};

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

/// `n` words of the stand-in's memory at GPU address `gpu`.
fn words_at(gpu: u64, n: usize) -> Vec<u32> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back().unwrap_or_else(|| panic!("no allocation at 0x{gpu:x}"));
        let off = (gpu - base) as usize;
        assert!(off + n * 4 <= a.size, "0x{gpu:x}+{} runs off the allocation at 0x{base:x}", n * 4);
        // SAFETY: inside the allocation (checked just above).
        unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off) as *const u32, n) }
            .iter()
            .map(|w| u32::from_le(*w))
            .collect()
    })
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

fn error() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
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

const VS: &str = "#version 300 es\nin vec4 p;\nout vec2 uv;\nvoid main() { uv = p.xy; gl_Position = p; }\n";

/// A program whose fragment stage samples `s0`, `s1`, ... of the sampler
/// types `samplers` with `texture`, all summed into a vec4.
fn program(samplers: &[&str]) -> GLuint {
    let decl: String = samplers.iter().enumerate().map(|(i, s)| format!("uniform highp {s} s{i};\n")).collect();
    let sum = (0..samplers.len()).map(|i| format!("vec4(texture(s{i}, uv))")).collect::<Vec<_>>().join(" + ");
    let fs = format!(
        "#version 300 es\nprecision highp float;\n{decl}in vec2 uv;\nout vec4 o;\nvoid main() {{ o = {sum}; }}\n"
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {fs}");
        p
    }
}

/// One 2x2 texture of `internal`, NEAREST, bound to unit `unit`, every
/// texel the 32-bit words `texel` (as many as `format` has components); its
/// name and the error of its glTexImage2D.
fn texture(unit: u32, internal: GLenum, format: GLenum, type_: GLenum, texel: &[u32]) -> (GLuint, GLenum) {
    let texels: Vec<u32> = (0..4).flat_map(|_| texel.iter().copied()).collect();
    // SAFETY: plain GL calls; `texels` holds 2x2 texels of `texel.len()` words.
    unsafe {
        let mut t: GLuint = 0;
        glActiveTexture(GL_TEXTURE0 + unit);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 2, 2, 0, format, type_, texels.as_ptr() as *const c_void);
        (t, error())
    }
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// One quad with program `p`, sampler `s<i>` on unit `i`, drawn (not
/// refused) and finished: the plane sensitivity and key the draw worked
/// out, and the last kick's writes.
fn draw(p: GLuint, units: usize) -> ([u32; 2], [Wide128Planes; 2], Vec<(u32, u32)>) {
    let refused = draws_refused();
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        for i in 0..units {
            let name = CString::new(format!("s{i}")).unwrap();
            glUniform1i(glGetUniformLocation(p, name.as_ptr()), i as GLint);
        }
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    assert_eq!(error(), GL_NO_ERROR, "draw");
    assert_eq!(draws_refused(), refused, "the draw is not refused");
    let (sense, key) = wide128_draw_state(p).expect("the draw asked the program's sensitivity");
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(error(), GL_NO_ERROR, "glFinish");
    let stream = with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the quad produced no kick");
        p.kicks.last().cloned().unwrap()
    });
    (sense, key, writes(&stream))
}

/// The words of slot `slot`'s descriptor in the stream, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/// CONFIG1 of slot `slot`'s descriptor: (FORMAT_EXT, [R, G, B, A] swizzle).
fn config1(stream: &[(u32, u32)], slot: u32, what: &str) -> (u32, [u32; 4]) {
    let d = descriptor(stream, slot).unwrap_or_else(|| panic!("{what}: no descriptor in slot {slot}"));
    let c1 = d[GC_TEXDESC_CONFIG1 as usize / 4];
    (c1 & 0x3f, [(c1 >> 8) & 7, (c1 >> 12) & 7, (c1 >> 16) & 7, (c1 >> 20) & 7])
}

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    sampled_texfmts() & gles_texfmt_bit(format) != 0
}

/// The class the core stored texture `name` in, and texel (0, 0) of level 0
/// as 32-bit words (a half of RGBA16F widened to its float's bits).
fn stored(name: GLuint) -> (u8, Vec<u32>) {
    let c = yakogl::gl::context::current().expect("context");
    let t = &c.objs().textures[name as usize];
    assert!(!t.mem.cpu.is_null(), "texture {name} has storage with a CPU view");
    let bpp = t.bpp as usize;
    // SAFETY: texel (0, 0) of level 0 lies inside the storage the core laid out.
    let b = unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(t.level_off[0] as usize), bpp) };
    let words = if t.texfmt == GLES_TEXFMT_RGBA16F {
        b.chunks(2).map(|h| half_to_f32(u16::from_ne_bytes([h[0], h[1]])).to_bits()).collect()
    } else {
        b.chunks(4).map(|w| u32::from_ne_bytes([w[0], w[1], w[2], w[3]])).collect()
    };
    (t.texfmt, words)
}

/// Does the GL core store `name` as a class the draw samples as two planes?
fn planes(texfmt: u8) -> bool {
    is_wide128(texfmt) && open(texfmt)
}

const ONE_F: u32 = 0x3f80_0000;

/* ---- RGB32F / RGB32I / RGB32UI ------------------------------------------ */

struct Rgb32 {
    name: &'static str,
    internal: GLenum,
    format: GLenum,
    type_: GLenum,
    sampler: &'static str,
    /// The class the format takes where the mask has it.
    class: u8,
    /// The texel uploaded (R, G, B) and the alpha a missing one reads.
    rgb: [u32; 3],
    one: u32,
}

const RGB32: [Rgb32; 3] = [
    Rgb32 {
        name: "GL_RGB32F",
        internal: GL_RGB32F,
        format: GL_RGB,
        type_: GL_FLOAT,
        sampler: "sampler2D",
        class: GLES_TEXFMT_RGBA32F,
        /* 0.5, 2.0, 4.0: exact as halves too */
        rgb: [0x3f00_0000, 0x4000_0000, 0x4080_0000],
        one: ONE_F,
    },
    Rgb32 {
        name: "GL_RGB32I",
        internal: GL_RGB32I,
        format: GL_RGB_INTEGER,
        type_: GL_INT,
        sampler: "isampler2D",
        class: GLES_TEXFMT_RGBA32I,
        rgb: [0x8000_0007, 0x7fff_fff0, 0xffff_fffd],
        one: 1,
    },
    Rgb32 {
        name: "GL_RGB32UI",
        internal: GL_RGB32UI,
        format: GL_RGB_INTEGER,
        type_: GL_UNSIGNED_INT,
        sampler: "usampler2D",
        class: GLES_TEXFMT_RGBA32UI,
        rgb: [0xdead_beef, 0x0123_4567, 0xffff_ffff],
        one: 1,
    },
];

/// An RGB32F/I/UI texture on its sampler: where the core holds it in the
/// 128-bit class the draw is keyed on its binding alone, B,A on slot 1 (the
/// stage's one slot, then its companion), both descriptors read A = ONE, and
/// `.w` is 1: the key's `no_alpha` (the join's constant) or the stored alpha
/// 1 the join reads. Closed: the integer formats are refused by name (no
/// class holds them), RGB32F is held as RGBA16F with alpha 1.0, and no draw
/// is keyed.
#[test]
fn rgb32_draws_are_keyed_as_two_planes_and_read_alpha_one() {
    for f in &RGB32 {
        on_gc7000(|| {
            let p = program(&[f.sampler]);
            let (name, err) = texture(0, f.internal, f.format, f.type_, &f.rgb);
            if err != GL_NO_ERROR {
                assert!(!open(f.class), "{}: refused though its class is sampled: 0x{err:x}", f.name);
                assert_eq!(err, GL_INVALID_OPERATION, "{}: refused by name", f.name);
                eprintln!("{}: no class holds it (wide128-flip): refused by name, the draw not keyed", f.name);
                let (sense, key, _) = draw(p, 1);
                assert_eq!(sense, [0, 1], "{}", f.name);
                assert_eq!(key, [Wide128Planes::NONE; 2], "{}: nothing bound is two planes", f.name);
                return;
            }
            let (class, texel) = stored(name);
            assert_eq!(texel[..3], f.rgb, "{}: the colour is stored as given", f.name);
            assert_eq!(texel.get(3), Some(&f.one), "{}: a missing alpha is stored as 1", f.name);
            let (sense, key, s) = draw(p, 1);
            assert_eq!(sense, [0, 1], "{}: the fragment stage samples binding 0", f.name);
            assert_eq!(config1(&s, 0, f.name).1[3], GC_SWIZZLE_1, "{}: no alpha: A = ONE", f.name);
            assert_eq!(key[0], Wide128Planes::NONE, "{}: the vertex stage samples nothing", f.name);
            if !planes(class) {
                eprintln!("{}: held in class {class}, not two planes (wide128-flip): the draw is not keyed", f.name);
                assert!(!is_wide128(class) || !open(class), "{}", f.name);
                assert_eq!(key[1], Wide128Planes::NONE, "{}: one plane, no key", f.name);
                return;
            }
            assert_eq!(key[1].mask, 1, "{}: binding 0 alone is keyed", f.name);
            assert_eq!(key[1].companion_of(0), Some(1), "{}: B,A on slot 1", f.name);
            assert_eq!(config1(&s, 1, f.name).1[3], GC_SWIZZLE_1, "{}: the companion is a copy", f.name);
            if key[1].lacks_alpha(0) {
                eprintln!("{}: the key carries no_alpha: the join writes 0x{:x} to w", f.name, f.one);
            } else {
                eprintln!(
                    "{}: the key does not carry no_alpha (gc-w128-key-plumb): w is plane 1's stored alpha 0x{:x}",
                    f.name, f.one
                );
            }
        });
    }
}

/* ---- RG32F and R32F ----------------------------------------------------- */

/// GL_RG32F and GL_R32F read blue 0 and alpha 1 (ES 3.0.6 table 3.24). R32F
/// is its own one-channel class (EXT_R32F), never keyed: green and blue are
/// the format's absent channels, alpha the descriptor's ONE. RG32F where it
/// is two planes: keyed on its binding, stored (r, g, 0, 1.0), so plane 1's
/// B,A are 0 and 1 whatever the key's no_alpha; held as RGBA16F (closed):
/// not keyed, the halves (r, g, 0, 1.0), A = ONE.
#[test]
fn rg32f_and_r32f_read_blue_zero_and_alpha_one() {
    let (r, g) = (0x3f00_0000u32, 0x4000_0000u32);
    on_gc7000(|| {
        let p = program(&["sampler2D"]);
        let (name, err) = texture(0, GL_R32F, GL_RED, GL_FLOAT, &[r]);
        assert_eq!(err, GL_NO_ERROR, "GL_R32F");
        let (class, texel) = stored(name);
        assert_eq!(class, GLES_TEXFMT_R32F, "GL_R32F is its own class");
        assert!(!is_wide128(class));
        assert_eq!(texel, [r], "GL_R32F: one word a texel");
        let (_, key, s) = draw(p, 1);
        assert_eq!(key, [Wide128Planes::NONE; 2], "GL_R32F: one plane, no key");
        let (ext, sw) = config1(&s, 0, "GL_R32F");
        assert_eq!(ext, GC_TEX_FORMAT_EXT_R32F, "GL_R32F: EXT_R32F");
        assert_eq!(
            sw,
            [GC_SWIZZLE_R, GC_SWIZZLE_G, GC_SWIZZLE_B, GC_SWIZZLE_1],
            "GL_R32F: G, B the absent channels, A = ONE"
        );
    });
    on_gc7000(|| {
        let p = program(&["sampler2D"]);
        let (name, err) = texture(0, GL_RG32F, GL_RG, GL_FLOAT, &[r, g]);
        assert_eq!(err, GL_NO_ERROR, "GL_RG32F");
        let (class, texel) = stored(name);
        assert_eq!(texel.len(), 4, "GL_RG32F: held in a four-channel class ({class})");
        assert_eq!(texel, [r, g, 0, ONE_F], "GL_RG32F: stored (r, g, 0, 1.0)");
        let (_, key, s) = draw(p, 1);
        assert_eq!(config1(&s, 0, "GL_RG32F").1[2..], [GC_SWIZZLE_B, GC_SWIZZLE_1], "GL_RG32F: B stored, A = ONE");
        if planes(class) {
            assert_eq!(key[1].mask, 1, "GL_RG32F: two planes, binding 0 keyed");
            assert_eq!(key[1].companion_of(0), Some(1), "GL_RG32F: B,A on slot 1");
        } else {
            eprintln!("GL_RG32F: held in class {class}, not two planes (wide128-flip): the draw is not keyed");
            assert_eq!(key, [Wide128Planes::NONE; 2], "GL_RG32F: one plane, no key");
        }
    });
}

/* ---- narrow 32-bit integers beside an RGBA32 unit ----------------------- */

/// R32UI, RG32UI, R32I and RG32I on unit 0 beside an RGBA32 texture of the
/// same sign on unit 1, both sampled in one draw: unit 0 is its own hardware
/// class (EXT_R32I, or EXT_G32R32I as one 8-byte texel) and is never keyed;
/// unit 1 is keyed with B,A on slot 2 (the stage's two slots, then the
/// companion) where the RGBA32 class is two planes, and keeps its alpha.
/// Closed: the RGBA32 texture is refused by name and nothing is keyed.
#[test]
fn narrow_32_bit_integers_beside_an_rgba32_unit_are_not_keyed() {
    let cases = [
        ("GL_R32UI", GL_R32UI, GL_RED_INTEGER, GL_UNSIGNED_INT, "usampler2D", GL_RGBA32UI),
        ("GL_RG32UI", GL_RG32UI, GL_RG_INTEGER, GL_UNSIGNED_INT, "usampler2D", GL_RGBA32UI),
        ("GL_R32I", GL_R32I, GL_RED_INTEGER, GL_INT, "isampler2D", GL_RGBA32I),
        ("GL_RG32I", GL_RG32I, GL_RG_INTEGER, GL_INT, "isampler2D", GL_RGBA32I),
    ];
    for (what, internal, format, type_, sampler, wide) in cases {
        on_gc7000(|| {
            let p = program(&[sampler, sampler]);
            let comps = if format == GL_RED_INTEGER { 1 } else { 2 };
            let narrow = [0x8000_0011u32, 0x7fff_ff22][..comps].to_vec();
            let (n, err) = texture(0, internal, format, type_, &narrow);
            assert_eq!(err, GL_NO_ERROR, "{what} is sampled");
            let (class, texel) = stored(n);
            assert!(!is_wide128(class), "{what}: its own class ({class}), not a 128-bit one");
            assert_eq!(texel, narrow, "{what}: {} word(s) a texel", comps);
            let (w, err) = texture(1, wide, GL_RGBA_INTEGER, type_, &[1, 2, 3, 4]);
            let (sense, key, s) = draw(p, 2);
            assert_eq!(sense, [0, 0b11], "{what}: the fragment stage samples bindings 0 and 1");
            assert_eq!(key[0], Wide128Planes::NONE, "{what}: the vertex stage samples nothing");
            assert_eq!(key[1].companion_of(0), None, "{what}: unit 0 is never keyed");
            assert!(!key[1].lacks_alpha(0), "{what}");
            assert_eq!(config1(&s, 0, what).0, gc_texfmt(class).format_ext, "{what}: unit 0's own word");
            if err != GL_NO_ERROR {
                assert_eq!(err, GL_INVALID_OPERATION, "{what}: the RGBA32 texture is refused by name");
                eprintln!("{what}: the RGBA32 unit beside it has no class (wide128-flip): nothing is keyed");
                assert_eq!(key[1], Wide128Planes::NONE, "{what}");
                return;
            }
            let (wclass, _) = stored(w);
            if !planes(wclass) {
                eprintln!("{what}: the RGBA32 unit is held in class {wclass}, not two planes: nothing is keyed");
                assert_eq!(key[1], Wide128Planes::NONE, "{what}");
                return;
            }
            assert_eq!(key[1].mask, 0b10, "{what}: unit 1 alone is keyed");
            assert_eq!(key[1].companion_of(1), Some(2), "{what}: unit 1's B,A on slot 2");
            assert!(!key[1].lacks_alpha(1), "{what}: RGBA32 has alpha");
        });
    }
}

/// An R32F unit beside an RGB32F one in one draw: the R32F unit is never
/// keyed; the RGB32F one is, B,A on slot 2, where the core holds it as two
/// planes, and nothing is keyed where it does not (closed: RGBA16F).
#[test]
fn r32f_beside_rgb32f_keys_only_the_rgb32f_unit() {
    on_gc7000(|| {
        let p = program(&["sampler2D", "sampler2D"]);
        let (_, err) = texture(0, GL_R32F, GL_RED, GL_FLOAT, &[0x3f00_0000]);
        assert_eq!(err, GL_NO_ERROR, "GL_R32F");
        let (t, err) = texture(1, GL_RGB32F, GL_RGB, GL_FLOAT, &[0x3f00_0000, 0x4000_0000, 0x4080_0000]);
        assert_eq!(err, GL_NO_ERROR, "GL_RGB32F");
        let (class, _) = stored(t);
        let (sense, key, s) = draw(p, 2);
        assert_eq!(sense, [0, 0b11]);
        assert_eq!(key[0], Wide128Planes::NONE);
        assert_eq!(key[1].companion_of(0), None, "R32F is never keyed");
        assert_eq!(config1(&s, 0, "GL_R32F").0, GC_TEX_FORMAT_EXT_R32F, "unit 0 keeps EXT_R32F");
        assert_eq!(config1(&s, 1, "GL_RGB32F").1[3], GC_SWIZZLE_1, "GL_RGB32F: A = ONE");
        if planes(class) {
            assert_eq!(key[1].mask, 0b10, "unit 1 alone is keyed");
            assert_eq!(key[1].companion_of(1), Some(2), "unit 1's B,A on slot 2");
            assert_eq!(config1(&s, 2, "the companion").1[3], GC_SWIZZLE_1, "the companion is a copy");
        } else {
            eprintln!("GL_RGB32F: held in class {class}, not two planes (wide128-flip): nothing is keyed");
            assert_eq!(key[1], Wide128Planes::NONE);
        }
    });
}
