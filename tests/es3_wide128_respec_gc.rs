// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the plane key and the shader a draw submits follow
//! the GL state from draw to draw (docs/notes/gc-128bit-texture-sampling.md).
//!
//! A draw that samples an RGBA32F/UI/I texture takes a plane-join shader
//! variant chosen per draw (`wide128_select`, src/gl/draw.rs): the program's
//! sensitivity is cached after the first draw and reset by a link, and the
//! variant is found per plane key. Nothing is remembered about the textures
//! themselves, so whatever the application does between draws has to show in
//! the next draw's key and in the fragment shader's address
//! (`GC_PS_INST_ADDR` of the kick):
//!
//! - (a) one texture re-specified RGBA8 -> RGBA32F -> RGBA8 -> RGBA32I (a new
//!   size each time: the descriptor is the new one too);
//! - (b) `glUniform1i` moving a sampler from an RGBA8 unit to an RGBA32 unit
//!   and back, with the RGBA32 texture bound all the while;
//! - (c) one texture used by two programs, each with its own cache;
//! - (d) a relink (the cache is reset), and `glDeleteTextures` of a bound
//!   RGBA32 texture and a new texture after it;
//! - (e) a draw with no RGBA32 binding after one with goes back to the plain
//!   shader, the very one the program had before.
//!
//! The sampled mask hook that opens the 128-bit classes
//! (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so no integration test
//! can add them: until GC_SAMPLED_TEXFMTS holds them (item gc-w128-flip)
//! these find the classes closed, say so on stderr and check the closed
//! state (the key stays empty, the plain shader is submitted on every draw);
//! from the flip on they check the key and the shader for real. Before the
//! flip they were run with the three classes added to GC_SAMPLED_TEXFMTS
//! locally.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_respec_gc
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

/* ---- what a draw submits ------------------------------------------------ */

const F8: u8 = GLES_TEXFMT_RGBA8;
const F32F: u8 = GLES_TEXFMT_RGBA32F;
const F32UI: u8 = GLES_TEXFMT_RGBA32UI;
const F32I: u8 = GLES_TEXFMT_RGBA32I;

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

fn closed_note(what: &str, class: &str, format: u8) {
    if !open(format) {
        eprintln!("{what}: {class} is not in GC_SAMPLED_TEXFMTS; the closed state is checked (gc-w128-flip)");
    }
}

fn error() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// What one draw left behind: the program's sensitivity and plane key, the
/// fragment shader's address, and the descriptor of every slot it wrote.
struct Obs {
    sense: [u32; 2],
    key: [yakogl::shaderc::Wide128Planes; 2],
    ps: u32,
    stream: Vec<(u32, u32)>,
}

impl Obs {
    /// The words of slot `slot`'s descriptor, if the draw wrote one.
    fn desc(&self, slot: u32) -> Option<Vec<u32>> {
        descriptor(&self.stream, slot)
    }

    /// The fragment key names exactly the units in `units`.
    fn fs_units(&self) -> u32 {
        self.key[1].mask
    }
}

/// One quad of program `p` sampling `unit` (its sampler `s` set to it).
fn draw_unit(p: GLuint, unit: u32) -> Obs {
    let kicks0 = with_platform(|p| p.kicks.len());
    let refused = draws_refused();
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), unit as GLint);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    assert_eq!(error(), GL_NO_ERROR, "draw");
    assert_eq!(draws_refused(), refused, "the draw is not refused");
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(error(), GL_NO_ERROR, "glFinish");
    let stream = with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the quad produced no kick");
        p.kicks.last().cloned().unwrap()
    });
    let stream = writes(&stream);
    let (sense, key) = wide128_draw_state(p).expect("the draw asked the program's sensitivity");
    let ps = last(&stream, GC_PS_INST_ADDR);
    Obs { sense, key, ps, stream }
}

fn draw0(p: GLuint) -> Obs {
    draw_unit(p, 0)
}

/// The last value `stream` wrote to `reg`.
fn last(stream: &[(u32, u32)], reg: u32) -> u32 {
    stream.iter().rev().find(|(r, _)| *r == reg).unwrap_or_else(|| panic!("no write of register 0x{reg:x}")).1
}

type Key = [yakogl::shaderc::Wide128Planes; 2];
const NONE: Key = [yakogl::shaderc::Wide128Planes::NONE; 2];

/// What a draw of a program sampling `unit` must have: with `class` (the
/// texture on that unit is RGBA32, stored as planes) the fragment key
/// holds exactly that unit, with its companion on another slot, and the
/// vertex key is empty; otherwise no key at all. Returns whether it was wide.
fn expect(what: &str, o: &Obs, unit: u32, class: Option<u8>) -> bool {
    let wide = class.is_some_and(|c| stored_wide(what, o, c));
    assert_eq!(o.sense[0], 0, "{what}: the vertex stage samples nothing");
    assert_eq!(o.sense[1], 1, "{what}: the fragment stage samples binding 0 (the unit is the uniform's value)");
    if !wide {
        assert_eq!(o.key, NONE, "{what}: no 128-bit binding, no key");
        return false;
    }
    assert!(o.key[0].is_empty(), "{what}: the vertex key is empty");
    assert_eq!(o.fs_units(), 1 << unit, "{what}: the fragment key names unit {unit}");
    let c = o.key[1].companion[unit as usize] as u32;
    assert_ne!(c, unit, "{what}: the companion is a slot of its own");
    let (d0, d1) = (
        o.desc(0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0")),
        o.desc(c).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot {c}")),
    );
    assert_ne!(d0[texdesc_lod_addr(0) as usize / 4], d1[texdesc_lod_addr(0) as usize / 4], "{what}: two planes");
    true
}

/// Does the draw's slot 0 hold plane 0 of `class` (EXT_G32R32F / EXT_G32R32I
/// of the vendored common_3d.xml.h)? Only when the class is in the sampled
/// mask *and* the GL core stores it as 16 bytes a texel; the storage is
/// item gc-w128-flip's, so a mask that is open and a texture stored as
/// RGBA16F is said on stderr and checked as the plain draw it is.
fn stored_wide(what: &str, o: &Obs, class: u8) -> bool {
    if !open(class) {
        return false;
    }
    let want = if class == F32F { 0x0b } else { 0x24 };
    let have = o.desc(0).map(|d| d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f);
    if have != Some(want) {
        eprintln!(
            "{what}: the mask is open but the GL core does not store the class as planes (format {have:x?}); plain"
        );
        return false;
    }
    true
}

/* ---- GL plumbing of the item -------------------------------------------- */

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// A program sampling `s` of type `ty` as `body` says (`texture(s, ...)`),
/// a second program on the same sampler when `body` differs.
fn program(ty: &str, body: &str) -> GLuint {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp {ty};\nuniform {ty} s;\nout vec4 o;\nvoid main() {{ o = {body}; }}\n"
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
        assert_eq!(ok, 1, "link failed");
        p
    }
}

fn float_program() -> GLuint {
    program("sampler2D", "texture(s, vec2(0.5))")
}

fn int_program() -> GLuint {
    program("isampler2D", "vec4(texture(s, vec2(0.5)))")
}

/// The format triple of a class: internal format, format, type.
fn triple(format: u8) -> (GLenum, GLenum, GLenum) {
    match format {
        F8 => (GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE),
        F32F => (GL_RGBA32F, GL_RGBA, GL_FLOAT),
        F32I => (GL_RGBA32I, GL_RGBA_INTEGER, GL_INT),
        F32UI => (GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT),
        _ => unreachable!(),
    }
}

/// Texture `t` on unit `unit`, level 0 specified as `format`, `n`x`n`.
/// A class the back end does not sample is refused by the GL core for the
/// integer ones (RGBA32F is stored as RGBA16F): then the error is asserted,
/// the texture is as it was, and the result is false.
fn specify(t: GLuint, unit: u32, format: u8, n: u32) -> bool {
    let (internal, fmt, ty) = triple(format);
    let texel = if format == F8 { 4 } else { 16 };
    let image = vec![0x3fu8; (n * n) as usize * texel];
    // SAFETY: plain GL calls; the image holds n*n texels of the class.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glActiveTexture(GL_TEXTURE0 + unit);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            internal as GLint,
            n as GLsizei,
            n as GLsizei,
            0,
            fmt,
            ty,
            image.as_ptr().cast(),
        );
    }
    let e = error();
    if (format == F32I || format == F32UI) && !open(format) {
        assert_ne!(e, GL_NO_ERROR, "an integer class nothing stores is refused");
        return false;
    }
    assert_eq!(e, GL_NO_ERROR, "specify: 0x{e:x}");
    true
}

fn gen_texture() -> GLuint {
    let mut t: GLuint = 0;
    // SAFETY: a plain GL call.
    unsafe { glGenTextures(1, &mut t) };
    t
}

/// Descriptor slot `slot` of `o` (the sampler binding, not the unit) has this size.
fn size_is(what: &str, o: &Obs, slot: u32, n: u32) {
    let d = o.desc(slot).unwrap_or_else(|| panic!("{what}: no descriptor in slot {slot}"));
    assert_eq!(d[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(n) | texdesc_size_height(n), "{what}: {n}x{n}");
}

/* ---- the tests ---------------------------------------------------------- */

/// (a) One texture object, re-specified between draws as RGBA8, RGBA32F,
/// RGBA8 and RGBA32I (a new size each time): the key is empty, the
/// RGBA32F's, empty again and the RGBA32I's, and the shader is the plain one
/// whenever the key is empty. The two programs (`sampler2D`, `isampler2D`)
/// each have the plain shader of their own.
#[test]
fn one_texture_respecified_between_draws() {
    closed_note("respecified", "RGBA32F/I", F32F);
    on_gc7000(|| {
        let (pf, pi) = (float_program(), int_program());
        let t = gen_texture();
        specify(t, 0, F8, 2);
        let plain_f = draw0(pf);
        expect("RGBA8 (f)", &plain_f, 0, None);
        size_is("RGBA8", &plain_f, 0, 2);

        specify(t, 0, F32F, 4);
        let f = draw0(pf);
        let wf = expect("RGBA32F", &f, 0, Some(F32F));
        size_is("RGBA32F", &f, 0, 4);
        if wf {
            assert_ne!(f.ps, plain_f.ps, "RGBA32F: the plane-join shader");
        } else {
            assert_eq!(f.ps, plain_f.ps, "RGBA32F closed: the plain shader");
        }

        specify(t, 0, F8, 8);
        let again = draw0(pf);
        expect("RGBA8 again", &again, 0, None);
        size_is("RGBA8 again", &again, 0, 8);
        assert_eq!(again.ps, plain_f.ps, "back to the plain shader, the very one");

        if specify(t, 0, F32I, 4) {
            let wide_i = draw0(pi);
            expect("RGBA32I", &wide_i, 0, Some(F32I));
            size_is("RGBA32I", &wide_i, 0, 4);
            assert_ne!(wide_i.ps, plain_f.ps, "RGBA32I: a plane-join shader of the integer program");
        } else {
            eprintln!("respecified: RGBA32I is refused by the GL core (closed); the integer draw is not made");
        }

        /* and the float program's cache was not disturbed: RGBA8 again */
        specify(t, 0, F8, 2);
        let o = draw0(pf);
        expect("RGBA8 last", &o, 0, None);
        assert_eq!(o.ps, plain_f.ps, "the plain shader after the whole run");
    });
}

/// (a, UI) RGBA8 -> RGBA32UI on an `isampler2D`-less path: the unsigned
/// class on a `usampler2D` program, and out of it again.
#[test]
fn respecified_to_rgba32ui_and_back() {
    closed_note("respecified", "RGBA32UI", F32UI);
    on_gc7000(|| {
        let p = program("usampler2D", "vec4(texture(s, vec2(0.5)))");
        let t = gen_texture();
        specify(t, 0, F8, 2);
        /* an RGBA8 texture on a usampler is not a legal pair; only the key
         * and the shader of the RGBA32UI draws are of interest, so start
         * from the integer 8-bit class */
        let o = {
            // SAFETY: a plain GL call; RGBA8UI is a sampled integer class.
            unsafe {
                let image = [0x3fu8; 16];
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA8UI as GLint,
                    2,
                    2,
                    0,
                    GL_RGBA_INTEGER,
                    GL_UNSIGNED_BYTE,
                    image.as_ptr().cast(),
                );
            }
            no_error("RGBA8UI");
            draw0(p)
        };
        expect("RGBA8UI", &o, 0, None);
        let wide = specify(t, 0, F32UI, 4);
        let w = draw0(p);
        let ww = expect("RGBA32UI", &w, 0, wide.then_some(F32UI));
        if ww {
            size_is("RGBA32UI", &w, 0, 4);
        }
        // SAFETY: a plain GL call.
        unsafe {
            let image = [0x3fu8; 16];
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8UI as GLint,
                2,
                2,
                0,
                GL_RGBA_INTEGER,
                GL_UNSIGNED_BYTE,
                image.as_ptr().cast(),
            );
        }
        no_error("RGBA8UI again");
        let back = draw0(p);
        expect("RGBA8UI again", &back, 0, None);
        assert_eq!(back.ps, o.ps, "the plain shader again");
        if ww {
            assert_ne!(w.ps, o.ps, "RGBA32UI: the plane-join shader");
        } else {
            assert_eq!(w.ps, o.ps, "closed: the RGBA8UI texture is still there, plain");
        }
    });
}

/// (b) The sampler `s` moved with `glUniform1i` between unit 0 (RGBA8) and
/// unit 1 (RGBA32F), the RGBA32F texture bound on unit 1 throughout: the
/// draw on unit 0 has no key and the plain shader, the draw on unit 1 has the
/// key of unit 1, and back.
#[test]
fn a_sampler_uniform_moved_between_units() {
    closed_note("uniform", "RGBA32F", F32F);
    on_gc7000(|| {
        let p = float_program();
        let (t0, t1) = (gen_texture(), gen_texture());
        specify(t0, 0, F8, 2);
        specify(t1, 1, F32F, 4);
        let a = draw_unit(p, 0);
        expect("unit 0", &a, 0, None);
        let b = draw_unit(p, 1);
        let wb = expect("unit 1", &b, 1, Some(F32F));
        size_is("unit 1", &b, 0, 4);
        let c = draw_unit(p, 0);
        expect("unit 0 again", &c, 0, None);
        assert_eq!(c.ps, a.ps, "unit 0 again: the plain shader");
        size_is("unit 0 again", &c, 0, 2);
        let d = draw_unit(p, 1);
        expect("unit 1 again", &d, 1, Some(F32F));
        assert_eq!(d.ps, b.ps, "unit 1 again: the same variant");
        if wb {
            assert_ne!(b.ps, a.ps, "unit 1: the plane-join shader");
        } else {
            assert_eq!(b.ps, a.ps, "closed: the plain shader on both units");
        }
    });
}

/// (b, 2) The other direction at once: the RGBA32F texture on unit 0 and an
/// RGBA8 one on unit 1; the key follows the uniform, not the bindings.
#[test]
fn the_key_follows_the_uniform_not_the_bound_units() {
    closed_note("uniform", "RGBA32F", F32F);
    on_gc7000(|| {
        let p = float_program();
        let (t0, t1) = (gen_texture(), gen_texture());
        specify(t0, 0, F32F, 4);
        specify(t1, 1, F8, 2);
        let a = draw_unit(p, 0);
        let wa = expect("unit 0 (wide)", &a, 0, Some(F32F));
        let b = draw_unit(p, 1);
        expect("unit 1 (RGBA8), unit 0 still RGBA32F", &b, 1, None);
        let c = draw_unit(p, 0);
        expect("unit 0 (wide) again", &c, 0, Some(F32F));
        assert_eq!(c.ps, a.ps, "the same variant again");
        assert_ne!(wa && c.ps == b.ps, true, "the plain shader of unit 1 is not the plane join");
    });
}

/// (c) One texture used by two programs. Each program asks its own
/// sensitivity and keeps its own variants: re-specifying the texture shows in
/// the next draw of either, and the other's last state is not disturbed.
#[test]
fn one_texture_two_programs() {
    closed_note("two programs", "RGBA32F", F32F);
    on_gc7000(|| {
        let a = float_program();
        let b = program("sampler2D", "texture(s, vec2(0.25)) * 0.5");
        let t = gen_texture();
        specify(t, 0, F8, 2);
        let (a0, b0) = (draw0(a), draw0(b));
        expect("A on RGBA8", &a0, 0, None);
        expect("B on RGBA8", &b0, 0, None);
        assert_ne!(a0.ps, b0.ps, "two programs, two shaders");

        specify(t, 0, F32F, 4);
        let a1 = draw0(a);
        let wa1 = expect("A on RGBA32F", &a1, 0, Some(F32F));
        /* B has not drawn: its state is what its last draw left */
        let (_, key_b) = wide128_draw_state(b).expect("B asked");
        assert_eq!(key_b, b0.key, "B's state is its own, A's draw did not touch it");
        let b1 = draw0(b);
        let wb1 = expect("B on RGBA32F", &b1, 0, Some(F32F));
        if wa1 && wb1 {
            assert_ne!(a1.ps, b1.ps, "two programs, two plane-join shaders");
            assert_ne!(a1.ps, a0.ps);
            assert_ne!(b1.ps, b0.ps);
        } else {
            assert_eq!((a1.ps, b1.ps), (a0.ps, b0.ps), "closed: each program's plain shader");
        }

        specify(t, 0, F8, 2);
        let a2 = draw0(a);
        expect("A on RGBA8 again", &a2, 0, None);
        assert_eq!(a2.ps, a0.ps);
        let (_, key_b) = wide128_draw_state(b).expect("B asked");
        assert_eq!(key_b, b1.key, "B still holds its last draw's key");
        let b2 = draw0(b);
        expect("B on RGBA8 again", &b2, 0, None);
        assert_eq!(b2.ps, b0.ps);
        /* the variants are found again */
        specify(t, 0, F32F, 4);
        let (a3, b3) = (draw0(a), draw0(b));
        assert_eq!((a3.ps, b3.ps), (a1.ps, b1.ps), "the cached variants of both programs");
    });
}

/// (d) A relink resets the cache: no state before the next draw, and the
/// next draw asks again (on RGBA32F it keys again, on RGBA8 it does not).
/// The relinked program's plain shader is its new one.
#[test]
fn a_relink_resets_the_cache() {
    closed_note("relink", "RGBA32F", F32F);
    on_gc7000(|| {
        let p = float_program();
        let t = gen_texture();
        specify(t, 0, F32F, 4);
        let a = draw0(p);
        let wa = expect("before the relink", &a, 0, Some(F32F));
        // SAFETY: plain GL calls.
        unsafe { glLinkProgram(p) };
        no_error("relink");
        assert!(wide128_draw_state(p).is_none(), "a link resets the cache: nothing before the next draw");
        let b = draw0(p);
        let _wb = expect("after the relink", &b, 0, Some(F32F));
        assert_eq!(b.key, a.key, "the same key after the relink");

        /* relink, then the texture goes RGBA8 before the first draw */
        // SAFETY: plain GL calls.
        unsafe { glLinkProgram(p) };
        assert!(wide128_draw_state(p).is_none());
        specify(t, 0, F8, 2);
        let c = draw0(p);
        expect("relinked, RGBA8", &c, 0, None);
        if wa {
            assert_ne!(c.ps, b.ps, "the plain shader, not the plane join of the old link");
        }
        /* and back */
        specify(t, 0, F32F, 4);
        let d = draw0(p);
        expect("RGBA32F after the relinked RGBA8", &d, 0, Some(F32F));
    });
}

/// (d, 2) `glDeleteTextures` of the bound RGBA32F texture: the unit is
/// unbound and the next draw has no key (and the plain shader); a texture
/// generated after it, under whatever name, is its own: RGBA8 has no key, and
/// the RGBA32F it is re-specified to has.
#[test]
fn a_deleted_texture_leaves_no_key_behind() {
    closed_note("delete", "RGBA32F", F32F);
    on_gc7000(|| {
        let p = float_program();
        let t = gen_texture();
        specify(t, 0, F32F, 4);
        let a = draw0(p);
        let wa = expect("RGBA32F", &a, 0, Some(F32F));
        // SAFETY: plain GL calls.
        unsafe { glDeleteTextures(1, &t) };
        no_error("glDeleteTextures");
        let b = draw0(p);
        expect("deleted", &b, 0, None);
        if wa {
            assert_ne!(b.ps, a.ps, "the plain shader once the texture is gone");
        }
        let n = gen_texture();
        if n != t {
            eprintln!("glGenTextures after the delete gave {n}, not {t}: a fresh name");
        }
        specify(n, 0, F8, 2);
        let c = draw0(p);
        expect("new RGBA8", &c, 0, None);
        assert_eq!(c.ps, b.ps, "the same plain shader");
        specify(n, 0, F32F, 4);
        let d = draw0(p);
        expect("new RGBA32F", &d, 0, Some(F32F));
        assert_eq!(d.ps, a.ps, "the variant of the first draw is found again");
    });
}

/// (e) A draw with no RGBA32 binding after draws with one goes back to the
/// plain shader: the unit is rebound to another texture, then unbound (name
/// 0), and each time the key is empty and the shader the first draw's.
#[test]
fn a_draw_without_an_rgba32_binding_is_plain_again() {
    closed_note("plain again", "RGBA32F", F32F);
    on_gc7000(|| {
        let p = float_program();
        let (t8, t32) = (gen_texture(), gen_texture());
        specify(t8, 0, F8, 2);
        let plain = draw0(p);
        expect("plain", &plain, 0, None);
        specify(t32, 0, F32F, 4);
        for i in 0..3 {
            let w = draw0(p);
            expect(&format!("wide {i}"), &w, 0, Some(F32F));
            /* the unit rebound to the RGBA8 texture */
            // SAFETY: plain GL calls.
            unsafe { glBindTexture(GL_TEXTURE_2D, t8) };
            let o = draw0(p);
            expect(&format!("rebound to RGBA8 {i}"), &o, 0, None);
            assert_eq!(o.ps, plain.ps, "rebound {i}: the plain shader");
            size_is("rebound", &o, 0, 2);
            // SAFETY: plain GL calls.
            unsafe { glBindTexture(GL_TEXTURE_2D, t32) };
        }
        /* unbound: texture 0, the sampled black texel */
        // SAFETY: plain GL calls.
        unsafe { glBindTexture(GL_TEXTURE_2D, 0) };
        let z = draw0(p);
        expect("unbound", &z, 0, None);
        assert_eq!(z.ps, plain.ps, "unbound: the plain shader");
    });
}
