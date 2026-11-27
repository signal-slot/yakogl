// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A plain 2D texture at GL_TEXTURE_BASE_LEVEL 1 and 2 on the PowerVR is
//! sized and sampled from its base level (ES 3.0.6 3.8.9: level_base is
//! sampled without a mipmap filter, 3.8.10 the chain is walked from it;
//! GLSL ES 3.00 8.8: `textureSize(s, lod)` is level `level_base + lod`'s
//! size). dEQP-GLES3 shaders.texture_functions.texturesize's "lod base"
//! rows (`TextureSizeCase`, es3fShaderTextureFunctionTests.cpp at dEQP
//! 93bca01: glTexStorage2D of `lod + lodBase + 1` levels, NEAREST,
//! GL_TEXTURE_BASE_LEVEL `lodBase`) failed on the AM62P with "Got unexpected
//! texture size" in the 10 sampler2d / isampler2d / usampler2d /
//! sampler2dshadow cases: the draw described every 2D bind from level 0
//! (`pvr::backend::build_tex_desc` passed `TexLod::DEFAULT`, so the
//! bind's `base_level`, which the GL core fills, never reached the words).
//!
//! Through the GL API on the host, a stand-in back end with the PowerVR's
//! layout fields (all zero, as `pvr::backend`'s table) keeps the 2D bind
//! of each draw, which goes through `pvr::backend::texture_descriptor`, the
//! descriptor builder the PowerVR draw binds with. Then:
//!
//! - the bind carries the base level, and the descriptor's linear image
//!   (TEXSTATE_STRIDE_IMAGE_WORD1, which has no baselevel field) is the base
//!   level's: IMAGE_WORD0 width/height, the stride and texaddr moved to it;
//! - `textureSize(s, il)` of the program, run in the byte model
//!   (tests/usc_exec) over that descriptor, is level `base + il`'s size,
//!   at dEQP's rows (100x31 lod 3, 128x64 lod 3, 64x64 lod 1, base 1) and
//!   at base 2;
//! - the sampled value: every texel the descriptor addresses (texaddr +
//!   (y * stride + x) * 4 for x, y inside its extent, NEAREST) is the base
//!   level's texel as glTexSubImage2D wrote it, read from the texture's
//!   own memory (the host's GPU address is its CPU pointer). The byte
//!   model's `smp` answers no memory, so the TPU's NEAREST fetch is
//!   modelled here from the words, as texstate.xml defines them;
//! - with NEAREST_MIPMAP_NEAREST the levels counted from the base
//!   (num_mip_levels), and base level 0 keeps level 0's words.
//!
//! Not here: cube, 2D array and 3D textures (their base level is the GL
//! core's to apply, docs/es3/shader-failure-map.md section 7) and
//! GL_TEXTURE_MAX_LEVEL / MIN_LOD / MAX_LOD (tests/es3_pvr_base_lod.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_texsize_base_level
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXTYPE_2D};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::backend::texture_descriptor;
use yakogl::shaderc::online::powervr::chain::compile_fragment;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::SHC_UNIFORM_NO_BLOCK;

mod usc_exec;

/* ---- the stand-in back end: it keeps the first texture bind of the last draw */

struct Seen(Option<GlesTextureBind>);
// SAFETY: the bind's `cpu` pointer is only carried, never dereferenced
// through SEEN; the tests are serialised by SERIAL.
unsafe impl Send for Seen {}
static SEEN: Mutex<Seen> = Mutex::new(Seen(None));

unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a live job.
    let d = unsafe { &*d };
    assert!(d.num_textures >= 1, "the program samples one texture");
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).0 = Some(d.textures[0]);
    0
}

/* Every field not named here is zero, as in pvr::backend's own table for
 * the layout fields (mip_level_align, linear_row_align, ...), so a field
 * another change adds needs no edit here (CLAUDE.md, "Two green pull
 * requests, red main"). */
static BACKEND: YakoglBackend = {
    // SAFETY: YakoglBackend is a pointer, Options of fn pointers, bools and
    // integers, for which all-zero is a valid value.
    let mut b: YakoglBackend = unsafe { core::mem::zeroed() };
    b.name = c"pvr-texsize-base-level".as_ptr() as *const c_char;
    b.init = Some(b_init);
    b.draw = Some(b_draw);
    b.flush = Some(b_ok);
    b.finish = Some(b_ok);
    b.heap_base = Some(b_heap_base);
    b.max_instance_divisor = 1;
    b
};

unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: `out` is the core's.
    unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
    0
}
unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

static OSAL: OsalOps = {
    // SAFETY: OsalOps is Options of fn pointers, for which all-zero is None.
    let mut o: OsalOps = unsafe { core::mem::zeroed() };
    o.gpu_alloc = Some(o_gpu_alloc);
    o.gpu_free = Some(o_gpu_free);
    o.cache_flush = Some(o_cache);
    o.cache_invd = Some(o_cache);
    o
};

/// One case at a time: the GL context, the compiler's arena and its
/// switches are process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

fn on_fresh_context(case: impl Fn()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &BACKEND;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL plumbing ------------------------------------------------------------ */

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

const VS: &str = "#version 300 es\nin vec4 p;\nout vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";

/// The program of dEQP's case, fragment stage: `textureSize(s, il)`, and a
/// sample of `s` so the draw is a sampling one too.
fn fs(kind: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {kind} s;\nuniform int il;\n\
         in vec2 v;\nout vec4 o;\nvoid main() {{ ivec2 t = textureSize(s, il); o = vec4(vec2(t), 0.0, 1.0) / 255.0 \
         + 0.0 * vec4(texture(s, v)); }}\n"
    )
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// One quad with `kind`'s program over texture unit 0: the bind the draw
/// handed the back end.
fn draw_bind(kind: &str) -> GlesTextureBind {
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).0 = None;
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs(kind)));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glUniform1i(glGetUniformLocation(p, c"il".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).0.expect("the quad was drawn")
}

/// Texel (x, y) of level `l`: its coordinates and its level, so no two
/// texels of one chain are the same (every side here is below 256).
fn texel(l: u32, x: u32, y: u32) -> [u8; 4] {
    [x as u8, y as u8, 0x40 | l as u8, 0x7f]
}

/// A 2D texture of `levels` levels of `ifmt` (GL_RGBA8 or GL_RGBA8I),
/// `w x h`, every level's texels [`texel`], with `min_filter`, NEAREST
/// magnification and GL_TEXTURE_BASE_LEVEL `base`, bound to unit 0.
fn texture(ifmt: GLenum, w: u32, h: u32, levels: u32, min_filter: GLenum, base: u32) {
    let fmt = if ifmt == GL_RGBA8I { GL_RGBA_INTEGER } else { GL_RGBA };
    let ty = if ifmt == GL_RGBA8I { GL_BYTE } else { GL_UNSIGNED_BYTE };
    let mut name: GLuint = 0;
    // SAFETY: one name; each level's pixels cover its extent.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
        glTexStorage2D(GL_TEXTURE_2D, levels as GLsizei, ifmt, w as GLsizei, h as GLsizei);
        for l in 0..levels {
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            let px: Vec<u8> = (0..lh).flat_map(|y| (0..lw).flat_map(move |x| texel(l, x, y))).collect();
            glTexSubImage2D(
                GL_TEXTURE_2D,
                l as GLint,
                0,
                0,
                lw as GLsizei,
                lh as GLsizei,
                fmt,
                ty,
                px.as_ptr() as *const c_void,
            );
        }
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min_filter as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_BASE_LEVEL, base as GLint);
    }
    no_error("texture");
}

/* ---- the descriptor's fields (texstate.xml) ---------------------------------- */

fn word(w: &[u32], i: usize) -> u64 {
    u64::from(w[i]) | (u64::from(w[i + 1]) << 32)
}

fn bits(v: u64, lo: u32, hi: u32) -> u64 {
    (v >> lo) & ((1u64 << (hi - lo + 1)) - 1)
}

/// The linear image a descriptor states: (width, height, stride in
/// texels, texaddr, num_mip_levels), from IMAGE_WORD0 width [34:47] and
/// height [48:61] (each less one) and STRIDE_IMAGE_WORD1 stride [0:14]
/// (less one), texaddr [16:53] (shift 2) and num_mip_levels [60:63].
fn image(desc: &[u32]) -> (u32, u32, u32, u64, u32) {
    let iw0 = word(desc, 0);
    let iw1 = word(desc, 2);
    assert_eq!(bits(iw0, 0, 2), 4, "TEXTYPE STRIDE: a linear 2D image");
    (
        bits(iw0, 34, 47) as u32 + 1,
        bits(iw0, 48, 61) as u32 + 1,
        bits(iw1, 0, 14) as u32 + 1,
        bits(iw1, 16, 53) << 2,
        bits(iw1, 60, 63) as u32,
    )
}

/// The TPU's NEAREST fetch of texel (x, y) of the image `desc` states, read
/// from the texture memory `bind` names (`bind.base_gpu` .. the end of its
/// allocation): a linear image, 4 bytes a texel.
fn fetch(desc: &[u32], storage: (u64, u64), x: u32, y: u32) -> [u8; 4] {
    let (_, _, stride, addr, _) = image(desc);
    /* texaddr is a 40-bit device address; a host pointer is wider, so its
     * bits above 40 are the allocation's (it does not cross 2^40) */
    let addr = (storage.0 & !((1u64 << 40) - 1)) | addr;
    let at = addr + (u64::from(y) * u64::from(stride) + u64::from(x)) * 4;
    assert!(at >= storage.0 && at + 4 <= storage.1, "texel ({x}, {y}) at 0x{at:x} is outside the texture's memory");
    // SAFETY: the host's GPU address is its CPU pointer (o_gpu_alloc), and
    // the four bytes lie inside the texture's allocation (asserted).
    unsafe { *(at as *const [u8; 4]) }
}

/// The texture memory of the 2D texture bound to unit 0: (start, end).
fn storage() -> (u64, u64) {
    let mut name: GLint = 0;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(GL_TEXTURE_BINDING_2D, &mut name) };
    let c = yakogl::gl::context::current().expect("a current context");
    let slot = yakogl::gl::names::slot_of(c, yakogl::gl::names::Kind::Texture, name as GLuint).expect("a texture");
    let t = c.texture_object(slot).expect("a texture object");
    (t.mem.gpu, t.mem.gpu + t.mem.size as u64)
}

/* ---- textureSize in the byte model ------------------------------------------ */

/// `textureSize(s, il)` of a fragment program on `kind` over descriptor
/// `desc`, run in the byte model: (x, y), each lane two bytes of a pixel.
fn texture_size(kind: &str, desc: &[u32], il: u32) -> [u32; 2] {
    let lane = |c: char| -> u32 {
        let src = format!(
            "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {kind} s;\nuniform int il;\n\
             out vec4 o;\nvoid main() {{ int q = textureSize(s, il).{c}; int hi = q / 256; \
             o = vec4(float(q - hi * 256), float(hi), 0.0, 1.0) / 255.0; }}\n"
        );
        let px = store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(&src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).unwrap_or_else(|e| panic!("front end: {}", e.msg));
            let rec = compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("refused: {e:?}\n{src}"));
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            let mut inp = usc_exec::sentinel_inputs(1.0e30);
            /* the shared file as the driver fills it: the sampler's
             * descriptor, then the uniform block */
            inp.shared[..desc.len()].copy_from_slice(desc);
            let base = desc.len();
            for u in rows.iter().filter(|u| u.dword_off != SHC_UNIFORM_NO_BLOCK) {
                let name: String = u.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
                let value = if name == "il" { il } else { 0 };
                for k in 0..u.dwords as usize {
                    inp.shared[base + u.dword_off as usize + k] = value;
                }
            }
            let out = usc_exec::run_binary(&image, &inp, true).unwrap_or_else(|e| panic!("the byte model: {e}\n{src}"));
            *out.get(&0).expect("a pixel")
        })
        .unwrap();
        (px & 0xff) + ((px >> 8) & 0xff) * 256
    };
    [lane('x'), lane('y')]
}

/* ---- the checks ---------------------------------------------------------------- */

/// The texture `w x h` of `levels` levels at base level `base`, NEAREST
/// (or `min_filter`): the bind carries the base level, the descriptor is the
/// base level's image and samples its texels, and `textureSize(s, lod)` is
/// level `base + lod`'s size for every `lod` the storage holds past it.
fn check(ifmt: GLenum, kind: &str, (w, h): (u32, u32), levels: u32, min_filter: GLenum, base: u32) {
    let what = format!("{kind} {w}x{h}, {levels} levels, base {base}, min 0x{min_filter:x}");
    texture(ifmt, w, h, levels, min_filter, base);
    let bind = draw_bind(kind);
    assert_eq!(bind.type_, GLES_TEXTYPE_2D, "{what}: a plain 2D bind");
    assert_eq!(u32::from(bind.base_level), base, "{what}: the bind's base level");
    let desc = texture_descriptor(&bind);
    let (bw, bh) = ((w >> base).max(1), (h >> base).max(1));
    let (iw, ih, _, _, nlev) = image(&desc);
    assert_eq!((iw, ih), (bw, bh), "{what}: IMAGE_WORD0 is the base level's extent");
    let mip = min_filter != GL_NEAREST && min_filter != GL_LINEAR;
    let want_levels = if mip { levels - base } else { 1 };
    assert_eq!(nlev, want_levels, "{what}: the levels counted from the base");
    /* the sampled value: every texel of the base level, NEAREST */
    let mem = storage();
    for y in 0..bh {
        for x in 0..bw {
            assert_eq!(fetch(&desc, mem, x, y), texel(base, x, y), "{what}: texel ({x}, {y}) is the base level's");
        }
    }
    /* textureSize from the base level on */
    for lod in 0..levels - base {
        let want = [(w >> (base + lod)).max(1), (h >> (base + lod)).max(1)];
        assert_eq!(texture_size(kind, &desc, lod), want, "{what}: textureSize(s, {lod})");
    }
}

/// dEQP's three "lod base" rows of TextureSizeCase (lodBase 1; `lod +
/// lodBase + 1` levels, NEAREST): 100x31 at lod 3 -> 6x1, 128x64 at lod 3
/// -> 8x4, 64x64 at lod 1 -> 16x16 (those are among the lods `check`
/// walks), for sampler2D and isampler2D.
#[test]
fn deqps_lod_base_rows_size_and_sample_from_level_1() {
    on_fresh_context(|| {
        for (ifmt, kind) in [(GL_RGBA8, "sampler2D"), (GL_RGBA8I, "isampler2D")] {
            for ((w, h), lod) in [((100, 31), 3), ((128, 64), 3), ((64, 64), 1)] {
                check(ifmt, kind, (w, h), lod + 1 + 1, GL_NEAREST, 1);
            }
        }
    });
}

/// Base level 2 of a whole chain, power-of-two and not.
#[test]
fn base_level_2_sizes_and_samples_from_level_2() {
    on_fresh_context(|| {
        for (w, h) in [(128u32, 64u32), (100, 31), (64, 64)] {
            let levels = 32 - w.max(h).leading_zeros();
            check(GL_RGBA8, "sampler2D", (w, h), levels, GL_NEAREST, 2);
        }
    });
}

/// With a mipmap filter the chain is walked from the base level: its
/// levels counted from there, base 1 and 2.
#[test]
fn a_mipmap_filter_walks_the_chain_from_the_base_level() {
    on_fresh_context(|| {
        for base in [1, 2] {
            check(GL_RGBA8, "sampler2D", (128, 64), 8, GL_NEAREST_MIPMAP_NEAREST, base);
            check(GL_RGBA8, "sampler2D", (100, 31), 7, GL_NEAREST_MIPMAP_NEAREST, base);
        }
    });
}

/// Base level 0 is level 0's image, as before the fix.
#[test]
fn base_level_0_keeps_level_0() {
    on_fresh_context(|| {
        check(GL_RGBA8, "sampler2D", (100, 31), 5, GL_NEAREST, 0);
        check(GL_RGBA8, "sampler2D", (128, 64), 8, GL_NEAREST_MIPMAP_NEAREST, 0);
    });
}
