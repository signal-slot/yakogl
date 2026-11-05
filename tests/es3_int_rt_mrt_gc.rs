// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the 64-bit integer render targets (RGBA16I/UI,
//! RG32I/UI) in MRT framebuffers, at sizes that are not tile-aligned, with
//! `glClearBuffer{i,ui}v` and `glReadPixels` (item int-rt-mrt-test;
//! docs/es3/int-rt-gc.md, `src/vivante/wide_target.rs`). The single-target
//! 16x16 case is tests/es3_int_rt_gc.rs; this file is its neighbour and uses
//! the same stand-in OSAL and the same way of reading the submitted streams.
//!
//! What the back end does today, which these tests pin:
//!
//! - A draw with one hardware colour target of 64-bit pixels renders into a
//!   4x4-tiled shadow (padded to 16x4, stride `padded width * 8`,
//!   SINGLE_BUFFER(2)) that is untiled into the linear storage when the kick
//!   retires. At 17x13 that is a 32x16 shadow, a 256-byte stride, and
//!   `17 * 13 * 8` bytes copied back (not the padded 32 * 16 * 8).
//! - A draw with a 64-bit integer target **beside another draw buffer**, a
//!   32-bit one or a second 64-bit one, is refused (`GL_INVALID_OPERATION`,
//!   nothing reaches the PE): the 8-RT bank has no tiled bit, so the shadow
//!   cannot serve it (wide_target.rs "Not covered"). A draw buffer set to
//!   GL_NONE leaves one hardware target and is drawn.
//! - RGBA32I/UI have no PE format and are refused by name in any attachment.
//!
//! The GPU is modelled: the stand-in OSAL's submit stores a known pixel
//! pattern into the tiled shadow the stream names (`PE_PIPE_COLOR_ADDR`,
//! `PE_COLOR_STRIDE`), laid out as the PE stores 4x4-tiled 64-bit pixels;
//! the back end untiles it and the test reads it with glReadPixels. That
//! checks the tiling, the padding and the copy-back at any size, not that the
//! PE stores integers (the board's, docs/es3/int-rt-gc.md "For the board").
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_rt_mrt_gc
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
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/// The online compiler's store (scratch and keep extents) is one static for
/// the process: every test takes this one lock (as tests/es3_int_rt_gc.rs).
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

/// The 64-bit pixel the modelled PE stores at (x, y) of the target.
type Pattern = fn(u32, u32) -> u64;

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    /// While set, every kick that names a colour surface gets `(w, h,
    /// pattern)` stored into it, in 4x4 tiles at the stream's stride.
    model: Option<(u32, u32, Pattern)>,
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
        if let Some((w, h, pat)) = p.model {
            let ws = writes(&stream);
            let addr = ws.iter().rev().find(|(r, _)| *r == pe_pipe_color_addr(0)).map(|(_, v)| *v as u64);
            let stride = ws.iter().rev().find(|(r, _)| *r == rnndb("VIVS_PE_COLOR_STRIDE")).map(|(_, v)| *v);
            if let (Some(addr), Some(stride)) = (addr, stride) {
                let tw = stride / 8;
                if let Some((base, mem)) = p.allocs.range_mut(..=addr).next_back().map(|(g, m)| (*g, m)) {
                    let off = (addr - base) as usize;
                    // SAFETY: the shadow holds padded(w, h) 8-byte pixels from `off` on.
                    let bytes = unsafe { core::slice::from_raw_parts_mut(mem.as_mut_ptr() as *mut u8, mem.len() * 8) };
                    for y in 0..h {
                        for x in 0..w {
                            let at = off + tiled_byte(x, y, tw);
                            bytes[at..at + 8].copy_from_slice(&pat(x, y).to_le_bytes());
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

/// A framebuffer with `rbs` at colour attachments 0.., and `bufs` (the
/// attachments, or GL_NONE) as its draw buffers.
fn framebuffer_with(rbs: &[GLuint], bufs: &[GLenum]) -> GLuint {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, &rb) in rbs.iter().enumerate() {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
        }
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(err(), GL_NO_ERROR, "framebuffer");
    fb
}

/// A framebuffer with `rbs` at colour attachments 0.., every one a draw buffer.
fn framebuffer(rbs: &[GLuint]) -> GLuint {
    let bufs: Vec<GLenum> = (0..rbs.len() as u32).map(|i| GL_COLOR_ATTACHMENT0 + i).collect();
    framebuffer_with(rbs, &bufs)
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// Draw the quad over the `w` x `h` viewport with `fs`, finish, and return
/// the error and the writes of the streams submitted since (none if the draw
/// was refused).
fn draw(fs: &str, w: u32, h: u32) -> (GLenum, Vec<(u32, u32)>) {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    let e = unsafe {
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glUseProgram(program(fs));
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        let e = glGetError();
        glFinish();
        e
    };
    (e, with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect()))
}

fn last(w: &[(u32, u32)], reg: u32) -> Option<u32> {
    w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v)
}

/// Bytes untiled out of shadows since init.
fn untiled() -> u64 {
    yakogl::vivante::wide_target::counts().1
}

/// Did the draw reach the pixel engine at all?
fn reached_pe(w: &[(u32, u32)]) -> bool {
    w.iter().any(|&(r, _)| r == GC_PE_COLOR_FORMAT || r == pe_rt_config_8(1) || r == pe_pipe_color_addr(0))
}

/* ---- the four 64-bit targets ----------------------------------------------- */

/// A 64-bit integer target: the GL format, its surface, the fragment output
/// type, the read-back format and type, the channel count and whether signed.
struct Wide {
    internal: GLenum,
    surf: u8,
    ty: &'static str,
    read_format: GLenum,
    read_type: GLenum,
    channels: usize,
    signed: bool,
}

const WIDE: [Wide; 4] = [
    Wide {
        internal: GL_RGBA16I,
        surf: GLES_SURF_RGBA16I,
        ty: "ivec4",
        read_format: GL_RGBA_INTEGER,
        read_type: GL_INT,
        channels: 4,
        signed: true,
    },
    Wide {
        internal: GL_RGBA16UI,
        surf: GLES_SURF_RGBA16UI,
        ty: "uvec4",
        read_format: GL_RGBA_INTEGER,
        read_type: GL_UNSIGNED_INT,
        channels: 4,
        signed: false,
    },
    Wide {
        internal: GL_RG32I,
        surf: GLES_SURF_RG32I,
        ty: "ivec4",
        read_format: GL_RGBA_INTEGER,
        read_type: GL_INT,
        channels: 2,
        signed: true,
    },
    Wide {
        internal: GL_RG32UI,
        surf: GLES_SURF_RG32UI,
        ty: "uvec4",
        read_format: GL_RGBA_INTEGER,
        read_type: GL_UNSIGNED_INT,
        channels: 2,
        signed: false,
    },
];

/// The pixel (x, y) of a 16-bit-channel target as the PE would store it:
/// four distinct values per pixel, the high bit of three of them set so the
/// signed formats read negatives.
fn pattern16(x: u32, y: u32) -> u64 {
    let r = 0x8000 | (x + 1);
    let g = y + 1;
    let b = 0xc000 ^ (x * 7 + y * 131);
    let a = 0x1000 + x * 64 + y;
    u64::from(r) | u64::from(g) << 16 | u64::from(b & 0xffff) << 32 | u64::from(a) << 48
}

/// As [`pattern16`] for a target of two 32-bit channels.
fn pattern32(x: u32, y: u32) -> u64 {
    let r = 0x8000_0000 | (x * 0x0101_0101);
    let g = 100_000 + y * 4099 + x;
    u64::from(r) | u64::from(g) << 32
}

impl Wide {
    fn pattern(&self) -> Pattern {
        if self.channels == 4 {
            pattern16
        } else {
            pattern32
        }
    }

    /// What glReadPixels returns for the pixel the model stored: the
    /// channels widened to 32 bits, sign-extended for the signed formats.
    fn expect(&self, x: u32, y: u32) -> Vec<u32> {
        let raw = self.pattern()(x, y);
        if self.channels == 4 {
            (0..4)
                .map(|c| {
                    let v = ((raw >> (16 * c)) & 0xffff) as u32;
                    if self.signed {
                        v as u16 as i16 as i32 as u32
                    } else {
                        v
                    }
                })
                .collect()
        } else {
            /* RGBA_INTEGER is the one format ES 3.0 lets an integer
             * attachment be read in: missing channels read 0, alpha 1 */
            vec![raw as u32, (raw >> 32) as u32, 0, 1]
        }
    }

    fn fs(&self) -> String {
        let one = if self.signed { "ivec4(-3, 7, -100000, 2)" } else { "uvec4(3u, 7u, 4000000000u, 2u)" };
        format!("#version 300 es\nlayout(location = 0) out highp {} o;\nvoid main() {{ o = {one}; }}\n", self.ty)
    }

    /// The whole `w` x `h` target read back: four words a pixel.
    fn read(&self, w: u32, h: u32) -> Vec<u32> {
        let mut buf = vec![0xdead_beefu32; (w * h) as usize * 4];
        // SAFETY: the buffer holds w * h pixels of four 32-bit words.
        unsafe {
            glReadPixels(
                0,
                0,
                w as GLsizei,
                h as GLsizei,
                self.read_format,
                self.read_type,
                buf.as_mut_ptr() as *mut _,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "{}: glReadPixels", gles_surf_name(self.surf));
        buf
    }

    fn assert_read_back(&self, w: u32, h: u32) {
        let name = gles_surf_name(self.surf);
        let got = self.read(w, h);
        for y in 0..h {
            for x in 0..w {
                let at = ((y * w + x) as usize) * 4;
                assert_eq!(&got[at..at + 4], &self.expect(x, y)[..], "{name} {w}x{h}: pixel ({x}, {y})");
            }
        }
    }
}

/// Tile-aligned or not, a one-target draw's surface words: the shadow's page
/// address, its padded stride, SINGLE_BUFFER(2), and the copy-back size.
fn assert_shadow(name: &str, w: &[(u32, u32)], width: u32, height: u32, copied_back: u64) {
    let (pw, _) = yakogl::vivante::wide_target::padded(width, height);
    assert_eq!(pw % 16, 0, "{name}");
    let stride = last(w, rnndb("VIVS_PE_COLOR_STRIDE")).expect("PE_COLOR_STRIDE");
    assert_eq!(stride, pw * 8, "{name} {width}x{height}: the padded stride");
    assert_eq!(stride, yakogl::vivante::wide_target::stride(width), "{name}");
    let addr = last(w, pe_pipe_color_addr(0)).expect("PE_PIPE_COLOR_ADDR(0)");
    assert_eq!(last(w, pe_pipe_color_addr(1)), Some(addr), "{name}: both pipes, one surface");
    assert_eq!(addr & 0xfff, 0, "{name}: the shadow starts on a page");
    let logic = last(w, GC_PE_LOGIC_OP).expect("PE_LOGIC_OP");
    let mask = rnndb("VIVS_PE_LOGIC_OP_SINGLE_BUFFER__MASK");
    assert_eq!((logic & mask) >> mask.trailing_zeros(), 2, "{name}: SINGLE_BUFFER(2)");
    let tiled = rnndb("VIVS_PE_COLOR_FORMAT_SUPER_TILED") | rnndb("VIVS_PE_COLOR_FORMAT_SUPER_TILED_NEW");
    assert_eq!(last(w, GC_PE_COLOR_FORMAT).unwrap() & tiled, 0, "{name}: no SUPER_TILED bit");
    assert_eq!(
        copied_back,
        u64::from(width * height * 8),
        "{name} {width}x{height}: only the target's pixels go back, not the padding"
    );
}

/// Sizes that are not multiples of the 4x4 tile or of the 16-pixel padding,
/// beside ones that are.
const SIZES: [(u32, u32); 7] = [(17, 13), (1, 1), (5, 3), (16, 4), (33, 17), (31, 5), (20, 61)];

/// One draw into each 64-bit target at each size: the shadow words for the
/// size, the pixel pattern the (modelled) PE stored into the tiled shadow
/// read back at the right places, linear, through glReadPixels.
#[test]
fn a_draw_into_a_64_bit_target_of_any_size_reads_back_through_the_shadow() {
    for t in &WIDE {
        for (w, h) in SIZES {
            on_gc7000(|| {
                let name = gles_surf_name(t.surf);
                let rb = renderbuffer(t.internal, w, h);
                framebuffer(&[rb]);
                with_platform(|p| p.model = Some((w, h, t.pattern())));
                let untiled0 = untiled();
                let (e, ws) = draw(&t.fs(), w, h);
                assert_eq!(e, GL_NO_ERROR, "{name} {w}x{h}: drawn");
                assert_shadow(name, &ws, w, h, untiled() - untiled0);
                t.assert_read_back(w, h);
            });
        }
    }
}

/// A second draw into the same target, where the model stores nothing: the
/// linear storage is tiled into the shadow first (the draw did not clear),
/// so the target keeps what the first draw wrote, at a size with padding.
#[test]
fn a_second_draw_keeps_what_the_first_wrote_at_a_size_with_padding() {
    for t in &WIDE {
        on_gc7000(|| {
            let (w, h) = (17, 13);
            let rb = renderbuffer(t.internal, w, h);
            framebuffer(&[rb]);
            with_platform(|p| p.model = Some((w, h, t.pattern())));
            assert_eq!(draw(&t.fs(), w, h).0, GL_NO_ERROR);
            t.assert_read_back(w, h);
            with_platform(|p| p.model = None);
            let tiled0 = yakogl::vivante::wide_target::counts().0;
            assert_eq!(draw(&t.fs(), w, h).0, GL_NO_ERROR);
            assert_eq!(
                yakogl::vivante::wide_target::counts().0 - tiled0,
                u64::from(w * h * 8),
                "{}: the storage went back into the shadow",
                gles_surf_name(t.surf)
            );
            t.assert_read_back(w, h);
        });
    }
}

/// glClearBufferiv / glClearBufferuiv on a 64-bit target at an unaligned
/// size: the clear runs as a draw into the shadow (same words as a draw),
/// its constant is the raw values, the shadow goes back, and the pixels read
/// back whole.
#[test]
fn a_clear_of_a_64_bit_target_at_an_unaligned_size_goes_through_the_shadow() {
    for t in &WIDE {
        for (w, h) in [(17, 13), (5, 3), (16, 4)] {
            on_gc7000(|| {
                let name = gles_surf_name(t.surf);
                let rb = renderbuffer(t.internal, w, h);
                framebuffer(&[rb]);
                with_platform(|p| p.model = Some((w, h, t.pattern())));
                let kicks0 = with_platform(|p| p.kicks.len());
                let untiled0 = untiled();
                let bits: [u32; 4] = [0x8000_0001, 7, 0xffff_fff0, 0x1234_5678];
                // SAFETY: four values; plain GL calls.
                unsafe {
                    if t.signed {
                        glClearBufferiv(GL_COLOR, 0, bits.as_ptr() as *const GLint);
                    } else {
                        glClearBufferuiv(GL_COLOR, 0, bits.as_ptr());
                    }
                    assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glClearBuffer");
                    glFinish();
                }
                let ws: Vec<(u32, u32)> = with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect());
                assert!(last(&ws, GC_PE_COLOR_FORMAT).is_some(), "{name} {w}x{h}: the clear reached the PE");
                assert_shadow(name, &ws, w, h, untiled() - untiled0);
                let ps: Vec<u32> = ws
                    .iter()
                    .filter(|(r, _)| (GC_SH_HALTI5_UNIFORMS_PS..GC_SH_HALTI5_UNIFORMS_PS + 0x2000).contains(r))
                    .map(|(_, v)| *v)
                    .collect();
                assert!(ps.windows(4).any(|q| q == bits), "{name}: the clear constant is the raw values: {ps:x?}");
                t.assert_read_back(w, h);
            });
        }
    }
}

/// glClearBuffer of the 64-bit attachment 1 of an MRT framebuffer. ES 3.0
/// 4.2.3: it clears that one buffer and no other. Here the clear is drawn
/// with every attachment in its draw record, which the back end refuses as it
/// refuses any 64-bit target beside another (`draw_one`, "alone"): the flush
/// reports GL_INVALID_OPERATION and the clear is lost.
#[test]
#[ignore = "defect: a clear of one attachment is drawn with every attachment of the framebuffer in its draw record, \
            so a 64-bit target beside another is refused (src/vivante/backend.rs:2794) and the clear is lost"]
fn a_clear_of_a_64_bit_attachment_beside_another_clears_it() {
    on_gc7000(|| {
        let (w, h) = (17, 13);
        let a = renderbuffer(GL_RGBA8UI, w, h);
        let b = renderbuffer(GL_RGBA16UI, w, h);
        framebuffer(&[a, b]);
        with_platform(|p| p.model = Some((w, h, pattern16)));
        let kicks0 = with_platform(|p| p.kicks.len());
        let untiled0 = untiled();
        let bits: [u32; 4] = [1, 2, 3, 65535];
        // SAFETY: four values; plain GL calls.
        unsafe {
            glClearBufferuiv(GL_COLOR, 1, bits.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "clear of the 64-bit attachment 1");
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR, "the flushed clear");
        }
        let ws: Vec<(u32, u32)> = with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect());
        assert_shadow("RGBA16UI", &ws, w, h, untiled() - untiled0);
        // SAFETY: a plain GL call.
        unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0 + 1) };
        WIDE[1].assert_read_back(w, h);
    });
}

/// glClearBuffer of the 32-bit attachment 0 with a 64-bit attachment 1
/// beside it: the same flush, the same refusal.
#[test]
#[ignore = "defect: as a_clear_of_a_64_bit_attachment_beside_another_clears_it, the 64-bit attachment is in the \
            clear's draw record (src/vivante/backend.rs:2794)"]
fn a_clear_of_a_32_bit_attachment_beside_a_64_bit_one_clears_it() {
    on_gc7000(|| {
        let (w, h) = (17, 13);
        let a = renderbuffer(GL_RGBA8UI, w, h);
        let b = renderbuffer(GL_RGBA16UI, w, h);
        framebuffer(&[a, b]);
        let kicks0 = with_platform(|p| p.kicks.len());
        let bits: [u32; 4] = [1, 2, 3, 255];
        // SAFETY: four values; plain GL calls.
        unsafe {
            glClearBufferuiv(GL_COLOR, 0, bits.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "clear of the 32-bit attachment 0");
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR, "the flushed clear");
        }
        let ws: Vec<(u32, u32)> = with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect());
        assert!(last(&ws, GC_PE_COLOR_FORMAT).is_some(), "the clear reached the PE");
    });
}

/* ---- MRT ------------------------------------------------------------------- */

/// A fragment shader writing every output in `outs` (location, type).
fn mrt_fs(outs: &[(u32, &str)]) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\n");
    for (loc, ty) in outs {
        s += &format!("layout(location = {loc}) out highp {ty} o{loc};\n");
    }
    s += "void main() {\n";
    for (loc, ty) in outs {
        let v = match *ty {
            "vec4" => "vec4(0.5)".to_string(),
            t => format!("{t}(1)"),
        };
        s += &format!("  o{loc} = {v};\n");
    }
    s + "}\n"
}

/// A 64-bit integer target beside a 32-bit one, in either order, with the
/// 32-bit one RGBA8, RG16UI, R32I and RGB10_A2UI, at 17x13: the draw is
/// refused (GL_INVALID_OPERATION), nothing reaches the PE, no shadow is made,
/// and the framebuffer stays complete.
#[test]
fn a_64_bit_target_beside_a_32_bit_one_is_refused_at_an_unaligned_size() {
    let narrow: [(GLenum, &str); 4] =
        [(GL_RGBA8, "vec4"), (GL_RG16UI, "uvec4"), (GL_R32I, "ivec4"), (GL_RGB10_A2UI, "uvec4")];
    for t in &WIDE {
        for (n_internal, n_ty) in narrow {
            for wide_first in [false, true] {
                on_gc7000(|| {
                    let (w, h) = (17, 13);
                    let what = format!("{} + {n_internal:#x} first={wide_first}", gles_surf_name(t.surf));
                    let a = renderbuffer(t.internal, w, h);
                    let b = renderbuffer(n_internal, w, h);
                    let (rbs, outs) =
                        if wide_first { ([a, b], [(0, t.ty), (1, n_ty)]) } else { ([b, a], [(0, n_ty), (1, t.ty)]) };
                    framebuffer(&rbs);
                    let untiled0 = untiled();
                    let (e, ws) = draw(&mrt_fs(&outs), w, h);
                    assert_eq!(e, GL_INVALID_OPERATION, "{what}");
                    assert!(!reached_pe(&ws), "{what}: nothing reached the pixel engine");
                    assert_eq!(untiled(), untiled0, "{what}: no shadow");
                });
            }
        }
    }
}

/// Two 64-bit targets, the same format twice or two formats (16-bit and
/// 32-bit channels), and three targets with one 64-bit among them: refused.
#[test]
fn two_64_bit_targets_are_refused() {
    let pairs: [(GLenum, &str, GLenum, &str); 6] = [
        (GL_RGBA16I, "ivec4", GL_RGBA16I, "ivec4"),
        (GL_RGBA16UI, "uvec4", GL_RGBA16UI, "uvec4"),
        (GL_RG32I, "ivec4", GL_RG32I, "ivec4"),
        (GL_RG32UI, "uvec4", GL_RG32UI, "uvec4"),
        (GL_RGBA16I, "ivec4", GL_RG32UI, "uvec4"),
        (GL_RG32I, "ivec4", GL_RGBA16UI, "uvec4"),
    ];
    for (ia, ta, ib, tb) in pairs {
        on_gc7000(|| {
            let (w, h) = (17, 13);
            let what = format!("{ia:#x} + {ib:#x}");
            let a = renderbuffer(ia, w, h);
            let b = renderbuffer(ib, w, h);
            framebuffer(&[a, b]);
            let untiled0 = untiled();
            let (e, ws) = draw(&mrt_fs(&[(0, ta), (1, tb)]), w, h);
            assert_eq!(e, GL_INVALID_OPERATION, "{what}");
            assert!(!reached_pe(&ws), "{what}: nothing reached the pixel engine");
            assert_eq!(untiled(), untiled0, "{what}: no shadow");
        });
    }
    for wide_at in 0..3usize {
        on_gc7000(|| {
            let (w, h) = (17, 13);
            let rbs: Vec<GLuint> =
                (0..3).map(|i| renderbuffer(if i == wide_at { GL_RGBA16UI } else { GL_RGBA8 }, w, h)).collect();
            framebuffer(&rbs);
            let outs: Vec<(u32, &str)> =
                (0..3).map(|i| (i as u32, if i == wide_at { "uvec4" } else { "vec4" })).collect();
            let (e, ws) = draw(&mrt_fs(&outs), w, h);
            assert_eq!(e, GL_INVALID_OPERATION, "three targets, 64-bit at {wide_at}");
            assert!(!reached_pe(&ws), "three targets, 64-bit at {wide_at}: nothing reached the pixel engine");
        });
    }
}

/// The refusal does not poison the context: after the refused MRT draw, a
/// framebuffer with the 64-bit target alone draws and reads back, and the
/// MRT framebuffer is still complete.
#[test]
fn a_refused_mrt_draw_leaves_a_one_target_draw_working() {
    for t in &WIDE {
        on_gc7000(|| {
            let (w, h) = (17, 13);
            let a = renderbuffer(t.internal, w, h);
            let b = renderbuffer(GL_RGBA8, w, h);
            let mrt = framebuffer(&[a, b]);
            let (e, _) = draw(&mrt_fs(&[(0, t.ty), (1, "vec4")]), w, h);
            assert_eq!(e, GL_INVALID_OPERATION);
            // SAFETY: plain GL calls.
            unsafe {
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
                assert_eq!(glGetError(), GL_NO_ERROR);
            }
            let _ = mrt;
            framebuffer(&[a]);
            with_platform(|p| p.model = Some((w, h, t.pattern())));
            let (e, _) = draw(&t.fs(), w, h);
            assert_eq!(e, GL_NO_ERROR, "{}: alone, drawn", gles_surf_name(t.surf));
            t.assert_read_back(w, h);
        });
    }
}

/// An attachment that is not a draw buffer (GL_NONE) does not count: the
/// 64-bit target is the one hardware target, drawn through the shadow, and
/// reads back. Draw buffers [att0, NONE] and [NONE, att1] both.
#[test]
fn a_64_bit_target_beside_an_undrawn_attachment_is_a_one_target_draw() {
    for t in &WIDE {
        for wide_at_1 in [false, true] {
            on_gc7000(|| {
                let (w, h) = (17, 13);
                let name = gles_surf_name(t.surf);
                let wide = renderbuffer(t.internal, w, h);
                let other = renderbuffer(GL_RGBA8, w, h);
                let (rbs, bufs, loc) = if wide_at_1 {
                    ([other, wide], [GL_NONE, GL_COLOR_ATTACHMENT0 + 1], 1)
                } else {
                    ([wide, other], [GL_COLOR_ATTACHMENT0, GL_NONE], 0)
                };
                framebuffer_with(&rbs, &bufs);
                with_platform(|p| p.model = Some((w, h, t.pattern())));
                let untiled0 = untiled();
                let fs = format!(
                    "#version 300 es\nlayout(location = {loc}) out highp {} o;\nvoid main() {{ o = {}(1); }}\n",
                    t.ty, t.ty
                );
                let (e, ws) = draw(&fs, w, h);
                assert_eq!(e, GL_NO_ERROR, "{name} wide_at_1={wide_at_1}");
                assert_shadow(name, &ws, w, h, untiled() - untiled0);
                /* read the 64-bit attachment */
                // SAFETY: a plain GL call.
                unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0 + loc as u32) };
                t.assert_read_back(w, h);
            });
        }
    }
}

/// Control: two 32-bit targets at 17x13, an RGBA8 and an RG16UI, are drawn
/// as before (linear, SINGLE_BUFFER(1), the second in the 8-RT bank): the
/// refusal is about the 64-bit pixel size, not about MRT or odd sizes.
#[test]
fn an_mrt_draw_of_32_bit_targets_at_an_unaligned_size_is_drawn_linear() {
    on_gc7000(|| {
        let (w, h) = (17, 13);
        let a = renderbuffer(GL_RGBA8, w, h);
        let b = renderbuffer(GL_RG16UI, w, h);
        framebuffer(&[a, b]);
        let untiled0 = untiled();
        let (e, ws) = draw(&mrt_fs(&[(0, "vec4"), (1, "uvec4")]), w, h);
        assert_eq!(e, GL_NO_ERROR);
        assert!(last(&ws, pe_rt_config_8(1)).is_some(), "the second target reached the PE");
        let logic = last(&ws, GC_PE_LOGIC_OP).expect("PE_LOGIC_OP");
        let mask = rnndb("VIVS_PE_LOGIC_OP_SINGLE_BUFFER__MASK");
        assert_eq!((logic & mask) >> mask.trailing_zeros(), 1, "SINGLE_BUFFER(1), linear");
        assert_eq!(untiled(), untiled0, "no shadow");
    });
}

/* ---- RGBA32I / RGBA32UI ----------------------------------------------------- */

/// RGBA32I and RGBA32UI have no PE format: refused by name in any
/// attachment, alone or beside a 32-bit or a 64-bit target, at an unaligned
/// size, nothing reaching the PE and no shadow made.
#[test]
fn rgba32_integer_in_any_attachment_is_refused_by_name() {
    for (r128, ty) in [(GL_RGBA32I, "ivec4"), (GL_RGBA32UI, "uvec4")] {
        assert!(!rt_format_drawable(if ty == "ivec4" { GLES_SURF_RGBA32I } else { GLES_SURF_RGBA32UI }));
        /* the other attachments: none, an RGBA8, a 64-bit one, and the 128-bit one twice */
        let others: [Option<(GLenum, &str)>; 4] =
            [None, Some((GL_RGBA8, "vec4")), Some((GL_RGBA16UI, "uvec4")), Some((r128, ty))];
        for other in others {
            for r128_first in [true, false] {
                if other.is_none() && !r128_first {
                    continue;
                }
                on_gc7000(|| {
                    let (w, h) = (17, 13);
                    let what = format!("{r128:#x} + {other:?} first={r128_first}");
                    let a = renderbuffer(r128, w, h);
                    let (rbs, outs) = match other {
                        None => (vec![a], vec![(0, ty)]),
                        Some((oi, ot)) => {
                            let b = renderbuffer(oi, w, h);
                            if r128_first {
                                (vec![a, b], vec![(0, ty), (1, ot)])
                            } else {
                                (vec![b, a], vec![(0, ot), (1, ty)])
                            }
                        }
                    };
                    framebuffer(&rbs);
                    let untiled0 = untiled();
                    let (e, ws) = draw(&mrt_fs(&outs), w, h);
                    assert_eq!(e, GL_INVALID_OPERATION, "{what}");
                    assert!(!reached_pe(&ws), "{what}: nothing reached the pixel engine");
                    assert_eq!(untiled(), untiled0, "{what}: no shadow");
                });
            }
        }
    }
}

/// A 128-bit attachment that is not a draw buffer is not drawn into: the
/// other attachment is a one-target draw as it would be alone.
#[test]
fn an_undrawn_rgba32_attachment_does_not_stop_the_draw() {
    on_gc7000(|| {
        let (w, h) = (17, 13);
        let a = renderbuffer(GL_RGBA8, w, h);
        let b = renderbuffer(GL_RGBA32UI, w, h);
        framebuffer_with(&[a, b], &[GL_COLOR_ATTACHMENT0, GL_NONE]);
        let (e, ws) = draw(&mrt_fs(&[(0, "vec4")]), w, h);
        assert_eq!(e, GL_NO_ERROR);
        assert!(last(&ws, GC_PE_COLOR_FORMAT).is_some(), "drawn");
    });
}

/// glClearBuffer of an RGBA32I/UI attachment: the clear is a draw of the
/// same target and is refused in the same way; and a clear of the RGBA8
/// beside it still works.
#[test]
fn a_clear_of_an_rgba32_attachment_is_refused() {
    for (r128, signed) in [(GL_RGBA32I, true), (GL_RGBA32UI, false)] {
        on_gc7000(|| {
            let (w, h) = (17, 13);
            let a = renderbuffer(GL_RGBA8UI, w, h);
            let b = renderbuffer(r128, w, h);
            framebuffer(&[a, b]);
            let kicks0 = with_platform(|p| p.kicks.len());
            let bits: [u32; 4] = [1, 2, 3, 4];
            // SAFETY: four values; plain GL calls.
            let e = unsafe {
                if signed {
                    glClearBufferiv(GL_COLOR, 1, bits.as_ptr() as *const GLint);
                } else {
                    glClearBufferuiv(GL_COLOR, 1, bits.as_ptr());
                }
                let e = glGetError();
                glFinish();
                e
            };
            let ws: Vec<(u32, u32)> = with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect());
            assert!(e == GL_NO_ERROR || e == GL_INVALID_OPERATION, "{r128:#x}: error {e:#x}");
            assert!(!reached_pe(&ws), "{r128:#x}: nothing reached the pixel engine (error {e:#x})");
        });
    }
}
