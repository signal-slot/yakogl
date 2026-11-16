// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): several draws in one frame whose bindings differ in
//! the 128-bit class each get their own descriptors and fragment program
//! variant (docs/notes/gc-128bit-texture-sampling.md; `wide128_select` in
//! src/gl/draw.rs, the companion slot in src/vivante/texture.rs).
//!
//! A draw that binds an RGBA32F/UI/I texture to a sampled unit runs the
//! plane-joined variant of its program and writes the unit's descriptor and
//! the companion's in the next slot; a draw of the same program with a
//! narrower texture on that unit runs the plain program with no key. All
//! draws of a test go into one kick (one `glFinish` at the end); the state
//! each draw sees is the register file as the stream has written it up to
//! that draw's DRAW_INSTANCED, and the plane key is read with
//! `wide128_draw_state` straight after each draw call.
//!
//! - `a_narrow_draw_between_two_wide_draws_leaves_no_companion`: A (RGBA32F),
//!   B (RGBA8 on the same unit), A again: B has no key and the plain program,
//!   A's second draw has A's key, A's program address and A's descriptor pair.
//! - `instanced_and_indexed_draws_take_the_key_of_gl_draw_arrays`: an
//!   RGBA32UI unit under glDrawArrays, glDrawArraysInstanced and
//!   glDrawElements: one key, one program, one descriptor pair.
//! - `two_programs_alternate_over_one_rgba32i_texture`: each program keeps its
//!   own variant over four alternating draws and both see the same planes.
//!
//! The mask hook that opens the 128-bit classes (`backend::TEST_EXTRA_SAMPLED`)
//! is `#[cfg(test)]`, so no integration test can add them: until
//! GC_SAMPLED_TEXFMTS holds them (item gc-w128-flip) each test finds its class
//! closed, says so on stderr and checks the closed state instead (no draw has
//! a key); and RGBA32F, which `sized_format` still stores as halves, is not
//! checked for planes until the core stores it. From the flip on the
//! assertions run for real.
//!
//! Not in a `report-es2` build: the ES 2.0 surface has no 128-bit formats
//! (glTexImage2D refuses them as GL_INVALID_ENUM) and no GLSL ES 3.00.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_multidraw_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::buffer::*;
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

/// The words of slot `slot`'s descriptor as the draw saw it, if one was written.
fn desc(regs: &BTreeMap<u32, u32>, slot: u32) -> Option<Vec<u32>> {
    let a = *regs.get(&nte_descriptor_addr(slot))?;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/// The format word of a descriptor's CONFIG1.
fn ext_format(d: &[u32]) -> u32 {
    d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f
}

fn program(sampler: &str, scale: &str) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp {sampler};\nuniform {sampler} s;\nout vec4 o;\nvoid main() {{ o = vec4(texelFetch(s, ivec2({scale}), 0)); }}\n"
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

/// A 4x4 nearest-filtered texture, level 0 only, filled with a pattern.
fn texture(internal: GLenum, format: GLenum, ty: GLenum, bytes_per_texel: usize) -> GLuint {
    let data: Vec<u8> = (0..16 * bytes_per_texel).map(|i| (i * 7 + 1) as u8).collect();
    // SAFETY: plain GL calls; `data` holds the 4x4 image.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 4, 4, 0, format, ty, data.as_ptr().cast());
        no_error("glTexImage2D");
        t
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Arrays,
    Instanced,
    Elements,
}

/// What one draw saw.
struct Seen {
    /// plane key of the draw ([vertex, fragment]) and the program's sensitivity
    sense: [u32; 2],
    planes: [yakogl::shaderc::Wide128Planes; 2],
}

/// One quad of `kind` with program `p` and `tex` on unit 0, no flush: the key
/// of the draw, asked straight after it.
fn issue(p: GLuint, tex: GLuint, kind: Kind) -> Seen {
    // SAFETY: plain GL calls; QUAD is a static, and the element buffer (if
    // any) was bound by `element_buffer`.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        match kind {
            Kind::Arrays => glDrawArrays(GL_TRIANGLE_STRIP, 0, 4),
            Kind::Instanced => glDrawArraysInstanced(GL_TRIANGLE_STRIP, 0, 4, 3),
            Kind::Elements => glDrawElements(GL_TRIANGLE_STRIP, 4, GL_UNSIGNED_SHORT, core::ptr::null()),
        }
    }
    no_error("draw");
    let (sense, planes) = wide128_draw_state(p).expect("the draw asked");
    Seen { sense, planes }
}

/// Bind an element buffer holding 0..4 (the default VAO keeps it).
fn element_buffer() {
    static INDICES: [u16; 4] = [0, 1, 2, 3];
    // SAFETY: plain GL calls; the data is a static of 8 bytes.
    unsafe {
        let mut b: GLuint = 0;
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, b);
        glBufferData(GL_ELEMENT_ARRAY_BUFFER, 8, INDICES.as_ptr().cast(), GL_STATIC_DRAW);
    }
    no_error("element buffer");
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

fn kicks_now() -> usize {
    with_platform(|p| p.kicks.len())
}

fn ps_addr(regs: &BTreeMap<u32, u32>) -> u32 {
    *regs.get(&GC_PS_INST_ADDR).expect("a program was set for the draw")
}

/// (name, class, sampler, internal, format, type, bytes) of the 128-bit
/// texture and of the narrow one a draw puts on the same unit.
struct Pair {
    wide: (&'static str, u8, u32, &'static str),
    sampler: &'static str,
    wide_fmt: (GLenum, GLenum),
    narrow: (GLenum, GLenum, GLenum, usize),
}

fn pairs() -> [Pair; 3] {
    [
        Pair {
            wide: WIDE[0],
            sampler: "sampler2D",
            wide_fmt: (GL_RGBA, GL_FLOAT),
            narrow: (GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, 4),
        },
        Pair {
            wide: WIDE[1],
            sampler: "usampler2D",
            wide_fmt: (GL_RGBA_INTEGER, GL_UNSIGNED_INT),
            narrow: (GL_RGBA8UI, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, 4),
        },
        Pair {
            wide: WIDE[2],
            sampler: "isampler2D",
            wide_fmt: (GL_RGBA_INTEGER, GL_INT),
            narrow: (GL_RGBA8I, GL_RGBA_INTEGER, GL_BYTE, 4),
        },
    ]
}

fn wide_texture(p: &Pair) -> GLuint {
    texture(p.wide.2, p.wide_fmt.0, p.wide_fmt.1, 16)
}

fn narrow_texture(p: &Pair) -> GLuint {
    texture(p.narrow.0, p.narrow.1, p.narrow.2, p.narrow.3)
}

/// Did the GL core store the class as planes? RGBA32F is still held as halves
/// by `sized_format` (src/gl/texture.rs) whatever the mask says; say so and
/// let the caller check nothing more.
fn stored_as_planes(name: &str, ext: &str, regs: &BTreeMap<u32, u32>) -> bool {
    let d = desc(regs, 0).unwrap_or_else(|| panic!("{name}: no descriptor in slot 0"));
    let planes = ext_format(&d) == rnndb(ext);
    if !planes {
        eprintln!("{name}: the GL core does not store the class as planes (texture.rs sized_format); not checked");
    }
    planes
}

/* ---- the cases ---------------------------------------------------------- */

/// A (wide), B (narrow on the same unit), A again, one program, one frame.
/// B draws the plain program with no key; A's second draw has A's key,
/// program and descriptor pair, so B left no stale companion behind and A's
/// variant was found again.
#[test]
fn a_narrow_draw_between_two_wide_draws_leaves_no_companion() {
    for pair in &pairs() {
        let name = pair.wide.0;
        let key = yakogl::shaderc::Wide128Planes::NONE.with(0, 1);
        if closed(pair) {
            continue;
        }
        on_gc7000(|| {
            let (wide, narrow) = (wide_texture(pair), narrow_texture(pair));
            let p = program(pair.sampler, "0");
            let k0 = kicks_now();
            let a = issue(p, wide, Kind::Arrays);
            let b = issue(p, narrow, Kind::Arrays);
            let a2 = issue(p, wide, Kind::Arrays);
            let s = finish(k0, 3);
            assert_eq!([a.sense, b.sense, a2.sense], [[0, 1]; 3], "{name}: one program, one sensitivity");
            assert!(b.planes.iter().all(|k| k.is_empty()), "{name}: the narrow draw has no key");
            if !stored_as_planes(name, pair.wide.3, &s[0]) {
                return;
            }
            assert!(a.planes[0].is_empty(), "{name}: no vertex key");
            assert_eq!(a.planes[1], key, "{name}: A keys unit 0 with companion slot 1");
            assert_eq!(a2.planes[1], key, "{name}: A again has A's key");
            assert!(a2.planes[0].is_empty());

            let (a0, a1) = (desc(&s[0], 0).expect("A slot 0"), desc(&s[0], 1).expect("A slot 1 (companion)"));
            assert_eq!(ext_format(&a1), rnndb(pair.wide.3), "{name}: the companion is a plane descriptor");
            let bd = desc(&s[1], 0).expect("B slot 0");
            assert_ne!(ext_format(&bd), rnndb(pair.wide.3), "{name}: B's unit is the narrow texture, not a plane");
            assert_ne!(bd, a0, "{name}: B's descriptor is its own");
            assert_eq!(desc(&s[2], 0).as_ref(), Some(&a0), "{name}: A's descriptor after B");
            assert_eq!(desc(&s[2], 1).as_ref(), Some(&a1), "{name}: A's companion after B is A's, not B's");

            assert_ne!(ps_addr(&s[1]), ps_addr(&s[0]), "{name}: B runs the plain program, A the joined variant");
            assert_eq!(ps_addr(&s[2]), ps_addr(&s[0]), "{name}: A's variant is reused");
        });
    }
}

/// An RGBA32UI unit under glDrawArrays, glDrawArraysInstanced and
/// glDrawElements: the same key, program variant and descriptor pair.
#[test]
fn instanced_and_indexed_draws_take_the_key_of_gl_draw_arrays() {
    let pair = &pairs()[1];
    let name = pair.wide.0;
    if closed(pair) {
        return;
    }
    on_gc7000(|| {
        element_buffer();
        let wide = wide_texture(pair);
        let p = program(pair.sampler, "0");
        let k0 = kicks_now();
        let seen = [Kind::Arrays, Kind::Instanced, Kind::Elements, Kind::Arrays].map(|k| issue(p, wide, k));
        let s = finish(k0, 4);
        if !stored_as_planes(name, pair.wide.3, &s[0]) {
            return;
        }
        let (d0, d1) = (desc(&s[0], 0).expect("slot 0"), desc(&s[0], 1).expect("companion"));
        for (i, (x, regs)) in seen.iter().zip(&s).enumerate() {
            let what = ["glDrawArrays", "glDrawArraysInstanced", "glDrawElements", "glDrawArrays again"][i];
            assert_eq!(x.sense, [0, 1], "{what}");
            assert!(x.planes[0].is_empty(), "{what}: no vertex key");
            assert_eq!(x.planes[1], seen[0].planes[1], "{what}: the key of glDrawArrays");
            assert_eq!(x.planes[1], yakogl::shaderc::Wide128Planes::NONE.with(0, 1), "{what}: unit 0, companion 1");
            assert_eq!(ps_addr(regs), ps_addr(&s[0]), "{what}: the same variant");
            assert_eq!(desc(regs, 0).as_ref(), Some(&d0), "{what}: slot 0");
            assert_eq!(desc(regs, 1).as_ref(), Some(&d1), "{what}: the companion");
        }
    });
}

/// Two programs over one RGBA32I texture, drawn P1, P2, P1, P2: each has its
/// own variant (its own program address), each keeps it over its second draw,
/// and both see the same descriptor pair.
#[test]
fn two_programs_alternate_over_one_rgba32i_texture() {
    let pair = &pairs()[2];
    let name = pair.wide.0;
    if closed(pair) {
        return;
    }
    on_gc7000(|| {
        let wide = wide_texture(pair);
        let (p1, p2) = (program(pair.sampler, "0"), program(pair.sampler, "1, 0"));
        let k0 = kicks_now();
        let seen = [p1, p2, p1, p2].map(|p| issue(p, wide, Kind::Arrays));
        let s = finish(k0, 4);
        if !stored_as_planes(name, pair.wide.3, &s[0]) {
            return;
        }
        let key = yakogl::shaderc::Wide128Planes::NONE.with(0, 1);
        for (i, x) in seen.iter().enumerate() {
            assert_eq!(x.planes[1], key, "{name}: draw {i}: unit 0, companion 1");
            assert!(x.planes[0].is_empty(), "{name}: draw {i}: no vertex key");
        }
        assert_eq!(ps_addr(&s[2]), ps_addr(&s[0]), "{name}: P1's variant is reused");
        assert_eq!(ps_addr(&s[3]), ps_addr(&s[1]), "{name}: P2's variant is reused");
        assert_ne!(ps_addr(&s[0]), ps_addr(&s[1]), "{name}: each program has its own variant");
        let (d0, d1) = (desc(&s[0], 0).expect("slot 0"), desc(&s[0], 1).expect("companion"));
        for (i, regs) in s.iter().enumerate() {
            assert_eq!(desc(regs, 0).as_ref(), Some(&d0), "{name}: draw {i}: slot 0");
            assert_eq!(desc(regs, 1).as_ref(), Some(&d1), "{name}: draw {i}: the companion");
        }
        /* each program's key is its own last draw's, from the program's state */
        for p in [p1, p2] {
            let (_, planes) = wide128_draw_state(p).expect("asked");
            assert_eq!(planes[1], key, "{name}: program {p}");
        }
    });
}

/// Is the pair's 128-bit class still closed (not in GC_SAMPLED_TEXFMTS)? Then
/// the GL core stores no 128-bit texture for it, no draw can carry a key, and
/// the multi-draw cases are not exercised; this says so and checks the bind.
fn closed(pair: &Pair) -> bool {
    if open(pair.wide.1) {
        return false;
    }
    eprintln!(
        "{}: not in GC_SAMPLED_TEXFMTS; the multi-draw plane variants are not exercised (gc-w128-flip)",
        pair.wide.0
    );
    let bind = GlesTextureBind { format: pair.wide.1, ..Default::default() };
    assert!(!yakogl::vivante::texture::wide128_sampled(&bind), "{}: no planes while closed", pair.wide.0);
    true
}
