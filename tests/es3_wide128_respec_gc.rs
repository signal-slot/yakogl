// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the per-draw 128-bit plane key and the shader a draw
//! submits follow the GL state from draw to draw (docs/notes/gc-128bit-
//! texture-sampling.md; `wide128_select` and `w128_variant_for` in
//! src/gl/draw.rs, the companion slot in src/vivante/texture.rs).
//!
//! The program's plane sensitivity is asked once and kept until a link; the
//! variant is cached per plane key. Neither may hold a draw to the state of
//! an earlier one:
//!
//! - (a) `one_name_respecified_rgba8_rgba32f_rgba8_rgba32i`: one texture name
//!   re-specified RGBA8 -> RGBA32F -> RGBA8 -> RGBA32I between draws, and
//!   `integer_name_respecified_narrow_wide_narrow_wide`: RGBA8I/UI ->
//!   RGBA32I/UI -> RGBA8I/UI -> RGBA32I/UI on one program;
//! - (b) `uniform1i_moves_the_sampler_between_a_narrow_and_a_wide_unit`: the
//!   sampler uniform set to an RGBA8 unit, an RGBA32 unit, back and again;
//! - (c) `one_texture_under_two_programs`: one RGBA32 texture read by a
//!   one-sampler and a two-sampler program in turn, each keeping its key;
//! - (d) `relink_resets_the_key_and_a_recreated_name_is_new`: a relink of the
//!   program with another fragment shader (sensitivity and key asked again),
//!   and glDeleteTextures plus re-creation under the same name, narrow and
//!   then wide;
//! - (e) `a_draw_without_a_wide_binding_goes_back_to_the_plain_shader`: after
//!   a keyed draw, a narrow texture, no texture and a program without
//!   samplers all draw with no key and the plain program.
//!
//! All draws of a case go into one frame (one `glFinish` at the end); the
//! state each draw saw is the register file as the stream has written it up
//! to that draw's DRAW_INSTANCED, and the key is read with
//! `wide128_draw_state` straight after each draw call.
//!
//! Which bindings are two planes is read from the draw (the G32R32 word in
//! the binding's descriptor) and held against the mask: the integer classes
//! are two planes exactly when the mask holds them; GL_RGBA32F is still held
//! as halves by the GL core (src/gl/texture.rs `sized_format`), so its
//! binding is one descriptor with no key, said on stderr, and from the day
//! the core stores it as planes the same draws check its pair. Each case runs
//! twice: with the mask as built, and with the three classes taken out of it
//! (`backend::TEST_HIDDEN_SAMPLED`), the closed state: the integer uploads
//! refused, no draw keyed, no G32R32 descriptor.
//!
//! Not in a `report-es2` build: the ES 2.0 surface has no 128-bit formats
//! and no GLSL ES 3.00.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_respec_gc
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

/* ---- the stand-in OSAL (as tests/es3_wide128_multidraw_gc.rs) ----------- */

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
/// Serialises the tests: the OSAL pointer, the back end, `g_backend` and the
/// mask hook are process-global.
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

/// Each case, with the mask as built and with the 128-bit classes closed.
fn both(case: impl Fn(bool)) {
    for closed in [false, true] {
        on_gc7000(closed, || case(closed));
    }
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

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// A fragment shader reading texel (0, 0) of each sampler of `names`, all of
/// type `sampler`, declared in that order (binding n: the n-th name).
fn fs_text(sampler: &str, names: &[&str]) -> String {
    let decl: String = names.iter().map(|n| format!("uniform {sampler} {n};\n")).collect();
    let sum: Vec<String> = names.iter().map(|n| format!("vec4(texelFetch({n}, ivec2(0), 0))")).collect();
    let body = if sum.is_empty() { "vec4(0.25)".to_string() } else { sum.join(" + ") };
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp {sampler};\n{decl}out vec4 o;\nvoid main() {{ o = {body}; }}\n"
    )
}

fn link(p: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
    }
}

/// A program of `fs`, its vertex shader `VS`; returns (program, fs shader).
fn program_fs(fs: &str) -> (GLuint, GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        let f = shader(GL_FRAGMENT_SHADER, fs);
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, f);
        link(p);
        (p, f)
    }
}

/// A program reading `names` (samplers of type `sampler`), each sampler
/// uniform set to unit 0.
fn program(sampler: &str, names: &[&str]) -> GLuint {
    let (p, _) = program_fs(&fs_text(sampler, names));
    set_units(p, &names.iter().map(|n| (*n, 0)).collect::<Vec<_>>());
    p
}

fn set_units(p: GLuint, units: &[(&str, GLint)]) {
    // SAFETY: plain GL calls; NUL-terminated names.
    unsafe {
        glUseProgram(p);
        for (n, u) in units {
            let n = CString::new(*n).unwrap();
            glUniform1i(glGetUniformLocation(p, n.as_ptr()), *u);
        }
    }
    no_error("sampler uniforms");
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/* ---- the classes ---------------------------------------------------------- */

/// A texel class a case puts on a unit: name, GL core class, internal
/// format, upload format and type, bytes a channel, and the G32R32 word of
/// its planes (`None`: no 128-bit class).
struct Class {
    name: &'static str,
    gles: u8,
    internal: GLenum,
    format: GLenum,
    ty: GLenum,
    mesa: Option<&'static str>,
}

const RGBA8: Class = Class {
    name: "RGBA8",
    gles: GLES_TEXFMT_RGBA8,
    internal: GL_RGBA8,
    format: GL_RGBA,
    ty: GL_UNSIGNED_BYTE,
    mesa: None,
};
const RGBA8I: Class = Class {
    name: "RGBA8I",
    gles: GLES_TEXFMT_RGBA8I,
    internal: GL_RGBA8I,
    format: GL_RGBA_INTEGER,
    ty: GL_BYTE,
    mesa: None,
};
const RGBA8UI: Class = Class {
    name: "RGBA8UI",
    gles: GLES_TEXFMT_RGBA8UI,
    internal: GL_RGBA8UI,
    format: GL_RGBA_INTEGER,
    ty: GL_UNSIGNED_BYTE,
    mesa: None,
};
const RGBA32F: Class = Class {
    name: "RGBA32F",
    gles: GLES_TEXFMT_RGBA32F,
    internal: GL_RGBA32F,
    format: GL_RGBA,
    ty: GL_FLOAT,
    mesa: Some("TEXTURE_FORMAT_EXT_G32R32F"),
};
const RGBA32I: Class = Class {
    name: "RGBA32I",
    gles: GLES_TEXFMT_RGBA32I,
    internal: GL_RGBA32I,
    format: GL_RGBA_INTEGER,
    ty: GL_INT,
    mesa: Some("TEXTURE_FORMAT_EXT_G32R32I"),
};
const RGBA32UI: Class = Class {
    name: "RGBA32UI",
    gles: GLES_TEXFMT_RGBA32UI,
    internal: GL_RGBA32UI,
    format: GL_RGBA_INTEGER,
    ty: GL_UNSIGNED_INT,
    mesa: Some("TEXTURE_FORMAT_EXT_G32R32I"),
};

/// (narrow, wide, sampler type) of the two integer 128-bit classes.
const INT_PAIRS: [(&Class, &Class, &str); 2] = [(&RGBA8I, &RGBA32I, "isampler2D"), (&RGBA8UI, &RGBA32UI, "usampler2D")];

fn open(c: &Class) -> bool {
    sampled_texfmts() & gles_texfmt_bit(c.gles) != 0
}

fn gen_texture() -> GLuint {
    let mut t: GLuint = 0;
    // SAFETY: plain GL call; one name.
    unsafe { glGenTextures(1, &mut t) };
    t
}

/// Bind `t` on unit `unit` (nearest filtering) and give it a 4x4 level 0 of
/// `c`. Texel (x, y) channel k holds `seed + 4 * (y * 4 + x) + k` (the float
/// of that value for RGBA32F). A closed integer 128-bit class is refused
/// (GL_INVALID_OPERATION) and leaves the texture as it was.
fn spec(unit: u32, t: GLuint, c: &Class, seed: u32) {
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
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, c.internal as GLint, 4, 4, 0, c.format, c.ty, bytes.as_ptr().cast());
    }
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    if c.gles != GLES_TEXFMT_RGBA32F && c.mesa.is_some() && !open(c) {
        assert_eq!(e, GL_INVALID_OPERATION, "{} on unit {unit}, closed: the upload is refused", c.name);
    } else {
        assert_eq!(e, GL_NO_ERROR, "{} on unit {unit}", c.name);
    }
}

/// The words texel (0, 0) of a `spec` texture holds.
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

/* ---- draws and what they submitted -------------------------------------- */

/// What one draw saw, asked straight after it.
#[derive(Clone, Copy)]
struct Seen {
    sense: [u32; 2],
    planes: [Wide128Planes; 2],
}

/// One quad with program `p` and the units as they are, no flush.
fn draw(p: GLuint) -> Seen {
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    let (sense, planes) = wide128_draw_state(p).expect("the draw asked for its key");
    Seen { sense, planes }
}

/// The register file as the stream has set it at each DRAW_INSTANCED, in order.
fn per_draw_state(words: &[u32]) -> Vec<BTreeMap<u32, u32>> {
    let mut regs = BTreeMap::new();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    if let Some(v) = words.get(i + 1 + k) {
                        regs.insert(base + 4 * k as u32, *v);
                    }
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                out.push(regs.clone());
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

fn kicks_now() -> usize {
    with_platform(|p| p.kicks.len())
}

/// Flush the frame and return the register file at each draw since `kicks0`.
fn finish(kicks0: usize, draws: usize) -> Vec<BTreeMap<u32, u32>> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    no_error("glFinish");
    let words: Vec<u32> = with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the draws produced no kick");
        p.kicks[kicks0..].concat()
    });
    let snaps = per_draw_state(&words);
    assert_eq!(snaps.len(), draws, "one DRAW_INSTANCED per draw call");
    snaps
}

/// The words of slot `slot`'s descriptor as the draw saw it, if one was written.
fn desc(regs: &BTreeMap<u32, u32>, slot: u32) -> Option<Vec<u32>> {
    let a = *regs.get(&nte_descriptor_addr(slot))?;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

fn ext_format(d: &[u32]) -> u32 {
    d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f
}

fn is_plane(regs: &BTreeMap<u32, u32>, slot: u32) -> bool {
    desc(regs, slot).is_some_and(|d| {
        let ext = ext_format(&d);
        ext == rnndb("TEXTURE_FORMAT_EXT_G32R32F") || ext == rnndb("TEXTURE_FORMAT_EXT_G32R32I")
    })
}

fn ps_addr(regs: &BTreeMap<u32, u32>) -> u32 {
    *regs.get(&GC_PS_INST_ADDR).expect("a program was set for the draw")
}

/// What a binding of a draw held: its binding (slot), the class sampled and
/// the seed of the image when it is still alive at the flush (`None`: the
/// texture was re-specified or deleted later in the frame, the texels are
/// not read back).
type Bound<'a> = (u8, &'a Class, Option<u32>);

/// Check the fragment bindings `bound` of one draw (`slots`: the slots the
/// stage owns) against what it submitted: which are two planes, read from the
/// descriptor and held against the mask (the integer classes exactly when
/// open; RGBA32F never when closed, and said on stderr when open and still
/// halves); the key `seen` carries for those, with their companions at
/// `slots + rank`; each pair's G32R32 word, the companion's level address
/// half-way into the 4x4 level, and the texels of a live image. Returns
/// whether any binding was two planes.
fn check(what: &str, seen: &Seen, regs: &BTreeMap<u32, u32>, slots: u8, bound: &[Bound]) -> bool {
    let mut pairs: Vec<(u8, u8)> = Vec::new();
    for &(b, c, seed) in bound {
        let slot = u32::from(b);
        let planes = is_plane(regs, slot);
        let what = format!("{what}: binding {b} ({})", c.name);
        match c.mesa {
            None => assert!(!planes, "{what}: not a 128-bit class, not a plane"),
            Some(_) if c.gles == GLES_TEXFMT_RGBA32F => {
                assert!(!planes || open(c), "{what}: planes only when the mask holds the class");
                if open(c) && !planes {
                    eprintln!("{what}: the GL core stores GL_RGBA32F as halves (texture.rs sized_format); no pair");
                }
            }
            Some(_) => assert_eq!(planes, open(c), "{what}: two planes exactly when the mask holds the class"),
        }
        if !planes {
            continue;
        }
        let comp = slots + pairs.len() as u8;
        pairs.push((b, comp));
        let d0 = desc(regs, slot).expect("the binding's descriptor");
        let d1 = desc(regs, u32::from(comp)).unwrap_or_else(|| panic!("{what}: no companion in slot {comp}"));
        let ext = rnndb(c.mesa.expect("a 128-bit class"));
        assert_eq!(ext_format(&d0), ext, "{what}: the R,G plane is the class's G32R32");
        assert_eq!(ext_format(&d1), ext, "{what}: the companion is the class's G32R32");
        let lod0 = texdesc_lod_addr(0) as usize / 4;
        assert_eq!(d1[lod0], d0[lod0] + 4 * 4 * 8, "{what}: plane 1 half-way into the level");
        if let Some(seed) = seed {
            let want = first_texel(c, seed);
            assert_eq!(words_at(u64::from(d0[lod0]), 2), want[..2], "{what}: R,G of (0, 0)");
            assert_eq!(words_at(u64::from(d1[lod0]), 2), want[2..], "{what}: B,A of (0, 0)");
        }
    }
    let key = pairs.iter().fold(Wide128Planes::NONE, |k, &(b, c)| k.with(b, c));
    assert!(seen.planes[0].is_empty(), "{what}: no vertex key");
    assert_eq!(seen.planes[1], key, "{what}: the fragment key follows the draw's bindings");
    !pairs.is_empty()
}

/// The plain program draws `plain` and the keyed draws `keyed` (of one
/// program and one key): each group shares its program address, and the two
/// differ when any draw was keyed.
fn check_programs(what: &str, s: &[BTreeMap<u32, u32>], plain: &[usize], keyed: &[usize]) {
    for w in plain.windows(2) {
        assert_eq!(ps_addr(&s[w[1]]), ps_addr(&s[w[0]]), "{what}: draws {} and {}: the plain program", w[0], w[1]);
    }
    for w in keyed.windows(2) {
        assert_eq!(ps_addr(&s[w[1]]), ps_addr(&s[w[0]]), "{what}: draws {} and {}: one variant", w[0], w[1]);
    }
    if let (Some(&p), Some(&k)) = (plain.first(), keyed.first()) {
        assert_ne!(ps_addr(&s[k]), ps_addr(&s[p]), "{what}: draw {k} runs the joined variant, draw {p} the plain");
    }
}

/* ---- the cases ---------------------------------------------------------- */

/// (a) One texture name re-specified between draws: RGBA8 (sampler2D
/// program), RGBA32F (the same program), RGBA8 again, then RGBA32I (an
/// isampler2D program). Each draw has the key of the class the name holds at
/// that draw: none for RGBA8, the pair for RGBA32F once the core stores it as
/// planes, the pair for RGBA32I when open; RGBA8 after RGBA32F runs the plain
/// program again.
#[test]
fn one_name_respecified_rgba8_rgba32f_rgba8_rgba32i() {
    both(|closed| {
        let pf = program("sampler2D", &["s"]);
        let pi = program("isampler2D", &["s"]);
        let t = gen_texture();
        let k0 = kicks_now();
        let mut seen = Vec::new();
        for (i, (c, p)) in [(&RGBA8, pf), (&RGBA32F, pf), (&RGBA8, pf), (&RGBA32I, pi)].into_iter().enumerate() {
            spec(0, t, c, 10 * i as u32 + 1);
            seen.push(draw(p));
        }
        let s = finish(k0, 4);
        let what = |i: usize| format!("closed {closed}: draw {i}");
        assert_eq!(seen.iter().map(|x| x.sense).collect::<Vec<_>>(), [[0, 1]; 4], "one sampler read at binding 0");
        check(&what(0), &seen[0], &s[0], 1, &[(0, &RGBA8, None)]);
        let f_planes = check(&what(1), &seen[1], &s[1], 1, &[(0, &RGBA32F, None)]);
        check(&what(2), &seen[2], &s[2], 1, &[(0, &RGBA8, None)]);
        /* closed: the RGBA32I upload is refused and the name keeps RGBA8 */
        let last = if closed { &RGBA8 } else { &RGBA32I };
        let i_planes = check(&what(3), &seen[3], &s[3], 1, &[(0, last, (!closed).then_some(31))]);
        assert_eq!(i_planes, !closed, "closed {closed}: RGBA32I is two planes exactly when open");
        check_programs(&what(2), &s, &[0, 2], if f_planes { &[1] } else { &[] });
        if !f_planes {
            assert_eq!(ps_addr(&s[1]), ps_addr(&s[0]), "closed {closed}: RGBA32F as halves runs the plain program");
        }
        assert_ne!(ps_addr(&s[3]), ps_addr(&s[0]), "closed {closed}: the isampler2D program is another program");
    });
}

/// (a) RGBA8I/UI -> RGBA32I/UI -> RGBA8I/UI -> RGBA32I/UI on one name and one
/// program: the second wide draw finds the first's variant again (its key is
/// cached per program), the narrow draws between run the plain program, and
/// each wide draw's planes are the storage the name holds at that draw.
#[test]
fn integer_name_respecified_narrow_wide_narrow_wide() {
    for (narrow, wide, sampler) in INT_PAIRS {
        both(|closed| {
            let p = program(sampler, &["s"]);
            let t = gen_texture();
            let k0 = kicks_now();
            let classes = [narrow, wide, narrow, wide];
            let seen: Vec<Seen> = classes
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    spec(0, t, c, 40 * i as u32 + 3);
                    draw(p)
                })
                .collect();
            let s = finish(k0, 4);
            let mut keyed = Vec::new();
            for (i, c) in classes.iter().enumerate() {
                let what = format!("{}, closed {closed}: draw {i}", wide.name);
                /* closed: the wide uploads are refused, the name keeps the narrow image */
                let held = if closed { narrow } else { c };
                let seed = (i == 3 && !closed).then_some(40 * 3 + 3);
                if check(&what, &seen[i], &s[i], 1, &[(0, held, seed)]) {
                    keyed.push(i);
                }
            }
            assert_eq!(keyed, if closed { vec![] } else { vec![1, 3] }, "{}, closed {closed}: keyed draws", wide.name);
            let plain: Vec<usize> = (0..4).filter(|i| !keyed.contains(i)).collect();
            check_programs(&format!("{}, closed {closed}", wide.name), &s, &plain, &keyed);
        });
    }
}

/// (b) glUniform1i moves the sampler of one program between unit 0 (a
/// narrow texture) and unit 1 (an RGBA32 texture), four draws: the key goes
/// on and off with the unit the uniform names, the binding stays 0 and its
/// companion slot 1, and the plain and the joined program alternate.
#[test]
fn uniform1i_moves_the_sampler_between_a_narrow_and_a_wide_unit() {
    let sets: [(&Class, &Class, &str); 3] = [(&RGBA8, &RGBA32F, "sampler2D"), INT_PAIRS[0], INT_PAIRS[1]];
    for (narrow, wide, sampler) in sets {
        both(|closed| {
            let p = program(sampler, &["s"]);
            let (tn, tw) = (gen_texture(), gen_texture());
            spec(0, tn, narrow, 5);
            spec(1, tw, wide, 70);
            let k0 = kicks_now();
            let units = [0, 1, 0, 1];
            let seen: Vec<Seen> = units
                .iter()
                .map(|&u| {
                    set_units(p, &[("s", u)]);
                    draw(p)
                })
                .collect();
            let s = finish(k0, 4);
            let mut keyed = Vec::new();
            for (i, &u) in units.iter().enumerate() {
                let what = format!("{}, closed {closed}: draw {i} (unit {u})", wide.name);
                /* a closed integer upload is refused: unit 1 holds an incomplete texture */
                let b: Bound = if u == 0 { (0, narrow, Some(5)) } else { (0, wide, Some(70)) };
                if check(&what, &seen[i], &s[i], 1, &[b]) {
                    keyed.push(i);
                }
            }
            let expect: Vec<usize> = if open(wide) && wide.gles != GLES_TEXFMT_RGBA32F { vec![1, 3] } else { vec![] };
            if wide.gles != GLES_TEXFMT_RGBA32F {
                assert_eq!(keyed, expect, "{}, closed {closed}: the draws on unit 1 are keyed", wide.name);
            }
            check_programs(&format!("{}, closed {closed}", wide.name), &s, &[0, 2], &keyed);
        });
    }
}

/// (c) One RGBA32 texture on unit 0 under two programs, drawn P1, P2, P1, P2:
/// P1 reads one sampler (key: binding 0, companion 1), P2 two samplers both
/// set to unit 0 (key: bindings 0 and 1, companions 2 and 3). Each draw has
/// its own program's key, each program finds its variant again, and each
/// program's `wide128_draw_state` is its own last draw's. Then the texture is
/// re-specified narrow and both programs draw plain.
#[test]
fn one_texture_under_two_programs() {
    for (narrow, wide, sampler) in INT_PAIRS {
        both(|closed| {
            let p1 = program(sampler, &["s"]);
            let p2 = program(sampler, &["a", "s"]);
            let t = gen_texture();
            spec(0, t, wide, 9);
            let k0 = kicks_now();
            let mut seen: Vec<Seen> = [p1, p2, p1, p2].map(draw).to_vec();
            spec(0, t, narrow, 11);
            seen.extend([p1, p2].map(draw));
            let s = finish(k0, 6);
            let tag = format!("{}, closed {closed}", wide.name);
            let held = if closed { None } else { Some(wide) };
            for i in 0..4 {
                let what = format!("{tag}: draw {i}");
                let c = held.unwrap_or(&RGBA8I);
                let keyed = if i % 2 == 0 {
                    assert_eq!(seen[i].sense, [0, 1], "{what}: P1 reads binding 0");
                    check(&what, &seen[i], &s[i], 1, &[(0, c, None)])
                } else {
                    assert_eq!(seen[i].sense, [0, 3], "{what}: P2 reads bindings 0 and 1");
                    check(&what, &seen[i], &s[i], 2, &[(0, c, None), (1, c, None)])
                };
                assert_eq!(keyed, !closed, "{what}: keyed exactly when open");
            }
            for i in 4..6 {
                let what = format!("{tag}: draw {i}, re-specified {}", narrow.name);
                let slots = if i == 4 { 1 } else { 2 };
                let b: Vec<Bound> = (0..slots).map(|b| (b, narrow, Some(11))).collect();
                assert!(!check(&what, &seen[i], &s[i], slots, &b), "{what}: no planes");
            }
            for p in [p1, p2] {
                let (_, planes) = wide128_draw_state(p).expect("asked");
                assert_eq!(planes, [Wide128Planes::NONE; 2], "{tag}: program {p}'s last draw was narrow");
            }
            if !closed {
                assert_eq!(ps_addr(&s[2]), ps_addr(&s[0]), "{tag}: P1's variant is reused");
                assert_eq!(ps_addr(&s[3]), ps_addr(&s[1]), "{tag}: P2's variant is reused");
                assert_ne!(ps_addr(&s[0]), ps_addr(&s[1]), "{tag}: each program has its own variant");
                assert_ne!(ps_addr(&s[4]), ps_addr(&s[0]), "{tag}: P1 narrow runs its plain program");
                assert_ne!(ps_addr(&s[5]), ps_addr(&s[1]), "{tag}: P2 narrow runs its plain program");
            }
        });
    }
}

/// (d) A relink resets the program's sensitivity and key: after a keyed draw,
/// `glLinkProgram` with another fragment shader (two samplers on unit 0)
/// leaves `wide128_draw_state` empty, and the next draw asks again and has
/// the two-binding key. Then the texture is deleted (the unit falls back to
/// texture 0, no key), re-created under the same name as a narrow texture
/// (no key) and again as an RGBA32 texture (the key, the new image's texels).
#[test]
fn relink_resets_the_key_and_a_recreated_name_is_new() {
    for (narrow, wide, sampler) in INT_PAIRS {
        both(|closed| {
            let tag = format!("{}, closed {closed}", wide.name);
            let (p, f1) = program_fs(&fs_text(sampler, &["s"]));
            set_units(p, &[("s", 0)]);
            let t = gen_texture();
            spec(0, t, wide, 1);
            let k0 = kicks_now();
            let mut seen = vec![draw(p)];
            // SAFETY: plain GL calls; the shaders were made above.
            unsafe {
                glDetachShader(p, f1);
                glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs_text(sampler, &["a", "s"])));
            }
            link(p);
            assert!(wide128_draw_state(p).is_none(), "{tag}: a link forgets the sensitivity and the key");
            set_units(p, &[("a", 0), ("s", 0)]);
            seen.push(draw(p));
            assert_eq!(seen[0].sense, [0, 1], "{tag}: before the relink, binding 0");
            assert_eq!(seen[1].sense, [0, 3], "{tag}: after the relink, bindings 0 and 1");

            /* delete the name, then re-create it: narrow, then wide */
            // SAFETY: plain GL calls; one name.
            unsafe {
                glActiveTexture(GL_TEXTURE0);
                glDeleteTextures(1, &t);
            }
            no_error("glDeleteTextures");
            seen.push(draw(p));
            spec(0, t, narrow, 50);
            seen.push(draw(p));
            // SAFETY: as above.
            unsafe { glDeleteTextures(1, &t) };
            spec(0, t, wide, 90);
            seen.push(draw(p));
            let s = finish(k0, 5);

            let c = if closed { &RGBA8I } else { wide };
            let k = check(&format!("{tag}: draw 0"), &seen[0], &s[0], 1, &[(0, c, None)]);
            assert_eq!(k, !closed, "{tag}: draw 0 keyed exactly when open");
            let k = check(&format!("{tag}: draw 1"), &seen[1], &s[1], 2, &[(0, c, None), (1, c, None)]);
            assert_eq!(k, !closed, "{tag}: draw 1 (relinked) keyed exactly when open");
            assert!(seen[2].planes.iter().all(Wide128Planes::is_empty), "{tag}: draw 2: a deleted texture, no key");
            assert!(!is_plane(&s[2], 0), "{tag}: draw 2: no plane on binding 0");
            let narrow_b: [Bound; 2] = [(0, narrow, Some(50)), (1, narrow, Some(50))];
            assert!(!check(&format!("{tag}: draw 3"), &seen[3], &s[3], 2, &narrow_b), "{tag}: draw 3 narrow");
            /* closed: the wide re-creation is refused, the new name holds no image */
            if closed {
                assert!(seen[4].planes.iter().all(Wide128Planes::is_empty), "{tag}: draw 4 closed: no key");
            } else {
                let wide_b: [Bound; 2] = [(0, wide, Some(90)), (1, wide, Some(90))];
                assert!(check(&format!("{tag}: draw 4"), &seen[4], &s[4], 2, &wide_b), "{tag}: draw 4 keyed");
                assert_eq!(seen[4].planes, seen[1].planes, "{tag}: draw 4 has the relinked program's key");
                assert_eq!(ps_addr(&s[4]), ps_addr(&s[1]), "{tag}: the relinked program's variant is reused");
                assert_ne!(ps_addr(&s[3]), ps_addr(&s[4]), "{tag}: the narrow draw runs the plain program");
            }
        });
    }
}

/// (e) After a keyed draw, the same program with a narrow texture on the
/// unit, with no texture on it (texture 0), and a program that samples
/// nothing: none has a key, the program's narrow draws run its plain program
/// (the one its first, narrow, draw ran), and the key comes back for a wide
/// draw after them.
#[test]
fn a_draw_without_a_wide_binding_goes_back_to_the_plain_shader() {
    for (narrow, wide, sampler) in INT_PAIRS {
        both(|closed| {
            let tag = format!("{}, closed {closed}", wide.name);
            let p = program(sampler, &["s"]);
            let (q, _) = program_fs(&fs_text(sampler, &[]));
            let (tn, tw) = (gen_texture(), gen_texture());
            spec(0, tn, narrow, 2);
            spec(0, tw, wide, 60);
            let bind = |t: GLuint| {
                // SAFETY: plain GL calls.
                unsafe {
                    glActiveTexture(GL_TEXTURE0);
                    glBindTexture(GL_TEXTURE_2D, t);
                }
            };
            let k0 = kicks_now();
            bind(tn);
            let n0 = draw(p);
            bind(tw);
            let w0 = draw(p);
            bind(tn);
            let n1 = draw(p);
            bind(tw);
            let w1 = draw(p);
            bind(0);
            let none = draw(p);
            bind(tw);
            let w2 = draw(p);
            let other = draw(q);
            bind(tn);
            let n2 = draw(p);
            let s = finish(k0, 8);

            let wc = if closed { None } else { Some(wide) };
            let mut keyed = Vec::new();
            for (i, x, c) in [(0, &n0, Some(narrow)), (1, &w0, wc), (2, &n1, Some(narrow)), (3, &w1, wc)]
                .into_iter()
                .chain([(5, &w2, wc), (7, &n2, Some(narrow))])
            {
                let what = format!("{tag}: draw {i}");
                match c {
                    Some(c) => {
                        let seed = if c.mesa.is_some() { 60 } else { 2 };
                        if check(&what, x, &s[i], 1, &[(0, c, Some(seed))]) {
                            keyed.push(i);
                        }
                    }
                    None => assert!(x.planes.iter().all(Wide128Planes::is_empty), "{what}: closed, no key"),
                }
            }
            assert!(none.planes.iter().all(Wide128Planes::is_empty), "{tag}: draw 4, texture 0: no key");
            assert!(!is_plane(&s[4], 0), "{tag}: draw 4, texture 0: no plane");
            assert_eq!(other.sense, [0, 0], "{tag}: draw 6: a program that samples nothing");
            assert!(other.planes.iter().all(Wide128Planes::is_empty), "{tag}: draw 6: no key");
            assert_eq!(keyed, if closed { vec![] } else { vec![1, 3, 5] }, "{tag}: the wide draws are keyed");
            check_programs(&tag, &s, &[0, 2, 7], &keyed);
            assert_eq!(ps_addr(&s[4]), ps_addr(&s[0]), "{tag}: draw 4 (texture 0) runs the plain program");
            assert_ne!(ps_addr(&s[6]), ps_addr(&s[0]), "{tag}: draw 6 is another program");
            if !closed {
                assert_ne!(ps_addr(&s[6]), ps_addr(&s[5]), "{tag}: draw 6 is not the joined variant");
            }
            let (_, last) = wide128_draw_state(p).expect("asked");
            assert_eq!(last, [Wide128Planes::NONE; 2], "{tag}: the program's last draw was narrow");
        });
    }
}
