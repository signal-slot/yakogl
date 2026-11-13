// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the float colour targets the vivante host back end
//! draws today, RGBA16F, R16F and R32F, and RG16F / RGB16F as they map (item
//! test-gc-half of plan #360). A baseline for the float-target design
//! (docs/notes/es3-float-targets.md) and for a later GC arm: it pins what is
//! emitted now, it does not claim the state is right where an ignored test
//! says otherwise.
//!
//! What it pins, per format, through the GL API on the stand-in OSAL:
//!
//! - the surface (`GLES_SURF_*`) a renderbuffer of the format draws as, its
//!   PE colour format (`pe_color_format`, rnndb `PE_FORMAT_*`), the bytes per
//!   pixel (RGBA16F 8, R16F 2, R32F 4) and the pixel engine's output mode
//!   (R32F `UIF32`, the halves `NORMAL`);
//! - the whole PE_COLOR_FORMAT word of a draw and of a glClearBufferfv, and
//!   PE_ALPHA_CONFIG with GL_BLEND on: the three float surfaces are blended
//!   like the unorm ones, the 32-bit R32F included (the blend is not turned
//!   off for a 32-bit float target; ES 3.0 has no such colour target without
//!   GL_EXT_color_buffer_float, and a blend of it needs EXT_float_blend);
//! - read-back: no resolve engine is involved for a float target (the clear
//!   is the quad, no RS_CONFIG / RS_KICKER; `rs_clear::rs_format` has no
//!   8-byte format and `rs_clear_of` takes A8R8G8B8 / X8R8G8B8 only), a
//!   glReadPixels decodes the storage on the CPU (src/gl/framebuffer.rs), so
//!   a float read-back has no RS format, and the colour reads back through
//!   GL_RGBA / GL_FLOAT exactly (values representable as halves).
//!
//! The GPU is modelled as in tests/es3_int32_rt_gc.rs: the stand-in OSAL's
//! submit stores the colour into the surface the stream names. It checks the
//! layout, the copy-back and the read path, not what the PE stores: that is
//! the board's (#316).
//!
//! RGBA16F is 64 bits a pixel and renders into the 4x4-tiled shadow
//! (`wide_target.rs`); R16F and R32F are linear.
//!
//! Where the state looks wrong against Mesa etnaviv (etnaviv_format.c:151,
//! `VT(R16G16_FLOAT, ..., EXT_G16R16F | EXT_FORMAT, G16R16F)`), the test pins
//! today's value and an `#[ignore]`d test names the reason: RG16F is drawn as
//! the 8-byte RGBA16F where etnaviv draws it as G16R16F, 4 bytes.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_half_float_rt_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::pe_color_format;
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;
use yakogl::vivante::shader::ps_color_output_mode;

/// The online compiler's store is one static for the process: every test
/// takes this one lock (as tests/es3_int_rt_gc.rs).
static STORE: Mutex<()> = Mutex::new(());

/// The value of `#define <name>` in driver/src/vivante/hw/state_3d.xml.h.
fn rnndb(name: &str) -> u32 {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/state_3d.xml.h");
    let text = std::fs::read_to_string(path).expect("the vendored rnndb header");
    let line = text
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(name) && l.starts_with("#define"))
        .unwrap_or_else(|| panic!("{name} is in state_3d.xml.h"));
    let v = line.split_whitespace().nth(2).expect("a value");
    u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
}
/* ---- the stand-in OSAL: host memory, every stream, a model of the PE ------ */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    /// While set, every kick that names a colour surface gets `(w, h, bytes
    /// a pixel, pixel)` stored into it: linear at the stream's stride for 4
    /// bytes a pixel, 4x4-tiled for 8 (the shadow).
    model: Option<(u32, u32, u32, Vec<u8>)>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

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
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor; it is cleared (the OSAL contract).
    unsafe {
        let m = *mem;
        with_platform(|p| p.allocs.remove(&m.gpu));
        *mem = OsalGpuMem::ZERO;
    }
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// Byte offset of pixel (x, y) in a 4x4-tiled surface `tw` pixels wide at 8
/// bytes a pixel: written here from the hardware's layout, not taken from
/// `wide_target.rs`, so that a wrong copy there shows.
fn tiled_byte(x: u32, y: u32, tw: u32) -> usize {
    (((y / 4) * (tw / 4) + x / 4) * 16 + (y % 4) * 4 + x % 4) as usize * 8
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

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else { return -22 };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        let stream = words.to_vec();
        if let Some((w, h, bpp, pixel)) = p.model.clone() {
            let ws = writes(&stream);
            let addr = ws.iter().rev().find(|(r, _)| *r == pe_pipe_color_addr(0)).map(|(_, v)| *v as u64);
            let stride = ws.iter().rev().find(|(r, _)| *r == rnndb("VIVS_PE_COLOR_STRIDE")).map(|(_, v)| *v);
            if let (Some(addr), Some(stride)) = (addr, stride) {
                if let Some((base, mem)) = p.allocs.range_mut(..=addr).next_back().map(|(g, m)| (*g, m)) {
                    let off = (addr - base) as usize;
                    // SAFETY: the surface holds the target's pixels from `off` on.
                    let bytes = unsafe { core::slice::from_raw_parts_mut(mem.as_mut_ptr() as *mut u8, mem.len() * 8) };
                    for y in 0..h {
                        for x in 0..w {
                            let at = off
                                + if bpp == 8 { tiled_byte(x, y, stride / 8) } else { (y * stride + x * bpp) as usize };
                            bytes[at..at + bpp as usize].copy_from_slice(&pixel);
                        }
                    }
                }
            }
        }
        p.kicks.push(stream);
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

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| p.model = None);
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
    with_platform(|p| {
        /* leaked, not freed, as tests/gc_npot_repeat.rs: an address a static
         * still names must stay memory */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL helpers ------------------------------------------------------------ */

fn err() -> GLenum {
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

fn program(fs: &str) -> GLuint {
    const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

fn renderbuffer(internal: GLenum, w: u32, h: u32) -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, internal, w as GLsizei, h as GLsizei);
    }
    assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(0x{internal:x}, {w}x{h})");
    rb
}

/// A framebuffer with `rb` at colour attachment 0, its one draw buffer; the
/// status it reports.
fn framebuffer(rb: GLuint) -> GLenum {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    let status = unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        let buf = GL_COLOR_ATTACHMENT0;
        glDrawBuffers(1, &buf);
        glCheckFramebufferStatus(GL_FRAMEBUFFER)
    };
    assert_eq!(err(), GL_NO_ERROR, "framebuffer");
    status
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

fn last(w: &[(u32, u32)], reg: u32) -> Option<u32> {
    w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v)
}

/* ---- the formats ------------------------------------------------------------ */

/// A float colour target: the GL format, the surface it draws as, the PE
/// format (rnndb name), the bytes a pixel, the channels GL's format has and
/// the output mode of the fragment shader.
struct Float {
    name: &'static str,
    internal: GLenum,
    surf: u8,
    pe: &'static str,
    bytes: u32,
    channels: usize,
    mode: u32,
}

const RGBA16F: Float = Float {
    name: "RGBA16F",
    internal: GL_RGBA16F,
    surf: GLES_SURF_RGBA16F,
    pe: "PE_FORMAT_A16B16G16R16F",
    bytes: 8,
    channels: 4,
    mode: GC_PS_OUTPUT_MODE_NORMAL,
};
const R16F: Float = Float {
    name: "R16F",
    internal: GL_R16F,
    surf: GLES_SURF_R16F,
    pe: "PE_FORMAT_R16F",
    bytes: 2,
    channels: 1,
    mode: GC_PS_OUTPUT_MODE_NORMAL,
};
const R32F: Float = Float {
    name: "R32F",
    internal: GL_R32F,
    surf: GLES_SURF_R32F,
    pe: "PE_FORMAT_R32F",
    bytes: 4,
    channels: 1,
    mode: GC_PS_OUTPUT_MODE_UIF32,
};
/* RGB16F and RG16F share the RGBA16F surface (`renderbuffer_surf_format`):
 * four half channels, the ones the format does not have ignored by GL */
const RGB16F: Float = Float {
    name: "RGB16F",
    internal: GL_RGB16F,
    surf: GLES_SURF_RGBA16F,
    pe: "PE_FORMAT_A16B16G16R16F",
    bytes: 8,
    channels: 3,
    mode: GC_PS_OUTPUT_MODE_NORMAL,
};
const RG16F: Float = Float {
    name: "RG16F",
    internal: GL_RG16F,
    surf: GLES_SURF_RGBA16F,
    pe: "PE_FORMAT_A16B16G16R16F",
    bytes: 8,
    channels: 2,
    mode: GC_PS_OUTPUT_MODE_NORMAL,
};

/// The three the back end draws as surfaces of their own.
const OWN: [&Float; 3] = [&RGBA16F, &R16F, &R32F];
/// ...and the two that map onto RGBA16F.
const MAPPED: [&Float; 2] = [&RGB16F, &RG16F];

/// The constant colour of the draws: every value is exact as a half.
const COLOUR: [f32; 4] = [0.5, 0.25, 0.125, 0.75];

fn half(v: f32) -> u16 {
    let bits = v.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    assert!(exp > 0 && exp < 31 && bits & 0x1fff == 0, "{v} is not an exact half");
    (((exp as u32) << 10) | ((bits >> 13) & 0x3ff)) as u16
}

impl Float {
    fn fs(&self) -> String {
        let [r, g, b, a] = COLOUR;
        format!(
            "#version 300 es\nlayout(location = 0) out highp vec4 o;\nvoid main() {{ o = vec4({r:?}, {g:?}, {b:?}, {a:?}); }}\n"
        )
    }

    /// The pixel the modelled PE stores for `COLOUR`.
    fn pixel(&self) -> Vec<u8> {
        match self.surf {
            GLES_SURF_R32F => COLOUR[0].to_le_bytes().to_vec(),
            GLES_SURF_R16F => half(COLOUR[0]).to_le_bytes().to_vec(),
            _ => COLOUR.iter().flat_map(|v| half(*v).to_le_bytes()).collect(),
        }
    }

    /// The PE_COLOR_FORMAT word of a draw (`is_clear` false) or a clear:
    /// FORMAT_EXT and FORMAT_MASK, COMPONENTS (a draw leaves A out for a
    /// format without it, a clear writes it), and OVERWRITE where all four
    /// are written and nothing blends.
    fn color_format_word(&self, is_clear: bool, blend: bool) -> u32 {
        let field = |mask: &str, v: u32| (v << rnndb(mask).trailing_zeros()) & rnndb(mask);
        let comps = if self.channels == 4 || is_clear { 0xf } else { 0x7 };
        field("VIVS_PE_COLOR_FORMAT_FORMAT_EXT__MASK", rnndb(self.pe))
            | rnndb("VIVS_PE_COLOR_FORMAT_FORMAT_MASK")
            | field("VIVS_PE_COLOR_FORMAT_COMPONENTS__MASK", comps)
            | if comps == 0xf && !blend { rnndb("VIVS_PE_COLOR_FORMAT_OVERWRITE") } else { 0 }
    }
}

/// glClearBufferfv of draw buffer 0, finished; the error and the writes of
/// the streams submitted since.
fn clear(c: [f32; 4]) -> (GLenum, Vec<(u32, u32)>) {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: four values; plain GL calls.
    let e = unsafe {
        glClearBufferfv(GL_COLOR, 0, c.as_ptr());
        let e = glGetError();
        glFinish();
        e
    };
    (e, with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect()))
}

/// The whole `w` x `h` target read back as GL_RGBA / GL_FLOAT.
fn read(f: &Float, w: u32, h: u32) -> Vec<f32> {
    let mut buf = vec![-1.0f32; (w * h) as usize * 4];
    // SAFETY: the buffer holds w * h pixels of four floats.
    unsafe { glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_FLOAT, buf.as_mut_ptr() as *mut _) };
    assert_eq!(err(), GL_NO_ERROR, "{}: glReadPixels", f.name);
    buf
}

/// A complete one-target framebuffer of `f`, `w` x `h`.
fn target(f: &Float, w: u32, h: u32) {
    let rb = renderbuffer(f.internal, w, h);
    assert_eq!(framebuffer(rb), GL_FRAMEBUFFER_COMPLETE, "{}", f.name);
}

/// Tile-aligned, and a size that pads both the 4x4 tile and the shadow's row.
const SIZES: [(u32, u32); 2] = [(16, 16), (17, 13)];

/// A draw of `COLOUR` over the `w` x `h` viewport, optionally blended with
/// SRC_ALPHA / ONE_MINUS_SRC_ALPHA; the error and the writes since.
fn draw_colour(f: &Float, w: u32, h: u32, blend: bool) -> (GLenum, Vec<(u32, u32)>) {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    let e = unsafe {
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glUseProgram(program(&f.fs()));
        if blend {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        }
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        let e = glGetError();
        glFinish();
        e
    };
    (e, with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect()))
}

/// The words of a one-target draw or clear into `f`: the colour format word,
/// OUTPUT_MODE0, no saturate, and the surface (SINGLE_BUFFER, stride, shadow).
fn assert_pipeline(f: &Float, is_clear: bool, ws: &[(u32, u32)], w: u32, h: u32, copied_back: u64) {
    let name = f.name;
    let color = last(ws, GC_PE_COLOR_FORMAT).unwrap_or_else(|| panic!("{name}: nothing reached the PE"));
    assert_eq!(color, f.color_format_word(is_clear, false), "{name}: PE_COLOR_FORMAT {color:#010x}");
    let tiled = rnndb("VIVS_PE_COLOR_FORMAT_SUPER_TILED") | rnndb("VIVS_PE_COLOR_FORMAT_SUPER_TILED_NEW");
    assert_eq!(color & tiled, 0, "{name}: no SUPER_TILED bit");
    let ext = last(ws, GC_PS_CONTROL_EXT).expect("PS_CONTROL_EXT");
    assert_eq!(ext & 0x7, f.mode, "{name}: OUTPUT_MODE0");
    let ctl = last(ws, GC_PS_CONTROL).expect("PS_CONTROL");
    assert_eq!(ctl & GC_PS_CONTROL_SATURATE_RT0, 0, "{name}: a float is not saturated");

    let logic = last(ws, GC_PE_LOGIC_OP).expect("PE_LOGIC_OP");
    let single_mask = rnndb("VIVS_PE_LOGIC_OP_SINGLE_BUFFER__MASK");
    let single = (logic & single_mask) >> single_mask.trailing_zeros();
    let stride = last(ws, rnndb("VIVS_PE_COLOR_STRIDE")).expect("PE_COLOR_STRIDE");
    let addr = last(ws, pe_pipe_color_addr(0)).expect("PE_PIPE_COLOR_ADDR(0)");
    assert_eq!(last(ws, pe_pipe_color_addr(1)), Some(addr), "{name}: both pipes, one surface");
    if f.bytes == 8 {
        let (pw, _) = yakogl::vivante::wide_target::padded(w, h);
        assert_eq!(single, 2, "{name}: SINGLE_BUFFER(2), the tiled 64-bit value");
        assert_eq!(stride, pw * 8, "{name} {w}x{h}: the shadow's padded stride");
        assert_eq!(addr & 0xfff, 0, "{name}: the shadow starts on a page");
        assert_eq!(copied_back, u64::from(w * h * 8), "{name} {w}x{h}: the shadow went back into the storage");
    } else {
        assert_eq!(single, 1, "{name}: SINGLE_BUFFER(1), linear");
        assert_eq!(stride, ((w + 1) & !1) * f.bytes, "{name} {w}x{h}: linear pitch, the width rounded up to even");
        assert_eq!(copied_back, 0, "{name}: no shadow");
    }
}

/// Every pixel reads back as GL's channels of `COLOUR`, then 0 and alpha 1.
fn assert_read_back(f: &Float, w: u32, h: u32) {
    let got = read(f, w, h);
    let mut want = [0.0, 0.0, 0.0, 1.0];
    want[..f.channels].copy_from_slice(&COLOUR[..f.channels]);
    for (i, px) in got.chunks(4).enumerate() {
        assert_eq!(px, want, "{} {w}x{h}: pixel ({}, {})", f.name, i as u32 % w, i as u32 / w);
    }
}

/* ---- the tables ------------------------------------------------------------- */

/// The surface table: PE format, extended-format flag, pixel size, output mode.
#[test]
fn the_three_own_surfaces_take_their_pe_format_size_and_output_mode() {
    for f in OWN {
        assert!(rt_format_drawable(f.surf), "{}", f.name);
        assert!(gles_surf_is_float(f.surf), "{}", f.name);
        assert_eq!(pe_color_format(f.surf), (rnndb(f.pe), true), "{}: {} in FORMAT_EXT", f.name, f.pe);
        assert_eq!(gles_surf_bytes_per_pixel(f.surf), f.bytes, "{}", f.name);
        assert_eq!(ps_color_output_mode(f.surf), f.mode, "{}", f.name);
        assert_eq!(gles_surf_name(f.surf), f.name);
    }
    /* the RS format a float read-back or fill could use: none for 8 bytes,
     * and the 2- and 4-byte fills are the unorm A4R4G4B4 / A8R8G8B8 */
    assert_eq!(yakogl::vivante::rs_clear::rs_format(8), None, "RGBA16F: no 8-byte RS fill");
    assert_eq!(yakogl::vivante::rs_clear::rs_format(2), Some(GC_RS_FORMAT_A4R4G4B4));
    assert_eq!(yakogl::vivante::rs_clear::rs_format(4), Some(GC_RS_FORMAT_A8R8G8B8));
}

/// RGB16F and RG16F: a renderbuffer of either is an RGBA16F surface, so the
/// PE draws it as A16B16G16R16F, 8 bytes a pixel.
#[test]
fn rgb16f_and_rg16f_renderbuffers_draw_as_the_rgba16f_surface() {
    for f in MAPPED {
        on_gc7000(|| {
            target(f, 16, 16);
            let (e, ws) = draw_colour(f, 16, 16, false);
            assert_eq!(e, GL_NO_ERROR, "{}", f.name);
            let color = last(&ws, GC_PE_COLOR_FORMAT).expect("PE_COLOR_FORMAT");
            let mask = rnndb("VIVS_PE_COLOR_FORMAT_FORMAT_EXT__MASK");
            assert_eq!((color & mask) >> mask.trailing_zeros(), rnndb("PE_FORMAT_A16B16G16R16F"), "{}", f.name);
            assert_eq!(last(&ws, rnndb("VIVS_PE_COLOR_STRIDE")), Some(16 * 8), "{}: 8 bytes a pixel", f.name);
        });
    }
}

/// Etnaviv draws RG16F as `G16R16F` (etnaviv_format.c:151), 4 bytes a pixel,
/// where this back end draws the 8-byte RGBA16F shadow (the test above pins
/// today's). This is the etnaviv value.
#[test]
#[ignore = "RG16F is drawn as A16B16G16R16F (8 bytes) where etnaviv uses PE_FORMAT_G16R16F (etnaviv_format.c:151); GC arm item"]
fn rg16f_draws_as_g16r16f_like_etnaviv() {
    on_gc7000(|| {
        target(&RG16F, 16, 16);
        let (e, ws) = draw_colour(&RG16F, 16, 16, false);
        assert_eq!(e, GL_NO_ERROR);
        let color = last(&ws, GC_PE_COLOR_FORMAT).expect("PE_COLOR_FORMAT");
        let mask = rnndb("VIVS_PE_COLOR_FORMAT_FORMAT_EXT__MASK");
        assert_eq!((color & mask) >> mask.trailing_zeros(), rnndb("PE_FORMAT_G16R16F"));
        assert_eq!(last(&ws, rnndb("VIVS_PE_COLOR_STRIDE")), Some(16 * 4));
    });
}

/* ---- completeness ---------------------------------------------------------- */

/// The three own surfaces and the two mapped ones are complete renderbuffer
/// colour attachments.
#[test]
fn a_renderbuffer_of_each_is_a_complete_colour_attachment() {
    for f in OWN.iter().chain(MAPPED.iter()) {
        on_gc7000(|| target(f, 16, 16));
    }
}

/* ---- draws and clears ------------------------------------------------------ */

/// A draw with GL_BLEND off: the PE_COLOR_FORMAT word, OUTPUT_MODE0, no
/// blend, the surface (linear at the even-padded pitch for R16F / R32F, the
/// tiled shadow for RGBA16F), and the colour read back.
#[test]
fn a_draw_into_each_emits_its_words_and_reads_back() {
    for f in OWN {
        for (w, h) in SIZES {
            on_gc7000(|| {
                target(f, w, h);
                with_platform(|p| p.model = Some((w, h, f.bytes, f.pixel())));
                let untiled0 = yakogl::vivante::wide_target::counts().1;
                let (e, ws) = draw_colour(f, w, h, false);
                assert_eq!(e, GL_NO_ERROR, "{} {w}x{h}", f.name);
                assert_pipeline(f, false, &ws, w, h, yakogl::vivante::wide_target::counts().1 - untiled0);
                assert_eq!(last(&ws, GC_PE_ALPHA_CONFIG), Some(0), "{}: GL_BLEND is off", f.name);
                assert_read_back(f, w, h);
            });
        }
    }
}

/// glClearBufferfv: the quad through the pixel engine, not the resolve
/// engine, with the same format word and surface as a draw; the colour reads
/// back.
#[test]
fn a_clear_of_each_is_the_quad_and_reads_back() {
    for f in OWN {
        for (w, h) in SIZES {
            on_gc7000(|| {
                target(f, w, h);
                with_platform(|p| p.model = Some((w, h, f.bytes, f.pixel())));
                let untiled0 = yakogl::vivante::wide_target::counts().1;
                let (e, ws) = clear(COLOUR);
                assert_eq!(e, GL_NO_ERROR, "{} {w}x{h}", f.name);
                assert!(
                    ws.iter().all(|(r, _)| *r != GC_RS_CONFIG && *r != GC_RS_KICKER),
                    "{}: a float clear is not a resolve-engine fill",
                    f.name
                );
                assert_pipeline(f, true, &ws, w, h, yakogl::vivante::wide_target::counts().1 - untiled0);
                assert_read_back(f, w, h);
            });
        }
    }
}

/// A draw with GL_BLEND on and SRC_ALPHA / ONE_MINUS_SRC_ALPHA: the blender is
/// enabled for each float target, the 32-bit R32F included, and the draw
/// loses OVERWRITE. (ES 3.0 4.1.8 ignores blending for integer buffers only;
/// blending a 32-bit float one is what EXT_float_blend allows, which this
/// back end does not look at.)
#[test]
fn gl_blend_enables_the_blender_on_every_float_target_r32f_included() {
    for f in OWN {
        on_gc7000(|| {
            target(f, 16, 16);
            let (e, ws) = draw_colour(f, 16, 16, true);
            assert_eq!(e, GL_NO_ERROR, "{}", f.name);
            let alpha = last(&ws, GC_PE_ALPHA_CONFIG).expect("PE_ALPHA_CONFIG");
            assert_ne!(alpha & GC_PE_ALPHA_BLEND_ENABLE_COLOR, 0, "{}: blender on ({alpha:#x})", f.name);
            let color = last(&ws, GC_PE_COLOR_FORMAT).expect("PE_COLOR_FORMAT");
            assert_eq!(color, f.color_format_word(false, true), "{}: no OVERWRITE while blending", f.name);
        });
    }
}

/// RGB16F and RG16F (the RGBA16F surface): the channels GL's format has read
/// back as drawn.
#[test]
fn rgb16f_and_rg16f_read_back_their_channels() {
    for f in MAPPED {
        on_gc7000(|| {
            target(f, 16, 16);
            with_platform(|p| p.model = Some((16, 16, 8, f.pixel())));
            assert_eq!(draw_colour(f, 16, 16, false).0, GL_NO_ERROR, "{}", f.name);
            let got = read(f, 16, 16);
            let n = f.channels;
            assert_eq!(got[..n], COLOUR[..n], "{}: the channels the format has", f.name);
        });
    }
}
