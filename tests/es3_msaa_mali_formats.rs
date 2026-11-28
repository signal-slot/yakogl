// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mali 4x framebuffer objects in each colour-renderable sized format, on the
//! host (mali-msaa-formats-tests).
//!
//! The REAL Mali back end (`yakogl_backend_mali`, `msaa-fbo`: `max_samples`
//! 4) over the mock OSAL and GPU model of tests/es3_msaa_mali.rs (copied
//! here: the model executes the frame descriptors, no shader is run). For
//! each of RGBA8, RGB8, RGB565, RGBA4, RGB5_A1, RGB10_A2 and SRGB8_ALPHA8 the
//! tests pin what the GL core and the Mali do today: the 4x storage keeps 4
//! samples, the FBO is complete, a clear and a draw reach the Mali with
//! `samples` 4, and the frames it emits (one colour target) are inspected:
//! sample count 4, the colour target's pixel format, the AVERAGE resolve
//! into the renderbuffer. The model's pixels are 4 bytes each whatever the
//! format, so a read back is only compared for the 4-byte RGBA8 layout.
//! Integer formats are pinned as today's GL core has them (refused).
//! HOST-ONLY: nothing here is board-verified.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,msaa-fbo --test es3_msaa_mali_formats
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

/* ---- the mock OSAL and the GPU model ---- */

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

/// The error of `glRenderbufferStorageMultisample(.., samples, format, 8, 8)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on colour 0 and,
/// when `depth` is set, a depth attachment of the same count. A storage error
/// comes back as `Err(error)`.
fn framebuffer(samples: GLsizei, depth: bool) -> Result<Fbo, GLenum> {
    framebuffer_in(samples, depth, GL_RGBA8)
}

/// [`framebuffer`] with colour 0 of `colour`.
fn framebuffer_in(samples: GLsizei, depth: bool, colour: GLenum) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    let attachments = [(colour, GL_COLOR_ATTACHMENT0), (GL_DEPTH_COMPONENT24, GL_DEPTH_ATTACHMENT)];
    for (format, point) in attachments.iter().take(if depth { 2 } else { 1 }) {
        let (e, rb) = storage(samples, *format);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, *point, GL_RENDERBUFFER, rb) };
    }
    let buf = [GL_COLOR_ATTACHMENT0];
    // SAFETY: plain GL calls.
    unsafe {
        glDrawBuffers(1, buf.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{samples} samples");
    }
    Ok(fbo)
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
/// The lower-left half: the 8x8 target's pixels with x + y = 7 are cut in
/// two by its diagonal edge (two of the four samples).
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

fn clear_black() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 0.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
}

/// Blit `read`'s colour (and depth when `depth`) into a fresh single-sample
/// framebuffer of the same size and return the GL error and that framebuffer.
fn resolve_blit(read: &Fbo, depth: bool) -> (GLenum, Fbo) {
    resolve_blit_in(read, depth, GL_RGBA8)
}

/// [`resolve_blit`] into a single-sample framebuffer of colour format `colour`.
fn resolve_blit_in(read: &Fbo, depth: bool, colour: GLenum) -> (GLenum, Fbo) {
    let dst = framebuffer_in(0, depth, colour).expect("a single-sample target");
    let mask = GL_COLOR_BUFFER_BIT | if depth { GL_DEPTH_BUFFER_BIT } else { 0 };
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, mask, GL_NEAREST);
    }
    (err(), dst)
}

/// The 8x8 RGBA8 pixels of a framebuffer.
fn read_pixels(fb: GLuint) -> Vec<[u8; 4]> {
    let mut v = vec![[0u8; 4]; 64];
    // SAFETY: room for 8x8 RGBA8; a framebuffer this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn rb_samples(rb: GLuint) -> GLint {
    let mut v = -1;
    // SAFETY: one value; a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    v
}

fn internalformat(format: GLenum, pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, pname, 1, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// The resolved `pixels` of a draw of the lower-left half over black:
/// `COLOUR` exactly where a pixel is wholly inside (`inside`), black where
/// wholly outside, and the mean of the two (two of four samples, +-1) on the
/// diagonal pixels `cut` names.
fn check_halves(pixels: &[[u8; 4]], inside: impl Fn(usize, usize) -> bool, cut: impl Fn(usize, usize) -> bool) {
    let colour = bytes(COLOUR);
    for y in 0..8 {
        for x in 0..8 {
            let p = pixels[y * 8 + x];
            if cut(x, y) {
                for c in 0..4 {
                    let half = i32::from(colour[c]) / 2;
                    assert!((i32::from(p[c]) - half).abs() <= 1, "edge pixel ({x},{y}) {p:?}");
                }
            } else if inside(x, y) {
                assert_eq!(p, colour, "interior pixel ({x},{y})");
            } else {
                assert_eq!(p, [0; 4], "outside pixel ({x},{y})");
            }
        }
    }
}

const FORMATS: [(GLenum, &str); 7] = [
    (GL_RGBA8, "RGBA8"),
    (GL_RGB8, "RGB8"),
    (GL_RGB565, "RGB565"),
    (GL_RGBA4, "RGBA4"),
    (GL_RGB5_A1, "RGB5_A1"),
    (GL_RGB10_A2, "RGB10_A2"),
    (GL_SRGB8_ALPHA8, "SRGB8_ALPHA8"),
];

/// What the GL core and the Mali make of a 4x FBO of each format today: the
/// narrow formats are stored as RGBA8 (`GLES_SURF_RGBA8888`, the frame's
/// `R8G8B8A8`), which the readback quantises to the format's bits.
/// (GL format, name, resolved pixel of the interior, of the edge (7,0) and of
/// the outside (0,7 side: (7,7)), all as read by RGBA/UNSIGNED_BYTE.)
const DRAWN: [(GLenum, &str, [u8; 4], [u8; 4], [u8; 4]); 6] = [
    (GL_RGBA8, "RGBA8", [64, 128, 191, 255], [32, 64, 96, 128], [0, 0, 0, 0]),
    (GL_RGB8, "RGB8", [64, 128, 191, 255], [32, 64, 96, 255], [0, 0, 0, 255]),
    (GL_RGB565, "RGB565", [66, 130, 189, 255], [33, 65, 99, 255], [0, 0, 0, 255]),
    (GL_RGBA4, "RGBA4", [68, 136, 187, 255], [34, 68, 102, 136], [0, 0, 0, 0]),
    (GL_RGB5_A1, "RGB5_A1", [66, 132, 189, 255], [33, 66, 99, 255], [0, 0, 0, 0]),
    (GL_RGB10_A2, "RGB10_A2", [64, 128, 191, 255], [32, 64, 96, 128], [0, 0, 0, 0]),
];

/// The 4x storage keeps 4 samples, and the FBO is complete, for every format
/// (`framebuffer_in` asserts the completeness) and the sample queries answer 4.
#[test]
fn four_x_storage_keeps_four_samples_in_every_format() {
    let _g = context();
    for (f, name) in FORMATS {
        assert_eq!(internalformat(f, GL_SAMPLES), 4, "{name}");
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 1, "{name}");
        let fbo = framebuffer_in(4, false, f).unwrap_or_else(|e| panic!("{name}: storage error {e:#x}"));
        assert_eq!(rb_samples(fbo.rbs[0]), 4, "{name}");
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb) };
        assert_eq!(geti(GL_SAMPLES), 4, "{name}");
        assert_eq!(geti(GL_SAMPLE_BUFFERS), 1, "{name}");
        /* storage with 0 samples stays plain single-sample storage */
        let one = framebuffer_in(0, false, f).unwrap();
        assert_eq!(geti(GL_SAMPLES), 0, "{name}: 0 samples is the single-sample storage");
        assert_eq!(geti(GL_SAMPLE_BUFFERS), 0, "{name}");
        drop(one);
    }
}

/// A clear and a half-covered draw into a 4x FBO of each format reach the
/// Mali with one colour target of `GLES_SURF_RGBA8888` (today's stored format
/// of the narrow ones) and `samples` 4, and the frames are the ones of the
/// RGBA8 case: sample count 4, the writeback format R8G8B8A8, the draw frame
/// keeping its samples (LAYERED) and the resolve frame averaging them into
/// the renderbuffer (AVERAGE).
#[test]
fn clear_and_draw_emit_four_sample_frames_in_every_format() {
    use yakogl::mali::genxml::v10::{ColorFormat, Msaa};
    let _g = context();
    for (f, name, ..) in DRAWN {
        gpu::reset();
        let fbo = framebuffer_in(4, false, f).expect("4 samples are legal storage");
        program();
        clear_black();
        let s = draw(&HALF);
        assert_eq!((s.samples, s.mali), (4, TAKEN), "{name}");
        assert_eq!(s.targets, vec![(s.gpu, GLES_SURF_RGBA8888)], "{name}: one colour target");
        let frames = gpu::frames();
        assert_eq!(frames.len(), 4, "{name}: the clear's kick and the draw's, each with its resolve");
        for (i, fr) in frames.iter().enumerate() {
            assert_eq!(fr.params.sample_count, 4, "{name} frame {i}");
            assert_eq!(fr.rt.writeback_format, ColorFormat::R8G8B8A8, "{name} frame {i}");
            if i % 2 == 0 {
                assert_eq!(fr.rt.writeback_msaa, Msaa::LAYERED, "{name} frame {i} keeps the samples");
                assert_ne!(fr.rt.writeback_buffer.base, s.gpu, "{name}: kept beside the renderbuffer");
            } else {
                assert_eq!(fr.rt.writeback_msaa, Msaa::AVERAGE, "{name} frame {i} resolves");
                assert_eq!(fr.rt.writeback_buffer.base, s.gpu, "{name}: into the renderbuffer");
                assert_eq!(fr.draws, 0, "{name}");
            }
        }
        assert_eq!(frames[2].draws, 1, "{name}: the application triangle");
        drop(fbo);
    }
}

/// The resolve blit into a single-sample FBO of the same format reads back as
/// today: the interior the colour as the format quantises it, the outside the
/// clear, the diagonal the mean of the two (HOST-ONLY: the model's AVERAGE).
#[test]
fn resolve_blit_reads_back_in_every_format() {
    let _g = context();
    for (f, name, inside, edge, outside) in DRAWN {
        let fbo = framebuffer_in(4, false, f).expect("4 samples are legal storage");
        program();
        clear_black();
        draw(&HALF);
        let (e, dst) = resolve_blit_in(&fbo, false, f);
        assert_eq!(e, GL_NO_ERROR, "{name}");
        let p = read_pixels(dst.fb);
        assert_eq!(p[0], inside, "{name} interior");
        assert_eq!(p[7], edge, "{name} edge (7,0)");
        assert_eq!(p[63], outside, "{name} outside");
        assert_eq!(p[3 * 8 + 2], inside, "{name} interior (2,3)");
        assert_eq!(p[4 * 8 + 5], outside, "{name} outside (5,4)");
        drop((dst, fbo));
    }
}

/// A blit between formats is refused whatever the count: the 4x RGBA8 FBO
/// does not resolve into an RGB565 one.
#[test]
fn a_resolve_into_another_format_is_invalid_operation() {
    let _g = context();
    let fbo = framebuffer_in(4, false, GL_RGBA8).unwrap();
    program();
    clear_black();
    draw(&HALF);
    let (e, _dst) = resolve_blit_in(&fbo, false, GL_RGB565);
    assert_eq!(e, GL_INVALID_OPERATION);
}

/// SRGB8_ALPHA8: the storage and the FBO are fine at 4 samples, but the Mali
/// has no target format for it (`surface_format`, NOT_BUILT_TARGET_FORMAT), so
/// a draw is refused: GL_INVALID_OPERATION from the GL core's
/// `refuse_undrawable_targets`, nothing reaches the queue.
#[test]
fn srgb8_alpha8_4x_draw_is_refused_today() {
    let _g = context();
    let fbo = framebuffer_in(4, false, GL_SRGB8_ALPHA8).expect("storage and completeness");
    assert_eq!(rb_samples(fbo.rbs[0]), 4);
    program();
    // SAFETY: plain GL calls; HALF is static.
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 0.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, HALF.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_INVALID_OPERATION);
    assert!(gpu::frames().iter().all(|f| f.draws == 0), "no application draw was kicked");
}

/// What a 4x SRGB8_ALPHA8 FBO should do: draw, and resolve into a 1x one.
/// Owner: the Mali back end (src/mali/backend.rs `surface_format`, and
/// src/mali/frame.rs `TargetFormat`: an sRGB writeback format, the blend and
/// preload conversions), plus the GL core's `GLES_SURF_SRGB8_A8` handoff.
#[test]
#[ignore = "Mali: no TargetFormat for GLES_SURF_SRGB8_A8, the draw is refused (src/mali/backend.rs surface_format)"]
fn srgb8_alpha8_4x_draws_and_resolves() {
    let _g = context();
    let fbo = framebuffer_in(4, false, GL_SRGB8_ALPHA8).expect("storage and completeness");
    program();
    clear_black();
    let s = draw(&HALF);
    assert_eq!((s.samples, s.mali), (4, TAKEN));
    let (e, dst) = resolve_blit_in(&fbo, false, GL_SRGB8_ALPHA8);
    assert_eq!(e, GL_NO_ERROR);
    let p = read_pixels(dst.fb);
    assert_eq!(p[63], [0; 4], "outside");
    assert_ne!(p[0], [0; 4], "interior");
}

/// Integer formats at 4 samples, as the GL core has them today (the spec's
/// rule: INVALID_OPERATION for any count, and no sample counts reported).
#[test]
fn integer_formats_are_refused_at_every_count() {
    let _g = context();
    for f in [GL_R8I, GL_RG16UI, GL_RGBA8I, GL_RGBA8UI, GL_RGBA32I, GL_R32UI, GL_RGB10_A2UI] {
        assert_eq!(storage(4, f).0, GL_INVALID_OPERATION, "{f:#x} at 4");
        assert_eq!(storage(1, f).0, GL_INVALID_OPERATION, "{f:#x} at 1");
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
    }
}
