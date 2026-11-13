// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): GL_TEXTURE_SWIZZLE_* and GL_TEXTURE_WRAP_R reach the
//! texture descriptor and the sampler (docs/notes/gc-texture-swizzle-wrap.md).
//!
//! Through the GL API on the stand-in OSAL, one quad per case, and the kept
//! stream read back:
//!
//! - the descriptor's CONFIG1 swizzle fields (bits 8-10, 12-14, 16-18, 20-22)
//!   for the identity, a permutation, ZERO/ONE and a GL_RGB texture whose
//!   alpha is the format's ONE;
//! - the same fields for each format of dEQP-GLES3 texture.swizzle the
//!   GC7000UL samples (alpha, luminance, luminance_alpha, red, rg, rgb,
//!   rgba; the case names in tools/cts/gles3-mustpass.txt) under each of the
//!   group's swizzles, and its single_channel group's twenty on RGBA8, all
//!   expected by GL ES 3.0 section 3.8.14 over the format's channels in
//!   docs/es3/gc-formats.md section 2;
//! - SAMP_CTRL0's WWRAP (bits 6-8) for a 3D texture from GL_TEXTURE_WRAP_R, and
//!   unchanged (= UWRAP) for a 2D one.
//!
//! What it cannot see: the GPU (the host renders black).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_tex_swizzle_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
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
/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

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

/* ---- decoding a submitted stream ------------------------------------------------------ */

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

/* ---- a fresh context, back end and OSAL per case -------------------------------------- */

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
        /* leaked, not freed: addresses keep counting up (gc_npot_repeat.rs) */
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

/* ---- GL plumbing ------------------------------------------------------------------------ */

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
const FS_2D: &str = concat!(
    "#version 300 es\nprecision mediump float;\nuniform sampler2D s;\nin vec2 v;\nout vec4 o;\n",
    "void main() { o = texture(s, v); }\n"
);
const FS_3D: &str = concat!(
    "#version 300 es\nprecision mediump float;\nuniform mediump sampler3D s;\nin vec2 v;\nout vec4 o;\n",
    "void main() { o = texture(s, vec3(v, 0.5)); }\n"
);

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// What one draw programmed for sampler slot 0: SAMP_CTRL0 and the
/// descriptor's CONFIG1 word.
struct Sampled {
    ctrl0: u32,
    config1: u32,
}

impl Sampled {
    /// CONFIG1's R, G, B, A swizzle fields (SWIZZLE_R..A: bits 8-10, 12-14,
    /// 16-18, 20-22).
    fn swizzle(&self) -> [u32; 4] {
        [8, 12, 16, 20].map(|s| (self.config1 >> s) & 7)
    }
    /// SAMP_CTRL0 UWRAP, VWRAP, WWRAP (bits 0-2, 3-5, 6-8).
    fn wraps(&self) -> [u32; 3] {
        [0, 3, 6].map(|s| (self.ctrl0 >> s) & 7)
    }
}

/// One quad with the program of `fs` sampling unit 0, drawn and finished.
fn render_quad(fs: &str) -> Sampled {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
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
    let w = writes(&stream);
    let last = |reg: u32| w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v);
    let must = |reg: u32, name: &str| last(reg).unwrap_or_else(|| panic!("{name} (0x{reg:x}) was never written"));
    let desc_gpu = u64::from(must(nte_descriptor_addr(0), "NTE_DESCRIPTOR_ADDR(0)"));
    assert_ne!(desc_gpu, 0, "no descriptor for slot 0");
    let desc: Vec<u32> =
        read_gpu(desc_gpu, GC_TEXDESC_BYTES).as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    Sampled {
        ctrl0: must(nte_descriptor_samp_ctrl0(0), "NTE_DESCRIPTOR_SAMP_CTRL0(0)"),
        config1: desc[GC_TEXDESC_CONFIG1 as usize / 4],
    }
}

fn param(target: GLenum, pname: GLenum, v: GLenum) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameteri(target, pname, v as GLint) };
}

/// A 2x2 texture of `internal`/`format` bound to unit 0, NEAREST, no mips.
fn texture_2d(internal: GLenum, format: GLenum) {
    let texels = [0x80u8; 16];
    // SAFETY: plain GL calls; `texels` holds 2x2 RGBA texels.
    unsafe {
        let mut t: GLuint = 0;
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 2, 2, 0, format, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    }
    param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    no_error("texture_2d");
}

/// A 2x2x`depth` RGBA8 3D texture bound to unit 0, NEAREST, no mips.
fn texture_3d(depth: i32) {
    let texels = vec![0x80u8; 16 * depth as usize];
    // SAFETY: plain GL calls; `texels` holds the 2x2xdepth image.
    unsafe {
        let mut t: GLuint = 0;
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_3D, t);
        glTexImage3D(
            GL_TEXTURE_3D,
            0,
            GL_RGBA8 as GLint,
            2,
            2,
            depth,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            texels.as_ptr().cast(),
        );
    }
    param(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    param(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    no_error("texture_3d");
}

fn swizzle_params(target: GLenum, s: [GLenum; 4]) {
    for (pname, v) in
        [GL_TEXTURE_SWIZZLE_R, GL_TEXTURE_SWIZZLE_G, GL_TEXTURE_SWIZZLE_B, GL_TEXTURE_SWIZZLE_A].into_iter().zip(s)
    {
        param(target, pname, v);
    }
    no_error("swizzle");
}

/// The hardware's TEXTURE_SWIZZLE values (common_3d.xml.h:117-122).
const SW_R: u32 = 0;
const SW_G: u32 = 1;
const SW_B: u32 = 2;
const SW_A: u32 = 3;
const SW_0: u32 = 4;
const SW_1: u32 = 5;

/// The hardware's TEXTURE_WRAPMODE values (common_3d.xml.h:107-110).
const WRAP_REPEAT: u32 = 0;
const WRAP_MIRRORED: u32 = 1;
const WRAP_CLAMP: u32 = 2;

#[test]
fn the_default_swizzle_is_the_identity() {
    on_gc7000(|| {
        texture_2d(GL_RGBA8, GL_RGBA);
        assert_eq!(render_quad(FS_2D).swizzle(), [SW_R, SW_G, SW_B, SW_A]);
    });
}

#[test]
fn a_permutation_reaches_config1() {
    on_gc7000(|| {
        texture_2d(GL_RGBA8, GL_RGBA);
        swizzle_params(GL_TEXTURE_2D, [GL_BLUE, GL_GREEN, GL_RED, GL_ALPHA]);
        assert_eq!(render_quad(FS_2D).swizzle(), [SW_B, SW_G, SW_R, SW_A]);
        swizzle_params(GL_TEXTURE_2D, [GL_ALPHA, GL_BLUE, GL_GREEN, GL_RED]);
        assert_eq!(render_quad(FS_2D).swizzle(), [SW_A, SW_B, SW_G, SW_R]);
    });
}

#[test]
fn zero_and_one_are_constants_in_config1() {
    on_gc7000(|| {
        texture_2d(GL_RGBA8, GL_RGBA);
        swizzle_params(GL_TEXTURE_2D, [GL_ONE, GL_ONE, GL_RED, GL_GREEN]);
        assert_eq!(render_quad(FS_2D).swizzle(), [SW_1, SW_1, SW_R, SW_G]);
        swizzle_params(GL_TEXTURE_2D, [GL_ZERO, GL_RED, GL_ZERO, GL_ONE]);
        assert_eq!(render_quad(FS_2D).swizzle(), [SW_0, SW_R, SW_0, SW_1]);
    });
}

#[test]
fn a_texture_with_no_alpha_composes_its_one() {
    on_gc7000(|| {
        texture_2d(GL_RGB8, GL_RGB);
        assert_eq!(render_quad(FS_2D).swizzle(), [SW_R, SW_G, SW_B, SW_1], "identity");
        /* alpha read by red: the format's own ONE moves with the channel */
        swizzle_params(GL_TEXTURE_2D, [GL_ALPHA, GL_GREEN, GL_BLUE, GL_RED]);
        assert_eq!(render_quad(FS_2D).swizzle(), [SW_1, SW_G, SW_B, SW_R]);
    });
}

/* ---- dEQP-GLES3 texture.swizzle: every format the GC7000UL samples ------------------ */

/// The GL swizzles `texture.swizzle.multi_channel` draws each format with
/// (`all_red` .. `all_one`, `bgra`, `abgr`, `one_one_red_green`: the group's
/// 63 + 20 case names in `tools/cts/gles3-mustpass.txt`'s
/// `functional.texture.swizzle`).
const DEQP_SWIZZLES: [([GLenum; 4], &str); 9] = [
    ([GL_RED, GL_RED, GL_RED, GL_RED], "all_red"),
    ([GL_GREEN, GL_GREEN, GL_GREEN, GL_GREEN], "all_green"),
    ([GL_BLUE, GL_BLUE, GL_BLUE, GL_BLUE], "all_blue"),
    ([GL_ALPHA, GL_ALPHA, GL_ALPHA, GL_ALPHA], "all_alpha"),
    ([GL_ZERO, GL_ZERO, GL_ZERO, GL_ZERO], "all_zero"),
    ([GL_ONE, GL_ONE, GL_ONE, GL_ONE], "all_one"),
    ([GL_BLUE, GL_GREEN, GL_RED, GL_ALPHA], "bgra"),
    ([GL_ALPHA, GL_BLUE, GL_GREEN, GL_RED], "abgr"),
    ([GL_ONE, GL_ONE, GL_RED, GL_GREEN], "one_one_red_green"),
];

/// The format's own channels, with alpha (docs/es3/gc-formats.md section 2:
/// every sampled class but the 128-bit ones writes the four words R G B A.
/// A narrow TE format presents what its texels hold: L8 spreads the
/// luminance over the channels, A8 carries alpha only, A8L8 luminance and
/// alpha — and the descriptor names those channels as the format supplies
/// them).
const HAS_ALPHA: [u32; 4] = [SW_R, SW_G, SW_B, SW_A];
/// The same for a base without alpha (GL_RGB, GL_LUMINANCE, GL_RED, GL_RG):
/// GL samples their alpha as 1, and the format's own ONE says so
/// (`GlesTextureBind::no_alpha`).
const NO_ALPHA: [u32; 4] = [SW_R, SW_G, SW_B, SW_1];

/// What GL ES 3.0 section 3.8.14 asks of TEXTURE_SWIZZLE_* over a format
/// whose channels are `texel`: each output component is the texel component
/// the parameter names, or the constant 0 / 1. The format goes first and the
/// GL swizzle composes on top — Mesa's util_format_compose_swizzles, and
/// etnaviv's get_texture_swiz (docs/es3/gc-formats.md section 1).
fn composed(texel: [u32; 4], gl: [GLenum; 4]) -> [u32; 4] {
    gl.map(|s| match s {
        GL_RED => texel[0],
        GL_GREEN => texel[1],
        GL_BLUE => texel[2],
        GL_ALPHA => texel[3],
        GL_ZERO => SW_0,
        GL_ONE => SW_1,
        _ => panic!("no swizzle 0x{s:x}"),
    })
}

/// A 2x2 texture of `internal`/`format` bound to unit 0, drawn once under the
/// identity (the format's own words) and once under each of the group's nine
/// swizzles, plus a mix of the constants with channels, each draw's CONFIG1
/// swizzle against [`composed`] over `texel`.
fn deqp_swizzle_case(name: &str, internal: GLenum, format: GLenum, texel: [u32; 4]) {
    on_gc7000(|| {
        texture_2d(internal, format);
        assert_eq!(render_quad(FS_2D).swizzle(), texel, "{name}: the identity is the format's own");
        for (gl, swizzle) in DEQP_SWIZZLES {
            swizzle_params(GL_TEXTURE_2D, gl);
            assert_eq!(render_quad(FS_2D).swizzle(), composed(texel, gl), "{name}_{swizzle}");
        }
        let mixed = [GL_ZERO, GL_ALPHA, GL_ZERO, GL_ONE];
        swizzle_params(GL_TEXTURE_2D, mixed);
        assert_eq!(render_quad(FS_2D).swizzle(), composed(texel, mixed), "{name}_zero_alpha_zero_one");
    });
}

#[test]
fn the_rule_composes_as_the_spec_reads() {
    /* the helper itself, by hand: a no-alpha base reads its ONE through alpha */
    assert_eq!(composed(NO_ALPHA, [GL_ALPHA, GL_ZERO, GL_RED, GL_ONE]), [SW_1, SW_0, SW_R, SW_1]);
    assert_eq!(composed(HAS_ALPHA, [GL_ALPHA, GL_BLUE, GL_GREEN, GL_RED]), [SW_A, SW_B, SW_G, SW_R]);
}

#[test]
fn deqp_alpha_a8() {
    deqp_swizzle_case("alpha", GL_ALPHA, GL_ALPHA, HAS_ALPHA);
}

#[test]
fn deqp_luminance_l8() {
    deqp_swizzle_case("luminance", GL_LUMINANCE, GL_LUMINANCE, NO_ALPHA);
}

#[test]
fn deqp_luminance_alpha_la8() {
    deqp_swizzle_case("luminance_alpha", GL_LUMINANCE_ALPHA, GL_LUMINANCE_ALPHA, HAS_ALPHA);
}

#[test]
fn deqp_red_r8() {
    deqp_swizzle_case("red", GL_R8, GL_RED, NO_ALPHA);
}

#[test]
fn deqp_rg_rg8() {
    deqp_swizzle_case("rg", GL_RG8, GL_RG, NO_ALPHA);
}

#[test]
fn deqp_rgb_rgb8() {
    deqp_swizzle_case("rgb", GL_RGB8, GL_RGB, NO_ALPHA);
}

#[test]
fn deqp_rgba_rgba8() {
    deqp_swizzle_case("rgba", GL_RGBA8, GL_RGBA, HAS_ALPHA);
}

/// texture.swizzle.single_channel's twenty cases: one component named, the
/// others at their default, on the RGBA8 the group uploads.
#[test]
fn deqp_single_channel_on_rgba8() {
    on_gc7000(|| {
        texture_2d(GL_RGBA8, GL_RGBA);
        for (k, own) in [(0usize, "r"), (1, "g"), (2, "b"), (3, "a")] {
            for (src, swizzle) in [
                (GL_RED, "red"),
                (GL_GREEN, "green"),
                (GL_BLUE, "blue"),
                (GL_ALPHA, "alpha"),
                (GL_ZERO, "zero"),
                (GL_ONE, "one"),
            ] {
                /* the group names a component other than the target's own */
                if [GL_RED, GL_GREEN, GL_BLUE, GL_ALPHA][k] == src {
                    continue;
                }
                let mut gl = [GL_RED, GL_GREEN, GL_BLUE, GL_ALPHA];
                gl[k] = src;
                swizzle_params(GL_TEXTURE_2D, gl);
                assert_eq!(render_quad(FS_2D).swizzle(), composed(HAS_ALPHA, gl), "single_channel.{own}_{swizzle}");
            }
        }
    });
}

#[test]
fn wrap_r_is_the_third_axis_of_a_3d_texture() {
    on_gc7000(|| {
        texture_3d(2);
        param(GL_TEXTURE_3D, GL_TEXTURE_WRAP_S, GL_REPEAT);
        param(GL_TEXTURE_3D, GL_TEXTURE_WRAP_T, GL_REPEAT);
        for (gl, hw) in [(GL_CLAMP_TO_EDGE, WRAP_CLAMP), (GL_MIRRORED_REPEAT, WRAP_MIRRORED), (GL_REPEAT, WRAP_REPEAT)]
        {
            param(GL_TEXTURE_3D, GL_TEXTURE_WRAP_R, gl);
            assert_eq!(render_quad(FS_3D).wraps(), [WRAP_REPEAT, WRAP_REPEAT, hw], "WRAP_R 0x{gl:x}");
        }
    });
}

#[test]
fn a_2d_texture_keeps_s_on_the_w_axis() {
    on_gc7000(|| {
        texture_2d(GL_RGBA8, GL_RGBA);
        param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_MIRRORED_REPEAT);
        param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE);
        assert_eq!(render_quad(FS_2D).wraps(), [WRAP_MIRRORED, WRAP_REPEAT, WRAP_MIRRORED]);
    });
}
