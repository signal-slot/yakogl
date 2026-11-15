// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): one draw sampling an RGBA32F, an RGBA32UI, an RGBA32I
//! and an RGBA8 texture on four units (item gc-w128-mixed;
//! docs/notes/gc-128bit-texture-sampling.md, gc-128bit-swizzle.md section 6).
//!
//! The program samples `sampler2D` on unit 0 (RGBA32F), `sampler2D` on unit 1
//! (RGBA8), `usampler2D` on unit 2 (RGBA32UI) and `isampler2D` on unit 3
//! (RGBA32I). With the 128-bit classes open (the flip, item gc-w128-flip):
//!
//! - the fragment stage has 4 slots and 3 wide bindings, so the companions
//!   take slots 4, 5 and 6 in unit order (units 0, 2, 3); the RGBA8 unit has
//!   none;
//! - each wide unit's descriptor pair carries Mesa's G32R32 plane word of
//!   its own type (G32R32F for the float unit, G32R32I for both integer
//!   units, which keep their I32/U32 join), plane 1 half-way into the level;
//! - the RGBA8 unit's descriptor is byte-identical to the one the same draw
//!   writes without the wide units (but for the level address).
//!
//! The sampled mask hook that opens the classes (`backend::TEST_EXTRA_SAMPLED`)
//! is `#[cfg(test)]`, so no integration test can add them. Which state
//! applies is read from the draw: a companion descriptor in slot 4 means the
//! planes are reached. Until then the test pins the closed state (no companion
//! slot, no unit stored as a G32R32 plane), says so on stderr, and checks the
//! RGBA8 unit as above; it needs no edit for the flip.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_mixed_gc
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

fn link(fs: &str) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
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

/// Unit 0: RGBA32F, 1: RGBA8, 2: RGBA32UI, 3: RGBA32I, each read at texel (0, 0).
fn mixed_program() -> GLuint {
    link(
        "#version 300 es\n\
         precision highp float;\nprecision highp int;\n\
         uniform sampler2D f;\nuniform sampler2D c;\nuniform highp usampler2D u;\nuniform highp isampler2D i;\n\
         out vec4 o;\n\
         void main() {\n\
         o = texelFetch(f, ivec2(0), 0) + texelFetch(c, ivec2(0), 0)\n\
           + vec4(texelFetch(u, ivec2(0), 0)) + vec4(texelFetch(i, ivec2(0), 0));\n}\n",
    )
}

/// Only the RGBA8 texture, as the one sampler (slot 0: a slot is the sampler's
/// index in the program, `GlesTextureBind::unit`).
fn rgba8_program() -> GLuint {
    link(
        "#version 300 es\nprecision highp float;\nuniform sampler2D c;\nout vec4 o;\n\
         void main() { o = texelFetch(c, ivec2(0), 0); }\n",
    )
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The words of slot `slot`'s descriptor in the last kick, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/// The last value `stream` wrote to `reg`.
fn last(stream: &[(u32, u32)], reg: u32) -> Option<u32> {
    stream.iter().rev().find(|(r, _)| *r == reg).map(|w| w.1)
}

/// One quad with program `p`, sampler uniforms `names[k]` on unit `k`: the
/// last kick's writes.
fn draw(p: GLuint, names: &[(&core::ffi::CStr, i32)]) -> Vec<(u32, u32)> {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        for (name, unit) in names {
            glUniform1i(glGetUniformLocation(p, name.as_ptr()), *unit);
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

const W: u32 = 8;
const H: u32 = 4;

/// The words of every texel of a class: a finite float in [1, 2) for RGBA32F.
fn texel(float: bool, salt: u32) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let h = (salt + 1).wrapping_mul(0x9e37_79b1) ^ (c as u32 + 1).wrapping_mul(0x27d4_eb2f);
        *o = if float { 0x3f80_0000 | (h & 0x007f_ffff) } else { h };
    }
    out
}

/// A one-level 8x4 texture of one value on unit `unit`.
fn upload_wide(unit: u32, class: &(&str, u8, u32, &str), value: [u32; 4]) -> GLenum {
    let (ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGBA),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    let image: Vec<u8> = (0..W * H).flat_map(|_| value.iter().flat_map(|w| w.to_le_bytes())).collect();
    // SAFETY: plain GL calls; the image holds W * H 16-byte texels.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glActiveTexture(GL_TEXTURE0 + unit);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, class.2 as GLint, W as GLsizei, H as GLsizei, 0, fmt, ty, image.as_ptr().cast());
    }
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn upload_rgba8(unit: u32) {
    let image: Vec<u8> = (0..W * H).flat_map(|k| [k as u8, 0x40, 0x80, 0xff]).collect();
    // SAFETY: plain GL calls; the image holds W * H 4-byte texels.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glActiveTexture(GL_TEXTURE0 + unit);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8 as GLint,
            W as GLsizei,
            H as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            image.as_ptr().cast(),
        );
    }
    no_error("RGBA8");
}

/* ---- the draw ----------------------------------------------------------- */

/// Index into WIDE, the unit of the class in the mixed draw, its uniform.
const UNITS: [(usize, u32, &core::ffi::CStr); 3] = [(0, 0, c"f"), (1, 2, c"u"), (2, 3, c"i")];
const RGBA8_UNIT: u32 = 1;
/// The fragment stage's slots: 4 units, then one companion per wide unit.
const FS_UNITS: u32 = 4;

fn value_of(class: usize) -> [u32; 4] {
    texel(class == 0, class as u32 + 1)
}

/// Descriptor words 16.. (everything but the sixteen level addresses).
fn but_levels(d: &[u32]) -> &[u32] {
    &d[16..]
}

/// The draw and the error each wide upload left (the closed state refuses
/// the integer classes, which the GL core cannot store).
fn mixed_draw() -> (Vec<(u32, u32)>, [GLenum; 3]) {
    let mut errors = [GL_NO_ERROR; 3];
    for (k, unit, _) in UNITS {
        errors[k] = upload_wide(unit, &WIDE[k], value_of(k));
    }
    upload_rgba8(RGBA8_UNIT);
    (draw(mixed_program(), &[(c"f", 0), (c"c", RGBA8_UNIT as i32), (c"u", 2), (c"i", 3)]), errors)
}

fn sampler_words(stream: &[(u32, u32)], slot: u32) -> Vec<Option<u32>> {
    [nte_descriptor_samp_ctrl0, nte_descriptor_samp_ctrl1, nte_descriptor_samp_lod_minmax, nte_descriptor_samp_lod_bias]
        .iter()
        .map(|r| last(stream, r(slot)))
        .collect()
}

/// The RGBA8 descriptor and sampler words of slot 0 in a draw with no wide unit.
fn rgba8_alone() -> (Vec<u32>, Vec<Option<u32>>) {
    let mut out = None;
    on_gc7000(|| {
        upload_rgba8(0);
        let s = draw(rgba8_program(), &[(c"c", 0)]);
        out = Some((descriptor(&s, 0).expect("slot 0"), sampler_words(&s, 0)));
    });
    out.unwrap()
}

/// Is the GL core storing the wide classes as planes in this draw? Read from
/// the descriptor of unit 0 (RGBA16F otherwise), not from the mask.
fn planes_reached(stream: &[(u32, u32)]) -> bool {
    let d = descriptor(stream, 0).expect("slot 0");
    d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f == rnndb(WIDE[0].3)
}

#[test]
fn one_draw_of_float_uint_int_and_rgba8_units() {
    let (alone, alone_samp) = rgba8_alone();
    on_gc7000(|| {
        let (s, errors) = mixed_draw();
        let rgba8 = descriptor(&s, RGBA8_UNIT).expect("the RGBA8 unit's descriptor");

        /* the RGBA8 unit: not keyed, identical to the draw without the wide units */
        assert_eq!(
            rgba8[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f,
            alone[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f,
            "RGBA8: the extended format word"
        );
        assert_eq!(but_levels(&rgba8), but_levels(&alone), "RGBA8: descriptor words beside the level addresses");
        assert_eq!(sampler_words(&s, RGBA8_UNIT), alone_samp, "RGBA8: sampler words");

        let reached = planes_reached(&s);
        assert_eq!(
            reached,
            WIDE.iter().all(|c| open(c.1)),
            "the planes are reached exactly when the mask holds the classes"
        );
        if !reached {
            eprintln!(
                "RGBA32F/UI/I: not in GC_SAMPLED_TEXFMTS; the mixed two-plane draw is not exercised (gc-w128-flip): closed state"
            );
            /* the GL core stores RGBA32F as halves and refuses the integer classes */
            assert_eq!(errors[0], GL_NO_ERROR, "closed: RGBA32F is stored (as RGBA16F)");
            assert_eq!(errors[1], GL_INVALID_OPERATION, "closed: RGBA32UI is refused");
            assert_eq!(errors[2], GL_INVALID_OPERATION, "closed: RGBA32I is refused");
            /* no companion descriptor in any slot past the four units */
            for slot in FS_UNITS..FS_UNITS + 4 {
                assert!(descriptor(&s, slot).is_none(), "closed: no companion in slot {slot}");
            }
            /* no unit is stored as a G32R32 plane */
            for unit in 0..FS_UNITS {
                if let Some(d) = descriptor(&s, unit) {
                    let ext = d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f;
                    assert!(
                        ext != rnndb(WIDE[0].3) && ext != rnndb(WIDE[1].3),
                        "closed: unit {unit} is a G32R32 plane"
                    );
                }
            }
            return;
        }

        assert_eq!(errors, [GL_NO_ERROR; 3], "open: the three uploads are stored");
        /* open: three companions after the four units, in unit order */
        let companions = [FS_UNITS, FS_UNITS + 1, FS_UNITS + 2];
        for (rank, (k, unit, name)) in UNITS.iter().enumerate() {
            let what = format!("{} on unit {unit} ({name:?})", WIDE[*k].0);
            let d0 = descriptor(&s, *unit).unwrap_or_else(|| panic!("{what}: no descriptor"));
            let d1 = descriptor(&s, companions[rank])
                .unwrap_or_else(|| panic!("{what}: no companion in slot {}", companions[rank]));
            for (p, d) in [&d0, &d1].into_iter().enumerate() {
                assert_eq!(
                    d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f,
                    rnndb(WIDE[*k].3),
                    "{what}: plane {p}: {}",
                    WIDE[*k].3
                );
                assert_eq!(
                    d[GC_TEXDESC_CONFIG0 as usize / 4],
                    texdesc_type(GC_TEX_TYPE_2D) | texdesc_addressing_mode(GC_TEX_ADDRESSING_TILED),
                    "{what}: plane {p}: type and tiled addressing"
                );
                assert_eq!(d[GC_TEXDESC_LINEAR_STRIDE as usize / 4], align4(W) * 8, "{what}: plane {p}: stride");
            }
            assert_eq!(but_levels(&d0), but_levels(&d1), "{what}: the companion is a copy but for the level addresses");
            let (a0, a1) = (d0[texdesc_lod_addr(0) as usize / 4], d1[texdesc_lod_addr(0) as usize / 4]);
            let plane = align4(W) * align4(H) * 8;
            assert_eq!(a1, a0 + plane, "{what}: plane 1 half-way into the level");
            let want = value_of(*k);
            assert_eq!(words_at(u64::from(a0), 2), want[..2], "{what}: R,G in plane 0");
            assert_eq!(words_at(u64::from(a1), 2), want[2..], "{what}: B,A in plane 1");
            assert_eq!(
                sampler_words(&s, *unit),
                sampler_words(&s, companions[rank]),
                "{what}: sampler words of both slots"
            );
        }
        /* the two integer units share the integer plane word, the float unit has its own */
        assert_ne!(rnndb(WIDE[0].3), rnndb(WIDE[1].3));
        assert_eq!(rnndb(WIDE[1].3), rnndb(WIDE[2].3));
        assert!(descriptor(&s, FS_UNITS + 3).is_none(), "three wide units, three companions");
    });
}
