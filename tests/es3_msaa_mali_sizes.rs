// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x renderbuffer framebuffers of the sizes dEQP draws at (119x131, 40x40)
//! and at tile edges, on the Mali (Valhall G610), through the GL API only,
//! on the host (mali-msaa-size-tests). It covers the Mali the way
//! tests/es3_msaa_gc_pad.rs covers the GC7000UL: a hidden size limit of a
//! 4x path shows at sizes that are not a multiple of the tile.
//!
//! The rig is that of tests/es3_msaa_mali.rs (the REAL Mali back end over a
//! mock OSAL whose submit runs a GPU model of the frame descriptors; no
//! shader is run, a covered sample takes the test shader's constant), copied
//! because that file is not a module of this one. It needs `msaa-fbo`.
//! Each size: the 4x FBO is complete with 4 samples, a clear and a draw
//! reach the Mali with `samples == 4`, every frame the model executes has the
//! FBO's width and height (the extent covers the whole FBO, edge tiles
//! included), and the resolve blit into a 1x FBO of the same size leaves
//! every pixel of the size resolved: the clear colour after a clear, the
//! shader colour after a covering draw, and for a half-covered draw the
//! colour wholly inside the edge, black wholly outside. HOST-ONLY: the
//! model, not the G610, executes the descriptors (BOARD-UNVERIFIED).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,msaa-fbo --test es3_msaa_mali_sizes
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "gpu-mali",
    feature = "msaa-fbo",
    not(feature = "report-es2")
))]
#![allow(dead_code)]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context::GlContext;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::backend::mali_draw;

/// What `mali_draw` answers for a draw it takes: the mock's queue.
const TAKEN: i32 = 0;

/// What the back end was handed by one draw, and what the Mali made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::target_gpu`.
    gpu: u64,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `GlesDraw::depth_id`.
    depth_id: u32,
    /// What `mali_draw` answered ([`TAKEN`] for a draw it takes, `-ENOTSUP`
    /// a refusal by name).
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
/// The ids `depth_resolve` was asked for.
static DEPTH_ASKED: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// The triangle the next draws rasterise in the model (clip space xyzw x3).
static TRIANGLE: Mutex<[f32; 12]> = Mutex::new([0.0; 12]);

/// The Mali's rotated 4x grid, as fractions of a pixel (both models).
const SAMPLE_POS: [(f32, f32); 4] = [(0.375, 0.125), (0.875, 0.375), (0.125, 0.625), (0.625, 0.875)];

/// The colour the test's fragment shader writes.
const COLOUR: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

fn bytes(rgba: [f32; 4]) -> [u8; 4] {
    rgba.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

fn covered(tri: &[f32; 12], x: f32, y: f32) -> bool {
    let p: Vec<(f32, f32)> = (0..3).map(|i| (tri[i * 4], tri[i * 4 + 1])).collect();
    let e = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
    let (w0, w1, w2) = (e(p[0], p[1]), e(p[1], p[2]), e(p[2], p[0]));
    (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0)
}

/// The clip-space position of sample `s` of pixel (x, y) in a w x h target.
fn sample_at(x: u32, y: u32, s: usize, w: u32, h: u32) -> (f32, f32) {
    let (dx, dy) = SAMPLE_POS[s];
    ((x as f32 + dx) / w as f32 * 2.0 - 1.0, (y as f32 + dy) / h as f32 * 2.0 - 1.0)
}

/* ---- without msaa-fbo: the smallest OSAL and stand-in back end ----------- */

mod gpu {
    use super::*;
    use std::sync::atomic::{AtomicI64, AtomicU64};
    use yakogl::mali::backend::{mali_depth_resolve, mali_init};
    use yakogl::mali::cs::Instr;
    use yakogl::mali::genxml::v10::*;
    use yakogl::osal::mali::{
        OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP,
    };
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
    use yakogl::util::ENOTSUP;

    /// The OSAL's answers: the board's (panthor-frame on the T6), as
    /// tests/mali_recreate_colour.rs has them.
    const GPU: OsalMaliGpu = OsalMaliGpu::T6;
    const HEAP: OsalMaliHeap =
        OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

    /// Buffers made so far: each one's GPU VA is its number times 16 MiB,
    /// far from any CPU address.
    static ALLOCS: AtomicU64 = AtomicU64::new(0);
    static FENCES: AtomicI64 = AtomicI64::new(0);
    /// Every live buffer: (GPU VA, CPU address, size).
    static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());
    /// Freed buffers, the last freed handed back (zeroed) to the next
    /// allocation of its size: a renderbuffer specified again at its size comes back at the
    /// address it had, the case keying by address must survive.
    static FREED: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());
    /// The application triangles drawn and not yet executed, in draw order:
    /// each RUN_IDVS of a stream takes the next.
    static PENDING: Mutex<Vec<[f32; 12]>> = Mutex::new(Vec::new());
    /// Every frame the model executed.
    pub static FRAMES: Mutex<Vec<Frame>> = Mutex::new(Vec::new());

    unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        let recycled = {
            let mut f = FREED.lock().unwrap();
            f.iter().rposition(|b| b.2 == size).map(|i| f.remove(i))
        };
        let (gpu, cpu) = match recycled {
            Some((gpu, cpu, _)) => {
                core::ptr::write_bytes(cpu as *mut u8, 0, size);
                (gpu, cpu)
            }
            None => {
                let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
                assert!(size <= 16 << 20);
                let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr() as usize;
                (0x40_0000_0000 + n * (16 << 20), cpu)
            }
        };
        BUFFERS.lock().unwrap().push((gpu, cpu, size));
        *out = OsalGpuMem { cpu: cpu as *mut c_void, gpu, phys: 0, size, impl_: gpu as *mut c_void };
        0
    }
    unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
        let gpu = (*mem).gpu;
        if gpu != 0 {
            let mut b = BUFFERS.lock().unwrap();
            if let Some(i) = b.iter().position(|e| e.0 == gpu) {
                FREED.lock().unwrap().push(b.remove(i));
            }
        }
        *mem = OsalGpuMem::ZERO;
        0
    }
    unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
    unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
        0
    }
    unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
        assert_eq!(len, core::mem::size_of::<OsalMaliKick>());
        let k = &*kick.cast::<OsalMaliKick>();
        execute(k.stream_gpu, k.stream_bytes as usize);
        FENCES.fetch_add(1, Ordering::Relaxed) + 1
    }
    unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }
    unsafe extern "C" fn m_completed() -> i64 {
        FENCES.load(Ordering::Relaxed)
    }
    unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
        match what {
            OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
            OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => {
                *out.cast::<OsalMaliHeap>() = HEAP
            }
            _ => return -ENOTSUP,
        }
        0
    }
    /// The library only calls the log hook as `log("%s", line)`.
    unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
        eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
    }

    static OSAL: OsalOps = OsalOps {
        init: Some(m_init),
        shutdown: None,
        gpu_alloc: Some(m_alloc),
        gpu_free: Some(m_free),
        mmu_map: None,
        map_phys: None,
        gpu_write: None,
        gpu_read: None,
        cache_flush: Some(m_cache),
        cache_invd: Some(m_cache),
        submit: Some(m_submit),
        wait: Some(m_wait),
        completed: Some(m_completed),
        reg_read: None,
        reg_write: None,
        query: Some(m_query),
        // SAFETY: see `m_log`: a two-pointer function has the variadic
        // one's calling convention for these arguments.
        log: Some(unsafe {
            core::mem::transmute::<
                unsafe extern "C" fn(*const c_char, *const c_char),
                unsafe extern "C" fn(*const c_char, ...),
            >(m_log)
        }),
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    };

    /// The CPU address of `len` bytes at GPU address `gpu`.
    fn cpu_of(gpu: u64, len: usize) -> *mut u8 {
        let b = BUFFERS.lock().unwrap();
        let &(base, cpu, _) = b
            .iter()
            .find(|&&(base, _, size)| gpu >= base && gpu + len as u64 <= base + size as u64)
            .unwrap_or_else(|| panic!("no buffer holds 0x{gpu:x}+{len}"));
        (cpu + (gpu - base) as usize) as *mut u8
    }

    fn read<const N: usize>(gpu: u64) -> [u8; N] {
        // SAFETY: a live buffer holds the range (`cpu_of`).
        unsafe { core::ptr::read_unaligned(cpu_of(gpu, N) as *const [u8; N]) }
    }

    fn pixel(gpu: u64) -> [u8; 4] {
        read::<4>(gpu)
    }

    fn put(gpu: u64, p: [u8; 4]) {
        // SAFETY: a live buffer holds the range (`cpu_of`).
        unsafe { core::ptr::write_unaligned(cpu_of(gpu, 4) as *mut [u8; 4], p) }
    }

    /// The colour preload of a frame, as its DCD 0 and texture say.
    #[derive(Clone, Copy, Debug)]
    pub struct Preload {
        pub flags: DcdFlags0,
        /// `Texture.Sample count` (the count).
        pub samples: u32,
        pub plane: GenericPlane,
    }

    /// One frame (RUN_FRAGMENT) as the model read and executed it.
    #[derive(Clone, Debug)]
    pub struct Frame {
        pub params: FramebufferParameters,
        pub rt: RgbRenderTarget,
        pub zs: Option<ZsTarget>,
        pub preload: Option<Preload>,
        /// The application triangles it rasterised.
        pub draws: usize,
    }

    const LINEAR_ZS: [&str; 13] = [
        "ZS.Write Format",
        "ZS.Block Format",
        "ZS.MSAA",
        "ZS.Clean Tile Write Enable",
        "ZS.Base",
        "ZS.Row Stride",
        "ZS.Surface Stride",
        "S.Write Format",
        "S.Block Format",
        "S.MSAA",
        "S.Base",
        "S.Row Stride",
        "S.Surface Stride",
    ];

    /// Run the stream at `gpu` as the GPU would, for what the tests read:
    /// the register moves are followed, every RUN_IDVS takes the next
    /// pending triangle, every RUN_FRAGMENT reads its framebuffer descriptor
    /// and executes the frame ([`run_frame`]).
    fn execute(gpu: u64, len: usize) {
        let raw = read_vec(gpu, len);
        let mut regs = [0u32; 128];
        let mut tris = Vec::new();
        for w in raw.as_chunks::<8>().0 {
            match Instr::decode(u64::from_le_bytes(*w)) {
                Ok(Instr::Move48(m)) => {
                    regs[m.destination as usize] = m.immediate as u32;
                    regs[m.destination as usize + 1] = (m.immediate >> 32) as u32;
                }
                Ok(Instr::Move32(m)) => regs[m.destination as usize] = m.immediate,
                Ok(Instr::RunIdvs(_)) => {
                    let mut p = PENDING.lock().unwrap();
                    assert!(!p.is_empty(), "a RUN_IDVS with no application draw pending");
                    tris.push(p.remove(0));
                }
                Ok(Instr::RunFragment(_)) => {
                    let ptr = u64::from(regs[40]) | u64::from(regs[41]) << 32;
                    let f = run_frame(ptr, &core::mem::take(&mut tris));
                    FRAMES.lock().unwrap().push(f);
                }
                _ => {}
            }
        }
    }

    fn read_vec(gpu: u64, len: usize) -> Vec<u8> {
        // SAFETY: a live buffer holds the range (`cpu_of`).
        unsafe { core::slice::from_raw_parts(cpu_of(gpu, len), len).to_vec() }
    }

    /// One frame of the framebuffer pointer `ptr`: its tile buffer started,
    /// drawn and written back as the descriptors say.
    fn run_frame(ptr: u64, tris: &[[f32; 12]]) -> Frame {
        let fbd = ptr & !0x3f;
        let params = FramebufferParameters::unpack(&read::<64>(fbd)).unwrap();
        let ext = fbd + Framebuffer::LENGTH as u64;
        let zs = (ptr & 2 != 0).then(|| ZsCrcExtension::unpack_as(&read::<64>(ext), &LINEAR_ZS).unwrap().zs);
        let rt_at = ext + if ptr & 2 != 0 { 64 } else { 0 };
        let rt = RgbRenderTarget::unpack_as(&read::<64>(rt_at), &["Writeback Format"]).unwrap();
        let preload = (params.pre_frame_0 != PrePostFrameShaderMode::NEVER).then(|| {
            let dcd = Draw::unpack(&read::<{ Draw::LENGTH }>(params.frame_shader_dcds)).unwrap();
            let table = dcd.shader.resources & !63;
            let entry = Resource::unpack(&read::<{ Resource::LENGTH }>(table + 3 * Resource::LENGTH as u64)).unwrap();
            let tex = Texture::unpack(&read::<{ Texture::LENGTH }>(entry.address)).unwrap();
            let plane = GenericPlane::unpack(&read::<32>(tex.surfaces)).unwrap();
            Preload { flags: dcd.flags_0, samples: tex.sample_count, plane }
        });
        let (w, h, n) = (params.width, params.height, params.sample_count as usize);
        assert!(n == 1 || n == 4, "{n} samples");
        /* the tile buffer: its start */
        let mut tile: Vec<[[u8; 4]; 4]> = (0..w * h)
            .map(|i| {
                let (x, y) = (u64::from(i % w), u64::from(i / w));
                match preload {
                    Some(p) => {
                        let at = |s: u64| {
                            pixel(
                                p.plane.pointer
                                    + s * u64::from(p.plane.slice_stride)
                                    + y * u64::from(p.plane.row_stride)
                                    + 4 * x,
                            )
                        };
                        if p.samples == 4 {
                            [at(0), at(1), at(2), at(3)]
                        } else {
                            [at(0); 4]
                        }
                    }
                    None => [rt.clear.color_0.to_le_bytes(); 4],
                }
            })
            .collect();
        /* the draws: a covered sample takes the shader's constant */
        for tri in tris {
            for y in 0..h {
                for x in 0..w {
                    for (s, sample) in tile[(y * w + x) as usize][..n].iter_mut().enumerate() {
                        let (cx, cy) = if n == 4 {
                            sample_at(x, y, s, w, h)
                        } else {
                            ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0, (y as f32 + 0.5) / h as f32 * 2.0 - 1.0)
                        };
                        if covered(tri, cx, cy) {
                            *sample = bytes(COLOUR);
                        }
                    }
                }
            }
        }
        /* the writeback */
        let b = rt.writeback_buffer;
        for y in 0..h {
            for x in 0..w {
                let px = tile[(y * w + x) as usize];
                let at = |s: u64| b.base + s * u64::from(b.surface_stride) + u64::from(y * b.row_stride + 4 * x);
                match rt.writeback_msaa {
                    Msaa::LAYERED => {
                        assert_eq!(n, 4);
                        for (s, v) in px.iter().enumerate() {
                            put(at(s as u64), *v);
                        }
                    }
                    Msaa::AVERAGE => {
                        let mut v = [0u8; 4];
                        for (c, o) in v.iter_mut().enumerate() {
                            let sum: u32 = px[..n].iter().map(|s| u32::from(s[c])).sum();
                            *o = ((sum + n as u32 / 2) / n as u32) as u8;
                        }
                        put(at(0), v);
                    }
                    Msaa::SINGLE => put(at(0), px[0]),
                    m => panic!("writeback MSAA {m:?}"),
                }
            }
        }
        Frame { params, rt, zs, preload, draws: tris.len() }
    }

    unsafe extern "C" fn w_draw(d: *const GlesDraw) -> i32 {
        let d = &*d;
        let rc = mali_draw(d);
        if rc == 0 && d.index_count != 0 {
            PENDING.lock().unwrap().push(*TRIANGLE.lock().unwrap());
        }
        super::record(d, rc);
        rc
    }

    unsafe extern "C" fn w_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
        DEPTH_ASKED.lock().unwrap().push(id);
        mali_depth_resolve(id, dst, w, h, pitch)
    }

    /// # Safety
    /// The caller holds the file's lock.
    pub unsafe fn install() {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        osal_set_ops(&OSAL);
        // SAFETY: a table of entry points and plain values; the copy is
        // leaked, so it outlives every call through it.
        let mut t = unsafe { core::ptr::read(&yakogl::mali::backend::yakogl_backend_mali) };
        assert_eq!(t.max_samples, 4, "msaa-fbo turns the Mali's 4x framebuffer objects on");
        t.draw = Some(w_draw);
        t.depth_resolve = Some(w_depth_resolve);
        backend::g_backend = Box::leak(Box::new(t));
        assert_eq!(mali_init(), 0);
    }

    /// Forget what earlier tests left: the frames and any triangle.
    pub fn reset() {
        FRAMES.lock().unwrap().clear();
        PENDING.lock().unwrap().clear();
    }

    pub fn frames() -> Vec<Frame> {
        FRAMES.lock().unwrap().clone()
    }
}

fn record(d: &GlesDraw, mali: i32) {
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        gpu: d.target_gpu,
        samples: d.samples,
        depth_id: d.depth_id,
        mali,
    });
}

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static CTX: Mutex<usize> = Mutex::new(0);

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics or leaked; the lock above
        // serialises the tests.
        unsafe {
            gpu::install();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            let c = yakogl::gl::context::gl_context_create(8, 8);
            assert!(!c.is_null());
            *CTX.lock().unwrap() = c as usize;
        }
    });
    set_window_samples(1);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_DEPTH_TEST);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    DEPTH_ASKED.lock().unwrap().clear();
    gpu::reset();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

/// The window's sample count, as an EGL surface of the 4x config sets it
/// (`default_samples`); the lock of `context()` is held by the caller.
fn set_window_samples(n: u8) {
    let c = *CTX.lock().unwrap() as *mut GlContext;
    // SAFETY: the context `context()` made; the tests take turns.
    unsafe { (*c).default_samples = n };
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- framebuffers, the program, the draw ------------------------------- */

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// A bound `w`x`h` framebuffer with an RGBA8 renderbuffer of `samples` on
/// colour 0.
fn framebuffer(samples: GLsizei, w: u32, h: u32) -> Fbo {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, w as GLsizei, h as GLsizei);
        assert_eq!(glGetError(), GL_NO_ERROR, "{samples} samples at {w}x{h}");
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        let buf = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, buf.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{samples} samples at {w}x{h}");
    }
    Fbo { fb, rbs: vec![rb] }
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

/// The current program: one float output, `COLOUR`.
fn program() {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Covers the whole target.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
/// The lower-left half: the diagonal from (w, 0) to (0, h) cuts the pixels it crosses.
static HALF: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
/// The upper-right half, the other side of the same diagonal.
static OTHER_HALF: [f32; 12] = [1.0, 1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0];

/// Draw one triangle and hand back what the back end was given for it.
fn draw(tri: &'static [f32; 12]) -> Seen {
    SEEN.lock().unwrap().clear();
    *TRIANGLE.lock().unwrap() = *tri;
    // SAFETY: `tri` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, tri.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

fn clear_to(rgba: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(rgba[0], rgba[1], rgba[2], rgba[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
}

/// Blit the whole of `read` (`w`x`h`) into a fresh 1x FBO of the same size
/// and return the GL error, and that FBO's `w * h` pixels.
fn resolve(read: &Fbo, w: u32, h: u32) -> (GLenum, Vec<[u8; 4]>) {
    let dst = framebuffer(0, w, h);
    let mut v = vec![[0u8; 4]; (w * h) as usize];
    let (w, h) = (w as GLint, h as GLint);
    // SAFETY: plain GL calls on names this test made; room for w x h RGBA8.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    let e = err();
    // SAFETY: as above.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, dst.fb);
        glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    (e, v)
}

/// The sizes: dEQP's two, 1x1, and one under, on and over a 16 and a 32
/// pixel tile edge (the Mali's tile is 16x16 up to 32x32 by its sample and
/// colour size), square and not.
const SIZES: [(u32, u32); 14] = [
    (119, 131),
    (40, 40),
    (1, 1),
    (15, 15),
    (16, 16),
    (17, 17),
    (31, 31),
    (32, 32),
    (33, 33),
    (15, 17),
    (17, 15),
    (16, 33),
    (33, 16),
    (1, 33),
];

/// Every frame of the kicks so far has the FBO's extent, and every frame
/// that draws or clears at 4x has 4 samples; the last frame is the resolve
/// and averages into the renderbuffer.
fn assert_extent(w: u32, h: u32) {
    let frames = gpu::frames();
    assert!(!frames.is_empty(), "{w}x{h}: no frame reached the GPU");
    for (i, f) in frames.iter().enumerate() {
        assert_eq!((f.params.width, f.params.height), (w, h), "{w}x{h}: frame {i} covers the whole FBO: {f:#?}");
        assert_eq!(f.params.sample_count, 4, "{w}x{h}: frame {i}");
    }
}

/// Run `body` for every size, with a bound 4x FBO and the program.
fn each_size(body: impl Fn(u32, u32, &Fbo)) {
    let _g = context();
    for (w, h) in SIZES {
        gpu::reset();
        let fbo = framebuffer(4, w, h);
        program();
        body(w, h, &fbo);
        drop(fbo);
        while err() != GL_NO_ERROR {}
    }
}

#[test]
fn a_4x_fbo_is_complete_at_every_size() {
    each_size(|w, h, fbo| {
        let mut v = -1;
        // SAFETY: one value; a renderbuffer this test made.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, fbo.rbs[0]);
            glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
            assert_eq!(v, 4, "{w}x{h}");
            let (mut rw, mut rh) = (0, 0);
            glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_WIDTH, &mut rw);
            glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_HEIGHT, &mut rh);
            assert_eq!((rw as u32, rh as u32), (w, h));
        }
    });
}

#[test]
fn a_clear_resolves_to_every_pixel() {
    let c = [0.5, 0.25, 0.75, 1.0];
    each_size(|w, h, fbo| {
        clear_to(c);
        assert_extent(w, h);
        let (e, px) = resolve(fbo, w, h);
        assert_eq!(e, GL_NO_ERROR, "{w}x{h}");
        for (i, p) in px.iter().enumerate() {
            assert_eq!(*p, bytes(c), "{w}x{h}: pixel ({},{})", i as u32 % w, i as u32 / w);
        }
    });
}

#[test]
fn a_covering_draw_resolves_to_every_pixel() {
    each_size(|w, h, fbo| {
        clear_to([0.0; 4]);
        let s = draw(&FULL);
        assert_eq!((s.samples, s.mali), (4, TAKEN), "{w}x{h}");
        assert_extent(w, h);
        let (e, px) = resolve(fbo, w, h);
        assert_eq!(e, GL_NO_ERROR, "{w}x{h}");
        for (i, p) in px.iter().enumerate() {
            assert_eq!(*p, bytes(COLOUR), "{w}x{h}: pixel ({},{})", i as u32 % w, i as u32 / w);
        }
    });
}

/// A half-covered draw: pixels wholly inside the lower-left triangle are
/// the colour, pixels wholly outside black, so the edge tiles (the last
/// column and row of tiles) are resolved as much as the first.
#[test]
fn a_half_covered_draw_resolves_inside_and_outside() {
    each_size(|w, h, fbo| {
        clear_to([0.0; 4]);
        let s = draw(&HALF);
        assert_eq!((s.samples, s.mali), (4, TAKEN), "{w}x{h}");
        assert_extent(w, h);
        let (e, px) = resolve(fbo, w, h);
        assert_eq!(e, GL_NO_ERROR, "{w}x{h}");
        let (fw, fh) = (w as f32, h as f32);
        for y in 0..h {
            for x in 0..w {
                let p = px[(y * w + x) as usize];
                let far = (x as f32 + 1.0) / fw + (y as f32 + 1.0) / fh; // far corner
                let near = x as f32 / fw + y as f32 / fh; // near corner
                if far < 1.0 - 1e-4 {
                    assert_eq!(p, bytes(COLOUR), "{w}x{h}: inside pixel ({x},{y})");
                } else if near > 1.0 + 1e-4 {
                    assert_eq!(p, [0; 4], "{w}x{h}: outside pixel ({x},{y})");
                }
            }
        }
    });
}

/// A redraw in a second kick preloads the first kick's samples at every
/// size: the second draw's other half completes the picture.
#[test]
fn a_second_kick_redraws_over_the_first() {
    each_size(|w, h, fbo| {
        clear_to([0.0; 4]);
        draw(&HALF);
        draw(&OTHER_HALF);
        assert_extent(w, h);
        let (e, px) = resolve(fbo, w, h);
        assert_eq!(e, GL_NO_ERROR, "{w}x{h}");
        let (fw, fh) = (w as f32, h as f32);
        for y in 0..h {
            for x in 0..w {
                let far = (x as f32 + 1.0) / fw + (y as f32 + 1.0) / fh;
                let near = x as f32 / fw + y as f32 / fh;
                if far < 1.0 - 1e-4 || near > 1.0 + 1e-4 {
                    assert_eq!(px[(y * w + x) as usize], bytes(COLOUR), "{w}x{h}: pixel ({x},{y})");
                }
            }
        }
    });
}
