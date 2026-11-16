// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): sampler objects and texture parameters reach both
//! planes of an RGBA32F / RGBA32UI texture
//! (docs/notes/gc-128bit-texture-sampling.md, gc-128bit-swizzle.md).
//!
//! A 128-bit binding is the primary slot plus a companion slot after the
//! stage's own, and the companion takes the same sampler state as its
//! binding (etnaviv_texture_desc.c:499-513). Per class, from the kick's
//! SAMP_CTRL0/CTRL1/LOD_MINMAX/LOD_BIAS writes and the two descriptors:
//!
//! - wrap S/T, min/mag filter (NEAREST, the only ones ES 3.0 completes these
//!   classes with), base/max level and min/max LOD set with `glTexParameter`
//!   appear identically in both slots, and the wrap and filter fields hold
//!   the values set;
//! - a sampler object bound with `glBindSampler` overrides the texture's
//!   wrap and filters in both slots, and unbinding it returns to the
//!   texture's own state;
//! - one sampler object shared by an RGBA32 unit and an RGBA8 unit in one
//!   draw: the RGBA8 unit gets no companion slot, and all three slots carry
//!   the sampler's state.
//!
//! The 128-bit classes are not in GC_SAMPLED_TEXFMTS yet (item
//! gc-w128-flip) and the mask hook (`backend::TEST_EXTRA_SAMPLED`) is
//! `#[cfg(test)]`, so no integration test can open them. While they are
//! closed each test says so on stderr and checks the closed state (the GL
//! core stores no such texture, so no companion slot is written); from the
//! flip on they run for real, as tests/es3_wide128_planes_gc.rs does.
//! Before the flip they were run with the classes added to the mask locally.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_sampler_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

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

/// (name, class, internal format, plane word, GLSL sampler, upload format, upload type)
type Class = (&'static str, u8, u32, &'static str, &'static str, GLenum, GLenum);

const WIDE: [Class; 2] = [
    ("RGBA32F", GLES_TEXFMT_RGBA32F, GL_RGBA32F, "TEXTURE_FORMAT_EXT_G32R32F", "sampler2D", GL_RGBA, GL_FLOAT),
    (
        "RGBA32UI",
        GLES_TEXFMT_RGBA32UI,
        GL_RGBA32UI,
        "TEXTURE_FORMAT_EXT_G32R32I",
        "usampler2D",
        GL_RGBA_INTEGER,
        GL_UNSIGNED_INT,
    ),
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

/* ---- the draw and the words it writes ------------------------------------ */

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// A program reading each `(sampler type, uniform name)` with `texelFetch`
/// at level 0 and adding the results (integer results converted to float).
fn program(samplers: &[(&str, &str)]) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (ty, name) in samplers {
        fs += &format!("precision highp {ty};\nuniform {ty} {name};\n");
    }
    fs += "out vec4 o;\nvoid main() {\n  o = vec4(0.0);\n";
    for (_, name) in samplers {
        fs += &format!("  o += vec4(texelFetch({name}, ivec2(0), 0));\n");
    }
    fs += "}\n";
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
        glUseProgram(p);
        for (unit, (_, name)) in samplers.iter().enumerate() {
            let n = CString::new(*name).unwrap();
            glUniform1i(glGetUniformLocation(p, n.as_ptr()), unit as GLint);
        }
        p
    }
}

/// One quad with program `p`: the last kick's writes.
fn draw(p: GLuint) -> Vec<(u32, u32)> {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
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

/// The last value written to `r`.
fn reg(stream: &[(u32, u32)], r: u32) -> Option<u32> {
    stream.iter().rev().find(|(x, _)| *x == r).map(|w| w.1)
}

/// Everything a slot's sampler holds: CTRL0, CTRL1, LOD_MINMAX, LOD_BIAS,
/// ANISOTROPY, and the descriptor's words but for the level addresses (the
/// first sixteen), as one comparable record.
#[derive(Debug, PartialEq, Eq, Clone)]
struct Slot {
    ctrl0: u32,
    ctrl1: u32,
    lod_minmax: u32,
    lod_bias: u32,
    aniso: u32,
    desc: Vec<u32>,
}

/// Slot `n`'s state in the draw, None where the draw wrote none.
fn slot(stream: &[(u32, u32)], n: u32) -> Option<Slot> {
    let addr = reg(stream, nte_descriptor_addr(n))?;
    Some(Slot {
        ctrl0: reg(stream, nte_descriptor_samp_ctrl0(n))?,
        ctrl1: reg(stream, nte_descriptor_samp_ctrl1(n))?,
        lod_minmax: reg(stream, nte_descriptor_samp_lod_minmax(n))?,
        lod_bias: reg(stream, nte_descriptor_samp_lod_bias(n))?,
        aniso: reg(stream, nte_descriptor_samp_anisotropy(n))?,
        desc: words_at(u64::from(addr), GC_TEXDESC_BYTES / 4)[16..].to_vec(),
    })
}

fn baselod(s: &Slot) -> u32 {
    s.desc[GC_TEXDESC_BASELOD as usize / 4 - 16]
}

fn config1(s: &Slot) -> u32 {
    s.desc[GC_TEXDESC_CONFIG1 as usize / 4 - 16]
}

const SIZE: u32 = 8;
const LEVELS: u32 = 4;

/// Bind a fresh `SIZE`x`SIZE` texture of `class` with `LEVELS` levels to the
/// active unit: NEAREST filters, repeat wrap.
fn upload(class: &Class) {
    let e = upload_raw(class);
    assert_eq!(e, GL_NO_ERROR, "RGBA32 upload: 0x{e:x}");
}

/// [`upload`] without the check: the GL error of the `glTexImage2D` calls.
fn upload_raw(class: &Class) -> GLenum {
    let words = vec![0u32; (SIZE * SIZE * 4) as usize];
    // SAFETY: plain GL calls; each level's image is at most `SIZE*SIZE` 16-byte texels.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        for l in 0..LEVELS {
            let n = (SIZE >> l) as GLsizei;
            glTexImage2D(GL_TEXTURE_2D, l as GLint, class.2 as GLint, n, n, 0, class.5, class.6, words.as_ptr().cast());
        }
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT as GLint);
    }
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// A `SIZE`x`SIZE` RGBA8 texture on the active unit, NEAREST, repeat.
fn upload_rgba8() {
    let bytes = vec![0x80u8; (SIZE * SIZE * 4) as usize];
    // SAFETY: plain GL calls; the image holds `SIZE*SIZE` 4-byte texels.
    unsafe {
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8 as GLint,
            SIZE as GLsizei,
            SIZE as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            bytes.as_ptr().cast(),
        );
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    no_error("RGBA8 upload");
}

fn tex_i(pname: GLenum, v: u32) {
    // SAFETY: a plain GL call on the bound 2D texture.
    unsafe { glTexParameteri(GL_TEXTURE_2D, pname, v as GLint) };
    no_error("glTexParameteri");
}

fn tex_f(pname: GLenum, v: f32) {
    // SAFETY: a plain GL call on the bound 2D texture.
    unsafe { glTexParameterf(GL_TEXTURE_2D, pname, v) };
    no_error("glTexParameterf");
}

fn samp_i(s: GLuint, pname: GLenum, v: u32) {
    // SAFETY: a plain GL call on a generated sampler.
    unsafe { glSamplerParameteri(s, pname, v as GLint) };
    no_error("glSamplerParameteri");
}

/// The GC value of a GL wrap mode.
fn gc_wrap(w: u32) -> u32 {
    match w {
        GL_CLAMP_TO_EDGE => GC_TEX_WRAP_CLAMP_TO_EDGE,
        GL_MIRRORED_REPEAT => GC_TEX_WRAP_MIRROR_REPEAT,
        _ => GC_TEX_WRAP_REPEAT,
    }
}

fn field(v: u32, shift: u32) -> u32 {
    (v >> shift) & 3
}

/// Is the class closed? If so, say so and check the closed state: the GL
/// core refuses the texture by name (RGBA32UI) or stores it as halves
/// (RGBA32F, texture.rs `sized_format`), so no draw writes a plane
/// descriptor or a companion slot.
fn closed(class: &Class) -> bool {
    if open(class.1) {
        return false;
    }
    eprintln!("{}: not in GC_SAMPLED_TEXFMTS; the sampler state is not exercised (gc-w128-flip)", class.0);
    on_gc7000(|| {
        if upload_raw(class) != GL_NO_ERROR {
            return;
        }
        let st = draw(program(&[(class.4, "s")]));
        let a = slot(&st, 0).unwrap_or_else(|| panic!("{}: nothing in slot 0", class.0));
        assert_ne!(config1(&a) & 0x3f, rnndb(class.3), "{}: no plane descriptor while closed", class.0);
        assert!(slot(&st, 1).is_none(), "{}: no companion slot while closed", class.0);
    });
    true
}

/// Both slots of unit 0's binding, or None (said on stderr) where the GL
/// core did not store the class as planes (src/gl/texture.rs `sized_format`
/// holds RGBA32F as halves whatever the mask says).
fn planes(class: &Class, stream: &[(u32, u32)]) -> Option<(Slot, Slot)> {
    let a = slot(stream, 0).unwrap_or_else(|| panic!("{}: nothing in slot 0", class.0));
    if config1(&a) & 0x3f != rnndb(class.3) {
        eprintln!("{}: the GL core does not store the class as planes (texture.rs sized_format); not checked", class.0);
        return None;
    }
    let b = slot(stream, 1).unwrap_or_else(|| panic!("{}: no companion slot 1", class.0));
    Some((a, b))
}

/// The two slots carry one sampler state and the same descriptor words but
/// for the level addresses (the first sixteen, not in `Slot`).
fn same(what: &str, a: &Slot, b: &Slot) {
    assert_eq!(a, b, "{what}: the companion slot's sampler and descriptor words equal the primary's");
}

/* ---- (a) texture parameters ----------------------------------------------- */

/// Wrap S and T, each of the three modes, and the NEAREST filter pairs
/// (min NEAREST or NEAREST_MIPMAP_NEAREST, mag NEAREST) land in both slots
/// with the values set.
#[test]
fn wrap_and_filter_of_the_texture_reach_both_planes() {
    for class in &WIDE {
        if closed(class) {
            continue;
        }
        on_gc7000(|| {
            let p = program(&[(class.4, "s")]);
            upload(class);
            let modes = [GL_REPEAT, GL_MIRRORED_REPEAT, GL_CLAMP_TO_EDGE];
            for (i, ws) in modes.into_iter().enumerate() {
                let wt = modes[(i + 1) % 3];
                for min in [GL_NEAREST, GL_NEAREST_MIPMAP_NEAREST] {
                    tex_i(GL_TEXTURE_WRAP_S, ws);
                    tex_i(GL_TEXTURE_WRAP_T, wt);
                    tex_i(GL_TEXTURE_MIN_FILTER, min);
                    let what = format!("{} wrap {ws:#x}/{wt:#x} min {min:#x}", class.0);
                    let Some((a, b)) = planes(class, &draw(p)) else { return };
                    same(&what, &a, &b);
                    assert_eq!(a.ctrl0 & 7, gc_wrap(ws), "{what}: U wrap");
                    assert_eq!((a.ctrl0 >> 3) & 7, gc_wrap(wt), "{what}: V wrap");
                    assert_eq!(field(a.ctrl0, 9), GC_TEX_FILTER_NEAREST, "{what}: min");
                    assert_eq!(field(a.ctrl0, 13), GC_TEX_FILTER_NEAREST, "{what}: mag");
                    let mip = if min == GL_NEAREST { GC_TEX_FILTER_NONE } else { GC_TEX_FILTER_NEAREST };
                    assert_eq!(field(a.ctrl0, 11), mip, "{what}: mip");
                }
            }
        });
    }
}

/// GL_TEXTURE_BASE_LEVEL / MAX_LEVEL and GL_TEXTURE_MIN_LOD / MAX_LOD reach
/// both slots alike (whatever the descriptor makes of them: BASELOD is in the
/// descriptor words compared, the LOD range in the sampler's); and max level
/// shortens the chain the descriptor names, in both.
#[test]
fn levels_and_lod_of_the_texture_reach_both_planes() {
    for class in &WIDE {
        if closed(class) {
            continue;
        }
        on_gc7000(|| {
            let p = program(&[(class.4, "s")]);
            upload(class);
            tex_i(GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
            for (base, max) in [(0, 1000), (0, 2), (1, 2), (1, 1)] {
                for (lo, hi) in [(-1000.0, 1000.0), (0.0, 1.0), (1.0, 2.0)] {
                    tex_i(GL_TEXTURE_BASE_LEVEL, base);
                    tex_i(GL_TEXTURE_MAX_LEVEL, max);
                    tex_f(GL_TEXTURE_MIN_LOD, lo);
                    tex_f(GL_TEXTURE_MAX_LOD, hi);
                    let what = format!("{} base {base} max {max} lod {lo}..{hi}", class.0);
                    let Some((a, b)) = planes(class, &draw(p)) else { return };
                    same(&what, &a, &b);
                }
            }
            tex_i(GL_TEXTURE_BASE_LEVEL, 0);
            tex_f(GL_TEXTURE_MIN_LOD, -1000.0);
            tex_f(GL_TEXTURE_MAX_LOD, 1000.0);
            tex_i(GL_TEXTURE_MAX_LEVEL, 0);
            let Some((a, b)) = planes(class, &draw(p)) else { return };
            same(class.0, &a, &b);
            assert_eq!((baselod(&a) >> 8) & 0xf, 0, "{}: max level 0 is one level", class.0);
            tex_i(GL_TEXTURE_MAX_LEVEL, 1000);
            let (a, b) = planes(class, &draw(p)).unwrap();
            same(class.0, &a, &b);
            assert_eq!((baselod(&a) >> 8) & 0xf, LEVELS - 1, "{}: the whole chain", class.0);
        });
    }
}

/* ---- (b) sampler objects --------------------------------------------------- */

/// A sampler object bound to unit 0 overrides the texture's wrap and filters
/// in both slots; unbinding it (`glBindSampler(0, 0)`) returns to the
/// texture's.
#[test]
fn a_sampler_object_overrides_both_planes_and_unbinding_restores() {
    for class in &WIDE {
        if closed(class) {
            continue;
        }
        on_gc7000(|| {
            let p = program(&[(class.4, "s")]);
            upload(class);
            /* the texture: repeat/repeat, NEAREST, no mip filter */
            let Some((own0, own1)) = planes(class, &draw(p)) else { return };
            same(&format!("{} own", class.0), &own0, &own1);
            assert_eq!(own0.ctrl0 & 7, GC_TEX_WRAP_REPEAT);
            assert_eq!(field(own0.ctrl0, 11), GC_TEX_FILTER_NONE);

            let mut s: GLuint = 0;
            // SAFETY: plain GL calls.
            unsafe {
                glGenSamplers(1, &mut s);
                glBindSampler(0, s);
            }
            samp_i(s, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            samp_i(s, GL_TEXTURE_WRAP_T, GL_MIRRORED_REPEAT);
            samp_i(s, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
            samp_i(s, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            let (a, b) = planes(class, &draw(p)).unwrap();
            let what = format!("{} sampler bound", class.0);
            same(&what, &a, &b);
            assert_eq!(a.ctrl0 & 7, GC_TEX_WRAP_CLAMP_TO_EDGE, "{what}: U wrap from the sampler");
            assert_eq!((a.ctrl0 >> 3) & 7, GC_TEX_WRAP_MIRROR_REPEAT, "{what}: V wrap from the sampler");
            assert_eq!(field(a.ctrl0, 11), GC_TEX_FILTER_NEAREST, "{what}: mip filter from the sampler");
            assert_eq!(field(a.ctrl0, 9), GC_TEX_FILTER_NEAREST, "{what}: min");
            assert_eq!(field(a.ctrl0, 13), GC_TEX_FILTER_NEAREST, "{what}: mag");

            /* the texture's own parameters do not show through a bound sampler */
            tex_i(GL_TEXTURE_WRAP_S, GL_MIRRORED_REPEAT);
            tex_i(GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            let (a, b) = planes(class, &draw(p)).unwrap();
            same(&format!("{what}, texture changed"), &a, &b);
            assert_eq!(a.ctrl0 & 7, GC_TEX_WRAP_CLAMP_TO_EDGE, "{what}: still the sampler's U wrap");
            assert_eq!((a.ctrl0 >> 3) & 7, GC_TEX_WRAP_MIRROR_REPEAT, "{what}: still the sampler's V wrap");

            // SAFETY: plain GL call.
            unsafe { glBindSampler(0, 0) };
            let (a, b) = planes(class, &draw(p)).unwrap();
            let what = format!("{} sampler unbound", class.0);
            same(&what, &a, &b);
            assert_eq!(a.ctrl0 & 7, GC_TEX_WRAP_MIRROR_REPEAT, "{what}: U wrap from the texture");
            assert_eq!((a.ctrl0 >> 3) & 7, GC_TEX_WRAP_CLAMP_TO_EDGE, "{what}: V wrap from the texture");
            assert_eq!(field(a.ctrl0, 11), GC_TEX_FILTER_NONE, "{what}: mip filter from the texture");
            // SAFETY: plain GL call.
            unsafe { glDeleteSamplers(1, &s) };
        });
    }
}

/* ---- (c) one sampler object on an RGBA32 unit and an RGBA8 unit -------------- */

/// A sampler object bound to unit 0 (the 128-bit texture) and unit 1 (an
/// RGBA8 texture): the fragment stage has two units, so the RGBA32 binding
/// takes slot 0 and its companion slot 2, the RGBA8 binding slot 1 and
/// nothing after it (slot 3 is never written). Slots 0, 1 and 2 carry the
/// sampler's wrap and filters; the RGBA8 descriptor is no plane.
#[test]
fn a_shared_sampler_object_gives_the_rgba8_unit_no_companion_slot() {
    for class in &WIDE {
        if closed(class) {
            continue;
        }
        on_gc7000(|| {
            let p = program(&[(class.4, "w"), ("sampler2D", "n")]);
            upload(class);
            // SAFETY: plain GL calls.
            unsafe { glActiveTexture(GL_TEXTURE1) };
            upload_rgba8();
            let mut s: GLuint = 0;
            // SAFETY: plain GL calls.
            unsafe {
                glGenSamplers(1, &mut s);
                glBindSampler(0, s);
                glBindSampler(1, s);
            }
            samp_i(s, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            samp_i(s, GL_TEXTURE_WRAP_T, GL_MIRRORED_REPEAT);
            samp_i(s, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            samp_i(s, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            let st = draw(p);
            let what = class.0;
            let wide0 = slot(&st, 0).unwrap_or_else(|| panic!("{what}: nothing in slot 0"));
            if config1(&wide0) & 0x3f != rnndb(class.3) {
                eprintln!(
                    "{what}: the GL core does not store the class as planes (texture.rs sized_format); not checked"
                );
                return;
            }
            let eight = slot(&st, 1).unwrap_or_else(|| panic!("{what}: nothing in slot 1"));
            let wide1 = slot(&st, 2).unwrap_or_else(|| panic!("{what}: no companion slot 2"));
            same(what, &wide0, &wide1);
            assert_ne!(config1(&eight) & 0x3f, rnndb(class.3), "{what}: the RGBA8 unit is no plane");
            assert!(slot(&st, 3).is_none(), "{what}: the RGBA8 unit has no companion slot");
            assert!(reg(&st, nte_descriptor_addr(3)).is_none(), "{what}: slot 3's address is never written");
            for (n, sl) in [("plane 0", &wide0), ("RGBA8", &eight), ("plane 1", &wide1)] {
                assert_eq!(sl.ctrl0 & 7, GC_TEX_WRAP_CLAMP_TO_EDGE, "{what}: {n}: U wrap");
                assert_eq!((sl.ctrl0 >> 3) & 7, GC_TEX_WRAP_MIRROR_REPEAT, "{what}: {n}: V wrap");
                assert_eq!(field(sl.ctrl0, 9), GC_TEX_FILTER_NEAREST, "{what}: {n}: min");
                assert_eq!(field(sl.ctrl0, 13), GC_TEX_FILTER_NEAREST, "{what}: {n}: mag");
            }
            // SAFETY: plain GL calls.
            unsafe { glDeleteSamplers(1, &s) };
        });
    }
}
