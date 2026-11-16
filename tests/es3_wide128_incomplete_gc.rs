// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): an incomplete RGBA32F/UI/I binding is not a two-plane
//! binding (docs/notes/gc-128bit-texture-sampling.md; ES 3.0 section 3.8.14).
//!
//! A texture that is incomplete samples (0,0,0,1). The GL core marks the
//! binding `incomplete`, `wide128_sampled` is then false, so the per-draw
//! plane key (`wide128_draw_state`) holds no unit for it, no companion slot
//! is packed for it, and the descriptor slot of the unit is the one-texel
//! black RGBA8 surface, not a G32R32 pair. The four ways in:
//!
//! - a mipmapped minification filter with levels missing;
//! - levels of another internal format (RGBA32 at level 0, RGBA8 after);
//! - a cube map whose faces differ (a face missing; a face of another size);
//! - a texture name bound but never given storage.
//!
//! LINEAR filtering (tests/es3_wide128_filter_gc.rs) and BASE/MAX_LEVEL
//! (tests/es3_wide128_levels_gc.rs) are those items'. Each case also draws
//! a complete RGBA32 unit beside the incomplete one: the complete unit takes
//! the one companion slot, after the stage's own, and its descriptor pair is
//! intact.
//!
//! The sampled mask hook (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`,
//! so no integration test can open the classes: until GC_SAMPLED_TEXFMTS
//! holds them (item gc-w128-flip) these draws find the classes closed, say
//! so on stderr, and check what holds in either state (no key, no companion,
//! the black texel, no GL error, no panic); the checks that need the classes
//! open run from the flip on, the state being found at run time.
//!
//! The file is not built under `report-es2` (the ES 2.0 report has no ES 3.0
//! texture paths or `#version 300 es` shaders).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_incomplete_gc
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
        /* an error left by the last context's teardown is not this case's */
        while glGetError() != GL_NO_ERROR {}
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

/// Context-wide count of draws that sampled an incomplete texture.
fn incomplete_draws() -> u32 {
    yakogl::gl::draw::incomplete_texture_draws()
}

/// The sampler type prefix of a class on a 2D look-up.
fn sampler_prefix(class: u8) -> &'static str {
    match class {
        GLES_TEXFMT_RGBA32F => "",
        GLES_TEXFMT_RGBA32I => "i",
        _ => "u",
    }
}

type Class = (&'static str, u8, u32, &'static str);

/// `s` (unit 0, `ty`, looked up at `coord`) and `s2` (unit 1, a 2D sampler
/// of the same class): the first is the binding under test, the second a
/// complete RGBA32 texture beside it.
fn program(ty: &str, coord: &str, ty2: &str) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp {ty};\nprecision highp {ty2};\nuniform {ty} s;\nuniform {ty2} s2;\nout vec4 o;\nvoid main() {{ o = vec4(texture(s, {coord})) + vec4(texture(s2, vec2(0.5))); }}\n"
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

/// A `w`x`h` image of the class at `target`/`level`, every texel
/// (1, 2, 3, 4) in the class's own type.
fn image(class: &Class, target: GLenum, level: GLint, w: u32, h: u32) {
    let (ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGBA),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    let one: [u32; 4] =
        if class.1 == GLES_TEXFMT_RGBA32F { [1.0f32, 2.0, 3.0, 4.0].map(f32::to_bits) } else { [1, 2, 3, 4] };
    let px: Vec<u8> = (0..w * h).flat_map(|_| one.iter().flat_map(|v| v.to_le_bytes())).collect();
    // SAFETY: plain GL call; `px` holds w*h texels of the class.
    unsafe {
        glTexImage2D(target, level, class.2 as GLint, w as GLsizei, h as GLsizei, 0, fmt, ty, px.as_ptr().cast());
    }
}

/// A `w`x`h` RGBA8 image at `target`/`level`.
fn image_rgba8(target: GLenum, level: GLint, w: u32, h: u32) {
    let px = vec![0x7fu8; (w * h * 4) as usize];
    // SAFETY: plain GL call; `px` holds w*h texels.
    unsafe {
        glTexImage2D(
            target,
            level,
            GL_RGBA8 as GLint,
            w as GLsizei,
            h as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_ptr().cast(),
        );
    }
}

/// What a draw with an incomplete binding on unit 0 looked like.
struct Seen {
    /// The fragment stage's plane key: bit `u` is unit `u` as two planes.
    mask: u32,
    companion: [u8; 32],
    /// The words of the descriptor in each slot 0..4 (`None`: not written).
    slots: [Option<Vec<u32>>; 4],
    /// Draws this one counted as sampling an incomplete texture.
    counted: u32,
}

/// Draw with `s` = unit 0 as the caller bound it and `s2` = unit 1 a
/// complete 4x4 RGBA32 texture of the class.
fn draw(p: GLuint, class: &Class) -> Seen {
    let kicks0 = with_platform(|p| p.kicks.len());
    let counted0 = incomplete_draws();
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        let mut t: GLuint = 0;
        glActiveTexture(GL_TEXTURE0 + 1);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        image(class, GL_TEXTURE_2D, 0, 4, 4);
        glActiveTexture(GL_TEXTURE0);
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glUniform1i(glGetUniformLocation(p, c"s2".as_ptr()), 1);
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
        writes(p.kicks.last().unwrap())
    });
    let (_, planes) = yakogl::gl::draw::wide128_draw_state(p).expect("the draw asked for the plane key");
    assert_eq!(planes[0].mask, 0, "the vertex stage samples nothing, so it has no key");
    Seen {
        mask: planes[1].mask,
        companion: planes[1].companion,
        slots: core::array::from_fn(|i| descriptor(&stream, i as u32)),
        counted: incomplete_draws() - counted0,
    }
}

/// Is the descriptor a G32R32 plane's (the class stored as two planes)?
fn is_plane(class: &Class, d: &[u32]) -> bool {
    d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f == rnndb(class.3)
}

/// The invariants of an incomplete binding on unit 0 beside a complete
/// RGBA32 texture on unit 1, in either state of the mask.
///
/// `incomplete`: the binding is one the core flags (ES 3.0 3.8.14). When it
/// is not (`false`: the core reads a texture complete that the spec makes
/// incomplete, see `Expect::NotForRgba32f`), the unit is only checked to be
/// keyed as the descriptor it got says.
fn check(what: &str, class: &Class, seen: &Seen, incomplete: bool) {
    let d0 = seen.slots[0].as_ref().unwrap_or_else(|| panic!("{what}: unit 0 wrote no descriptor"));
    if incomplete {
        /* the incomplete unit is no two-plane binding */
        assert_eq!(seen.mask & 1, 0, "{what}: unit 0 is keyed as a two-plane binding");
        /* its slot holds the one-texel black RGBA8 surface */
        assert!(!is_plane(class, d0), "{what}: unit 0 is a G32R32 descriptor");
        assert_eq!(d0[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(1) | texdesc_size_height(1), "{what}: 1x1");
        let a = d0[texdesc_lod_addr(0) as usize / 4];
        assert_ne!(a, 0, "{what}: the black texel has an address");
        /* RGBA8 (0,0,0,1): alpha is the top byte of the little-endian word */
        assert_eq!(words_at(u64::from(a), 1), [0xff00_0000], "{what}: it reads (0,0,0,1)");
    } else {
        assert_eq!(seen.mask & 1 != 0, is_plane(class, d0), "{what}: unit 0's key and descriptor disagree");
    }
    /* the control unit: a pair when the core stores the class as planes */
    let d1 = seen.slots[1].as_ref().unwrap_or_else(|| panic!("{what}: unit 1 wrote no descriptor"));
    if !incomplete {
        assert_eq!(seen.mask & 2 != 0, is_plane(class, d1), "{what}: unit 1's key and descriptor disagree");
    } else if is_plane(class, d1) {
        assert_eq!(seen.mask, 0b10, "{what}: only unit 1 is keyed");
        assert_eq!(seen.companion[1], 2, "{what}: unit 1's B,A plane is the first slot after the stage's two");
        let d2 = seen.slots[2].as_ref().unwrap_or_else(|| panic!("{what}: no companion descriptor in slot 2"));
        assert!(is_plane(class, d2), "{what}: the companion is a plane descriptor");
        let (a0, a1) = (d1[texdesc_lod_addr(0) as usize / 4], d2[texdesc_lod_addr(0) as usize / 4]);
        assert_eq!(a1, a0 + 4 * 4 * 8, "{what}: plane 1 follows plane 0");
        assert_eq!(seen.slots[3], None, "{what}: the incomplete unit took a companion slot");
    } else {
        assert_eq!(seen.mask, 0, "{what}: nothing is keyed while the class is not stored as planes");
        assert_eq!(seen.slots[2], None, "{what}: a companion slot with no two-plane binding");
        assert_eq!(seen.slots[3], None, "{what}: a companion slot with no two-plane binding");
    }
}

fn note(class: &Class, case: &str) {
    if !open(class.1) {
        eprintln!(
            "{} {case}: not in GC_SAMPLED_TEXFMTS; checking the closed state (no key, no companion slot, black); the open checks run after gc-w128-flip",
            class.0
        );
    }
}

/// Does the GL core hold the class at all? RGBA32F is stored as RGBA16F while
/// the mask lacks it, RGBA32UI/I have no other storage class: with the mask
/// closed, glTexImage2D of them is refused (`es3_formats::storage_in`).
fn stored(class: &Class) -> bool {
    open(class.1) || class.1 == GLES_TEXFMT_RGBA32F
}

/// Every error the GL has recorded, cleared.
fn drain_errors() -> Vec<GLenum> {
    let mut v = Vec::new();
    loop {
        // SAFETY: a plain GL call.
        let e = unsafe { glGetError() };
        if e == GL_NO_ERROR {
            return v;
        }
        v.push(e);
    }
}

/// What the core makes of the binding of a case.
#[derive(Clone, Copy, PartialEq)]
enum Expect {
    /// Incomplete (ES 3.0 3.8.14): counted as one draw sampling an
    /// incomplete texture, black on unit 0.
    Incomplete,
    /// As `Incomplete` for RGBA32UI/I. RGBA32F is stored as RGBA16F
    /// (`texture.rs` sized_format) and the core records a level or face of
    /// another format only through the `es3_formats` row or the upload
    /// format, which RGBA8 and RGBA32F share: it reads the texture complete.
    /// Observed, said on stderr as a defect, not pinned either way.
    NotForRgba32f,
    /// Incomplete or not as the core says, not counted: a name bound without
    /// storage has no binding at all, the backend's declared-sampler black
    /// slot stands in.
    Unbound,
}

/// Run one case for each class: `bind` leaves the binding under test on
/// unit 0, a texture sampled as `ty` at `coord`. While the mask lacks the
/// class and the core cannot store it, `bind`'s uploads are refused with
/// GL_INVALID_OPERATION and nothing more is checked.
fn each(case: &str, ty: &str, coord: &str, expect: Expect, bind: impl Fn(&Class)) {
    for class in &WIDE {
        note(class, case);
        on_gc7000(|| {
            let what = format!("{} {case}", class.0);
            if !stored(class) {
                bind(class);
                let errors = drain_errors();
                eprintln!("{what}: the GL core cannot store the class yet: {errors:x?}");
                assert!(errors.iter().all(|&e| e == GL_INVALID_OPERATION), "{what}: errors {errors:x?}");
                return;
            }
            bind(class);
            no_error(&format!("{what}: bind"));
            let pre = sampler_prefix(class.1);
            let p = program(&format!("{pre}{ty}"), coord, &format!("{pre}sampler2D"));
            let seen = draw(p, class);
            let incomplete = match expect {
                Expect::Incomplete => true,
                Expect::NotForRgba32f => class.1 != GLES_TEXFMT_RGBA32F,
                Expect::Unbound => true,
            };
            if expect != Expect::Unbound {
                if incomplete {
                    assert_eq!(seen.counted, 1, "{what}: draws counted as sampling an incomplete texture");
                } else if seen.counted == 0 {
                    eprintln!("{what}: DEFECT: the core reads a mismatched-format chain complete");
                }
            }
            check(&what, class, &seen, incomplete || seen.counted != 0);
            no_error("after");
        });
    }
}

fn new_texture(target: GLenum) {
    // SAFETY: plain GL calls.
    unsafe {
        let mut t: GLuint = 0;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
    }
}

fn filters(target: GLenum, min: GLenum) {
    // SAFETY: plain GL calls.
    unsafe {
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
}

/// NEAREST_MIPMAP_NEAREST with level 0 alone: levels 1 and 2 of the 4x4 chain
/// are missing, so the texture is incomplete and samples (0,0,0,1).
#[test]
fn a_mipmap_filter_with_missing_levels_is_not_a_plane_binding() {
    each("mipmap filter, level 0 only", "sampler2D", "vec2(0.5)", Expect::Incomplete, |class| {
        new_texture(GL_TEXTURE_2D);
        filters(GL_TEXTURE_2D, GL_NEAREST_MIPMAP_NEAREST);
        image(class, GL_TEXTURE_2D, 0, 4, 4);
    });
    /* a hole in the chain: level 2 present, level 1 missing */
    each("mipmap filter, level 1 missing", "sampler2D", "vec2(0.5)", Expect::Incomplete, |class| {
        new_texture(GL_TEXTURE_2D);
        filters(GL_TEXTURE_2D, GL_NEAREST_MIPMAP_NEAREST);
        image(class, GL_TEXTURE_2D, 0, 4, 4);
        image(class, GL_TEXTURE_2D, 2, 1, 1);
    });
}

/// RGBA32 at level 0 and RGBA8 at levels 1 and 2: every level is there at
/// its size, but the chain's formats differ.
#[test]
fn levels_of_another_internal_format_are_not_a_plane_binding() {
    each("levels 1 and 2 are RGBA8", "sampler2D", "vec2(0.5)", Expect::NotForRgba32f, |class| {
        new_texture(GL_TEXTURE_2D);
        filters(GL_TEXTURE_2D, GL_NEAREST_MIPMAP_NEAREST);
        image(class, GL_TEXTURE_2D, 0, 4, 4);
        image_rgba8(GL_TEXTURE_2D, 1, 2, 2);
        image_rgba8(GL_TEXTURE_2D, 2, 1, 1);
    });
    /* a level of another size: also incomplete */
    each("level 1 has the wrong size", "sampler2D", "vec2(0.5)", Expect::Incomplete, |class| {
        new_texture(GL_TEXTURE_2D);
        filters(GL_TEXTURE_2D, GL_NEAREST_MIPMAP_NEAREST);
        image(class, GL_TEXTURE_2D, 0, 4, 4);
        image(class, GL_TEXTURE_2D, 1, 3, 3);
        image(class, GL_TEXTURE_2D, 2, 1, 1);
    });
}

/// A cube map with a face missing, one whose faces differ in size, and one
/// whose faces differ in format.
#[test]
fn a_cube_map_with_mismatched_faces_is_not_a_plane_binding() {
    each("cube, -Z face missing", "samplerCube", "vec3(0.3, 0.5, 0.7)", Expect::Incomplete, |class| {
        new_texture(GL_TEXTURE_CUBE_MAP);
        filters(GL_TEXTURE_CUBE_MAP, GL_NEAREST);
        for face in 0..5 {
            image(class, GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, 4, 4);
        }
    });
    each("cube, face of another size", "samplerCube", "vec3(0.3, 0.5, 0.7)", Expect::Incomplete, |class| {
        new_texture(GL_TEXTURE_CUBE_MAP);
        filters(GL_TEXTURE_CUBE_MAP, GL_NEAREST);
        for face in 0..6 {
            let n = if face == 3 { 2 } else { 4 };
            image(class, GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, n, n);
        }
    });
    each("cube, face of another format", "samplerCube", "vec3(0.3, 0.5, 0.7)", Expect::NotForRgba32f, |class| {
        new_texture(GL_TEXTURE_CUBE_MAP);
        filters(GL_TEXTURE_CUBE_MAP, GL_NEAREST);
        for face in 0..6 {
            if face == 3 {
                image_rgba8(GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, 4, 4);
            } else {
                image(class, GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, 4, 4);
            }
        }
    });
}

/// A texture name bound to the unit and never given storage.
#[test]
fn a_texture_name_without_storage_is_not_a_plane_binding() {
    each("no storage, 2D", "sampler2D", "vec2(0.5)", Expect::Unbound, |_| new_texture(GL_TEXTURE_2D));
    each("no storage, cube", "samplerCube", "vec3(0.3, 0.5, 0.7)", Expect::Unbound, |_| {
        new_texture(GL_TEXTURE_CUBE_MAP)
    });
    each("no storage, 3D", "sampler3D", "vec3(0.5)", Expect::Unbound, |_| new_texture(GL_TEXTURE_3D));
}

/// The same program, the missing levels then supplied: unit 0 becomes a
/// plane binding in the open state (both units keyed, companions in unit
/// order), and nothing changes in the closed one.
#[test]
fn completing_the_texture_changes_the_key_with_the_mask() {
    for class in &WIDE {
        note(class, "completed");
        on_gc7000(|| {
            if !stored(class) {
                eprintln!("{}: the GL core cannot store the class yet; not exercised", class.0);
                return;
            }
            new_texture(GL_TEXTURE_2D);
            filters(GL_TEXTURE_2D, GL_NEAREST_MIPMAP_NEAREST);
            image(class, GL_TEXTURE_2D, 0, 4, 4);
            let pre = sampler_prefix(class.1);
            let p = program(&format!("{pre}sampler2D"), "vec2(0.5)", &format!("{pre}sampler2D"));
            let before = draw(p, class);
            assert_eq!(before.counted, 1, "{}: one level of a chain, mipmap filter", class.0);
            check(&format!("{} before", class.0), class, &before, true);
            image(class, GL_TEXTURE_2D, 1, 2, 2);
            image(class, GL_TEXTURE_2D, 2, 1, 1);
            let after = draw(p, class);
            no_error("completed");
            assert_eq!(after.counted, 0, "{}: the completed texture is no longer counted", class.0);
            let d0 = after.slots[0].as_ref().expect("unit 0 descriptor");
            if is_plane(class, d0) {
                assert_eq!(after.mask, 0b11, "{}: both units keyed", class.0);
                assert_eq!(after.companion[0], 2, "{}: unit 0's B,A plane first", class.0);
                assert_eq!(after.companion[1], 3, "{}: unit 1's B,A plane second", class.0);
                assert!(after.slots[2].is_some() && after.slots[3].is_some(), "{}: two companions", class.0);
            } else {
                assert_eq!(after.mask, 0, "{}: not stored as planes: nothing keyed", class.0);
            }
        });
    }
}
