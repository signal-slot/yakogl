// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x multisampling of framebuffer objects on the Mali (Valhall G610), through
//! the GL API, on the host (msaa-pin-mali-file, mali-msaa-fbo).
//!
//! docs/notes/es3-msaa-renderbuffers.md (b), (c4) and (f4)-(f5). The Mali
//! keeps 4x framebuffer objects only with the `msaa-fbo` cargo feature: its
//! table's `max_samples` is 4 then and 0 without (src/mali/backend.rs),
//! because a hardware-facing change ships opt-in and none of this is
//! board-verified. The file builds one of two rigs, one per feature set:
//!
//! * Without `msaa-fbo`: the GL core is driven into a stand-in back end with
//!   `max_samples` 0 whose draw entry first runs the Mali's own `mali_draw`
//!   (its `check` and program checks; no Mali OSAL, so a draw it takes ends
//!   at `-ENODEV`) and then models the contract of note (b) 4-7. The pins
//!   hold the capability off: `GL_MAX_SAMPLES` 1,
//!   `glRenderbufferStorageMultisample` `GL_INVALID_OPERATION` for 2 or more
//!   samples, one sample plain storage, an FBO draw with `GlesDraw::samples` 1.
//! * With `msaa-fbo`: the REAL Mali back end (`yakogl_backend_mali`, its draw
//!   and `depth_resolve` wrapped to record) over a mock OSAL whose submit
//!   runs a GPU model ([`gpu`]). The model reads every frame of a kick from
//!   the command stream as the GPU would (the framebuffer descriptor at each
//!   RUN_FRAGMENT, its render target, the colour preload's DCD and texture)
//!   and executes what the descriptors say on the mock's memory: a tile
//!   buffer of the frame's sample count, started from the RT clear words or
//!   from the preload texture (4 samples: one plane a sample, its slice
//!   stride; 1 sample: that value in every sample), the application's
//!   triangles rasterised at the rotated 4x grid, and the writeback by its
//!   `Writeback MSAA` (LAYERED: one plane a sample at the surface stride;
//!   AVERAGE: the mean into the base; SINGLE: sample 0). No shader is run:
//!   the colour of a covered sample is the test fragment shader's constant.
//!   So the pixels the tests read back after `glBlitFramebuffer` are the
//!   model's execution of the Mali's descriptors: they check where the Mali
//!   keeps and finds the samples, which frame preloads what, and that the
//!   resolve lands in the renderbuffer the GL core copies; the tests also
//!   assert the descriptor words themselves. HOST-ONLY: the G610's own
//!   AVERAGE rounding and which depth sample its Z/S writeback keeps are
//!   BOARD-UNVERIFIED (note (f4)).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test es3_msaa_mali
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,msaa-fbo --test es3_msaa_mali
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]
/* each rig leaves the other's helpers unused */
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
#[cfg(not(feature = "msaa-fbo"))]
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
#[cfg(not(feature = "msaa-fbo"))]
use yakogl::util::ENODEV;

/// What `mali_draw` answers for a draw it takes: without `msaa-fbo` there is
/// no Mali OSAL (`-ENODEV`, no queue on the host); with it, the mock's queue.
#[cfg(not(feature = "msaa-fbo"))]
const TAKEN: i32 = -ENODEV;
#[cfg(feature = "msaa-fbo")]
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

#[cfg(not(feature = "msaa-fbo"))]
mod standin {
    use super::*;
    use yakogl::backend::{GlesHeap, YakoglBackend};

    /// Freed blocks by size: `gpu_free` hands one back to the next
    /// `gpu_alloc` of its size, zeroed.
    static FREED: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

    unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        let recycled = {
            let mut f = FREED.lock().unwrap();
            f.iter().position(|(s, _)| *s == size).map(|i| f.remove(i).1)
        };
        let p = match recycled {
            Some(p) => {
                core::ptr::write_bytes(p as *mut u8, 0, size);
                p as *mut u8
            }
            None => Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr(),
        };
        *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
        0
    }
    unsafe extern "C" fn m_gpu_free(mem: *mut OsalGpuMem) -> i32 {
        FREED.lock().unwrap().push(((*mem).size, (*mem).cpu as usize));
        0
    }
    unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
    unsafe extern "C" fn b_init() -> i32 {
        0
    }
    unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
        0x1000_0000
    }
    unsafe extern "C" fn b_ok() -> i32 {
        0
    }

    /* The stand-in's model of note (b) 4-7: four samples a pixel of a 4x
     * target, kept per (gpu, w, h) until `attachment_gone`, loaded from the
     * resolved memory when nothing is kept, averaged into the target after
     * every draw. */
    type Pixel = [[u8; 4]; 4];
    /// (gpu, w, h) and the target's pixels.
    type Kept = ((u64, u32, u32), Vec<Pixel>);
    pub static STORE: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

    unsafe fn model(d: &GlesDraw) {
        if d.samples != 4 || d.target_gpu == 0 {
            return;
        }
        let (w, h, pitch) = (d.target_w, d.target_h, backend::draw_pitch(d) as usize);
        let mem = core::slice::from_raw_parts_mut(d.target_gpu as *mut u8, pitch * h as usize * 4);
        let key = (d.target_gpu, w, h);
        let mut store = STORE.lock().unwrap();
        let at = match store.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                /* nothing kept: every sample starts as the resolved colour */
                let px = (0..w * h)
                    .map(|i| {
                        let (x, y) = ((i % w) as usize, (i / w) as usize);
                        let o = (y * pitch + x) * 4;
                        [[mem[o], mem[o + 1], mem[o + 2], mem[o + 3]]; 4]
                    })
                    .collect();
                store.push((key, px));
                store.len() - 1
            }
        };
        let px = &mut store[at].1;
        if d.do_clear && !d.clear_quad {
            px.fill([bytes(d.clear_rgba); 4]);
        }
        if d.index_count != 0 {
            let tri = *TRIANGLE.lock().unwrap();
            for y in 0..h {
                for x in 0..w {
                    let row = if d.flip_y { h - 1 - y } else { y };
                    for (s, sample) in px[(row * w + x) as usize].iter_mut().enumerate() {
                        let (cx, cy) = sample_at(x, y, s, w, h);
                        if covered(&tri, cx, cy) {
                            *sample = bytes(COLOUR);
                        }
                    }
                }
            }
        }
        for y in 0..h as usize {
            for x in 0..w as usize {
                let p = px[y * w as usize + x];
                for c in 0..4 {
                    let sum: u32 = p.iter().map(|s| u32::from(s[c])).sum();
                    mem[(y * pitch + x) * 4 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
    }

    unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
        let d = &*d;
        let mali = mali_draw(d);
        super::record(d, mali);
        model(d);
        0
    }

    unsafe extern "C" fn b_gone(_id: u32) {
        /* the model keys by address: whatever it keeps is dropped, the
         * contract of note (b) 6 for the one object the GL core names */
        STORE.lock().unwrap().clear();
    }

    /// Sample 0 of the triangle's depth (0.5 where covered, else 1.0), for
    /// the note's "sample 0 of the pixel" (b) 8.
    unsafe extern "C" fn b_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
        DEPTH_ASKED.lock().unwrap().push(id);
        let tri = *TRIANGLE.lock().unwrap();
        for y in 0..h {
            for x in 0..w {
                let (cx, cy) = sample_at(x, y, 0, w, h);
                *dst.add((y * pitch + x) as usize) = if covered(&tri, cx, cy) { 0.5 } else { 1.0 };
            }
        }
        0
    }

    static OSAL: OsalOps = OsalOps {
        init: None,
        shutdown: None,
        gpu_alloc: Some(m_gpu_alloc),
        gpu_free: Some(m_gpu_free),
        mmu_map: None,
        map_phys: None,
        gpu_write: None,
        gpu_read: None,
        cache_flush: Some(m_cache),
        cache_invd: Some(m_cache),
        submit: None,
        wait: None,
        completed: None,
        reg_read: None,
        reg_write: None,
        query: None,
        log: None,
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    };

    static BACKEND: YakoglBackend = YakoglBackend {
        name: c"es3-msaa-mali".as_ptr() as *const c_char,
        native_attrib_formats: 0,
        rt_rgba8888_is_bgra: false,
        mip_level_align: 0,
        compressed_blocks_in_rows: false,
        native_layered_textures: false,
        front_facing_shader_variant: false,
        etc1_decoded_at_upload: false,
        init: Some(b_init),
        shutdown: None,
        draw: Some(b_draw),
        flush: Some(b_ok),
        finish: Some(b_ok),
        finish_previous: None,
        kick_id: None,
        kick_pending: None,
        heap_base: Some(b_heap_base),
        counters: None,
        texture_released: None,
        surface_swapped: None,
        texture_uploaded: None,
        max_instance_divisor: 1,
        wait_kick: None,
        samples_texel_copies: false,
        attachment_gone: Some(b_gone),
        depth_resolve: Some(b_depth_resolve),
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: None,
        return_mipmap_chain: None,
        linear_row_align: 0,
        uniforms_host_only: false,
        ds_write: None,
        stencil_resolve: None,
        max_samples: 0,
    };

    /// # Safety
    /// The caller holds the file's lock.
    pub unsafe fn install() {
        backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
        osal_set_ops(&OSAL);
        backend::g_backend = &BACKEND;
    }
}

/* ---- with msaa-fbo: the real Mali over a mock OSAL and a GPU model ------- */

#[cfg(feature = "msaa-fbo")]
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
            #[cfg(not(feature = "msaa-fbo"))]
            standin::install();
            #[cfg(feature = "msaa-fbo")]
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
    #[cfg(not(feature = "msaa-fbo"))]
    standin::STORE.lock().unwrap().clear();
    #[cfg(feature = "msaa-fbo")]
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
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    let attachments = [(GL_RGBA8, GL_COLOR_ATTACHMENT0), (GL_DEPTH_COMPONENT24, GL_DEPTH_ATTACHMENT)];
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
    let dst = framebuffer(0, depth).expect("a single-sample target");
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

/* ---- both feature sets ---------------------------------------------------- */

/// The Mali's table: `max_samples` 4 with `msaa-fbo`, 0 without (note (a),
/// (f5): the capability stays off until the host test passes, and ships
/// opt-in).
#[test]
fn the_mali_table_gates_max_samples_on_msaa_fbo() {
    let want = if cfg!(feature = "msaa-fbo") { 4 } else { 0 };
    assert_eq!(yakogl::mali::backend::yakogl_backend_mali.max_samples, want);
}

/// An integer format stays refused at any count: the spec's rule, not a
/// limit, so it holds with the capability on too.
#[test]
fn integer_formats_stay_refused() {
    let _g = context();
    for f in [GL_R8I, GL_RGBA8UI, GL_RGBA32UI] {
        assert_eq!(storage(1, f).0, GL_INVALID_OPERATION, "{f:#x}");
        assert_eq!(storage(4, f).0, GL_INVALID_OPERATION, "{f:#x}");
    }
}

/// A blit *into* a multisample framebuffer is refused by the spec.
#[test]
fn a_blit_into_a_multisample_framebuffer_is_invalid_operation() {
    let _g = context();
    let ms = framebuffer(1, false).unwrap();
    let single = framebuffer(0, false).unwrap();
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, single.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, ms.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_INVALID_OPERATION);
}

/* ---- without msaa-fbo: the capability off --------------------------------- */

/// `GL_MAX_SAMPLES` is 1 (note (b) 1: `max(max_samples, 1)`).
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn max_samples_is_one_without_msaa_fbo() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 1);
}

/// `glGetInternalformativ`: one sample count, 1, for a renderable format; no
/// counts for an integer one (ES 3.0 4.4.2.1).
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn internalformat_counts_without_msaa_fbo() {
    let _g = context();
    for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB565, GL_RGB8] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 1, "{f:#x}");
        assert_eq!(internalformat(f, GL_SAMPLES), 1, "{f:#x}");
    }
    for f in [GL_R8I, GL_RGBA32I] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
    }
}

/// GL_SAMPLES and GL_SAMPLE_BUFFERS of the window and of framebuffers of
/// zero-sample and one-sample storage, and the renderbuffer's own count.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn renderbuffer_samples_read_back_what_is_stored() {
    let _g = context();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0), "the window of the 8x8 test context");
    let fbo = framebuffer(0, false).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
    assert_eq!(rb_samples(fbo.rbs[0]), 0);
    drop(fbo);
    /* one sample is plain storage "with sample buffers" */
    let fbo = framebuffer(1, false).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 1));
    assert_eq!(rb_samples(fbo.rbs[0]), 1);
}

/// 4 (and 2) samples: `GL_INVALID_OPERATION`, no storage, the count being
/// past `GL_MAX_SAMPLES`; a negative count is `GL_INVALID_VALUE` first.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn four_samples_is_invalid_operation() {
    let _g = context();
    assert_eq!(storage(4, GL_RGBA8).0, GL_INVALID_OPERATION);
    assert_eq!(storage(2, GL_RGBA8).0, GL_INVALID_OPERATION);
    assert_eq!(storage(4, GL_DEPTH_COMPONENT24).0, GL_INVALID_OPERATION);
    assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
    assert_eq!(framebuffer(4, false).err(), Some(GL_INVALID_OPERATION));
}

/// One sample: a draw into a 1-sample FBO reaches the Mali with
/// `GlesDraw::samples == 1` (not the window's count), which it takes (no
/// refusal; with no queue on the host it ends at `-ENODEV`), and a resolve
/// blit into a single-sample framebuffer is no error.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn a_one_sample_fbo_draws_with_samples_one() {
    let _g = context();
    set_window_samples(4);
    let fbo = framebuffer(1, false).unwrap();
    program();
    let s = draw(&FULL);
    assert_eq!(s.samples, 1, "an FBO is single-sampled whatever the window is");
    assert_eq!(s.targets.len(), 1);
    assert_eq!(s.mali, TAKEN, "taken by the Mali's checks");
    assert_eq!(resolve_blit(&fbo, false).0, GL_NO_ERROR);
}

/// The 4x window: with the config's 4 samples the draw reaches the Mali with
/// `samples == 4` and passes its checks (`NOT_BUILT_SAMPLES` is a later
/// check, in the frame builder, which the host path cannot reach: the queue
/// comes first). The window's 4x draw itself is
/// `src/mali/frame_msaa_tests.rs`'s.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn the_4x_window_draw_reaches_the_mali_with_samples_four() {
    let _g = context();
    set_window_samples(4);
    program();
    let s = draw(&FULL);
    assert_eq!(s.samples, 4);
    assert_eq!(s.mali, TAKEN);
}

/* ---- with msaa-fbo: the Mali's 4x framebuffer objects -------------------- */

/// 4 once the Mali's table says so (note (b) 1).
#[cfg(feature = "msaa-fbo")]
#[test]
fn max_samples_is_four() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
    assert_eq!(internalformat(GL_RGBA8, GL_SAMPLES), 4);
    assert_eq!(internalformat(GL_RGBA8, GL_NUM_SAMPLE_COUNTS), 1);
    assert_eq!(internalformat(GL_RGBA32I, GL_NUM_SAMPLE_COUNTS), 0);
    assert_eq!(storage(8, GL_RGBA8).0, GL_INVALID_OPERATION, "past GL_MAX_SAMPLES");
    assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
}

/// The last two frames, those a 4x draw into the renderbuffer at `rb` makes
/// in one kick: the
/// draw's frame writing its samples into the sample store (Writeback MSAA
/// LAYERED, one plane a sample of the renderbuffer's rows), then the resolve
/// frame reading them back each sample its own (a 4-sample texture over the
/// store, the DCD multisampled and run per sample, pre frame 0 ALWAYS) and
/// averaging them into the renderbuffer (AVERAGE at `rb`). Returns the
/// store's address.
#[cfg(feature = "msaa-fbo")]
fn assert_kept_and_resolved(frames: &[gpu::Frame], rb: u64) -> u64 {
    use yakogl::mali::genxml::v10::{Msaa, PrePostFrameShaderMode};
    use yakogl::msaa_hw::mali as hw;
    let [.., draw, resolve] = frames else { panic!("a draw frame and its resolve last: {frames:#?}") };
    let row = draw.rt.writeback_buffer.row_stride;
    let plane = hw::store_surface_stride(row, 8).expect("a 64-byte row");
    assert_eq!((row, plane), (64, 512), "8 RGBA8 texels on 64 bytes, 8 rows a plane");
    assert_eq!(draw.params.sample_count, 4);
    assert_eq!(
        (draw.params.sample_count, draw.params.sample_pattern.0),
        (4, hw::sample_words(4).unwrap().1),
        "the rotated 4x grid"
    );
    assert_eq!(draw.rt.writeback_msaa.0, hw::ResolveMode::Layered.msaa());
    let store = draw.rt.writeback_buffer.base;
    assert_ne!(store, rb, "the samples are kept beside the renderbuffer, not in it");
    assert_eq!(draw.rt.writeback_buffer.surface_stride, plane);
    assert_eq!(resolve.draws, 0);
    assert_eq!(resolve.params.pre_frame_0, PrePostFrameShaderMode::ALWAYS);
    let p = resolve.preload.expect("the resolve reads the samples back");
    assert_eq!((p.samples, p.plane.pointer, p.plane.slice_stride, p.plane.size), (4, store, plane, 4 * plane));
    assert!(p.flags.multisample_enable && p.flags.evaluate_per_sample);
    assert_eq!(resolve.rt.writeback_msaa, Msaa::AVERAGE);
    assert_eq!(resolve.rt.writeback_msaa.0, hw::resolve_mode(hw::FormatClass::Unorm).msaa());
    assert_eq!((resolve.rt.writeback_buffer.base, resolve.rt.writeback_buffer.surface_stride), (rb, 0));
    store
}

/// A 4x RGBA8 FBO drawn with a half-covered triangle: the storage is legal,
/// the draw reaches the Mali with `samples == 4`, which keeps its samples in
/// a slot of the sample store and resolves them into the renderbuffer in
/// the same kick (the words, [`assert_kept_and_resolved`]); the resolve blit
/// then copies that into a single-sample FBO, whose pixels (the GPU model's
/// execution of the words, [`gpu`]) are the half-covered edge: the colour
/// inside, black outside, half of it on the diagonal.
#[cfg(feature = "msaa-fbo")]
#[test]
fn rgba8_4x_fbo_half_covered_edge() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    assert_eq!(rb_samples(fbo.rbs[0]), 4);
    assert_eq!(geti(GL_SAMPLES), 4);
    program();
    clear_black();
    let s = draw(&HALF);
    assert_eq!((s.samples, s.mali), (4, TAKEN));
    /* glFinish after the clear submitted it alone: its kick keeps the
     * cleared samples and resolves them, and the draw's frame then starts
     * from those samples */
    let frames = gpu::frames();
    assert_eq!(frames.len(), 4, "the clear's kick and the draw's, each with its resolve: {frames:#?}");
    let store = assert_kept_and_resolved(&frames[..2], s.gpu);
    assert_eq!(assert_kept_and_resolved(&frames, s.gpu), store, "one slot for the renderbuffer");
    assert!(frames[0].preload.is_none() && frames[0].draws == 0, "a frame from a whole clear loads nothing");
    assert_eq!(frames[2].preload.map(|p| (p.samples, p.plane.pointer)), Some((4, store)));
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    check_halves(&read_pixels(dst.fb), |x, y| x + y < 7, |x, y| x + y == 7);
}

/// The resolve blit: edge pixels are averaged, the interior is exact.
#[cfg(feature = "msaa-fbo")]
#[test]
fn resolve_blit_averages_the_edge_and_keeps_the_interior() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    program();
    clear_black();
    draw(&HALF);
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    let p = read_pixels(dst.fb);
    let colour = bytes(COLOUR);
    assert_eq!(p[0], colour, "interior");
    assert_eq!(p[63], [0; 4], "outside");
    for c in 0..3 {
        assert!(p[7][c] != 0 && p[7][c] != colour[c], "the diagonal pixel (7,0) is a mean: {:?}", p[7]);
    }
}

/// A redraw in a later kick preloads the kept samples: the second frame's
/// colour preload reads the slot's four planes, each sample its own, so the
/// edge of the first kick survives (two of four samples covered stays half,
/// where a reload of the resolved colour into all four would make it three
/// quarters).
#[cfg(feature = "msaa-fbo")]
#[test]
fn the_edge_of_an_earlier_job_survives_a_later_one() {
    use yakogl::mali::genxml::v10::PrePostFrameShaderMode;
    use yakogl::msaa_hw::mali as hw;
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    program();
    clear_black();
    let first = draw(&HALF);
    let store = assert_kept_and_resolved(&gpu::frames(), first.gpu);
    gpu::reset();
    let again = draw(&HALF);
    assert_eq!(again.gpu, first.gpu);
    let frames = gpu::frames();
    assert_eq!(assert_kept_and_resolved(&frames, again.gpu), store, "the same slot");
    let p = frames[0].preload.expect("the redraw starts from the kept samples");
    let want = hw::colour_preload(hw::PreloadSource::SampleStore, 64, 8, false).unwrap();
    assert_eq!(frames[0].params.pre_frame_0.0, want.pre_frame_mode);
    assert_eq!(frames[0].params.pre_frame_0, PrePostFrameShaderMode::INTERSECT);
    assert_eq!(
        (p.samples, p.plane.pointer, p.plane.slice_stride, p.plane.size),
        (4, store, want.slice_stride, want.plane_size)
    );
    assert_eq!(
        (p.flags.multisample_enable, p.flags.evaluate_per_sample),
        (want.multisample_enable, want.evaluate_per_sample)
    );
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    check_halves(&read_pixels(dst.fb), |x, y| x + y < 7, |x, y| x + y == 7);
}

/// A 4x depth attachment: the frame keeps its depth in the single-sampled
/// D24S8 plane (one value a pixel, ZS.MSAA SINGLE: sample 0 is written back,
/// as Mesa's newer path does, `00b3776d`; no surface stride), and the blit's
/// depth goes through `depth_resolve`, which reads that plane. That the G610
/// writes sample 0 under SINGLE is BOARD-UNVERIFIED (note (f4)); the model
/// does not execute depth, so the values are not checked here.
///
/// The destination's depth is a texture: the Mali has no `ds_write` entry,
/// so a depth blit into a renderbuffer is `GL_INVALID_OPERATION` at any
/// sample count (pinned below, for 1 sample too), a gap of its own.
#[cfg(feature = "msaa-fbo")]
#[test]
fn depth_resolve_reads_one_sample_a_pixel() {
    use yakogl::gl::texture::*;
    use yakogl::mali::genxml::v10::Msaa;
    let _g = context();
    let fbo = framebuffer(4, true).expect("4 samples are legal storage");
    program();
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_DEPTH_TEST);
        glClearDepthf(1.0);
        glClearColor(0.0, 0.0, 0.0, 0.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    let s = draw(&HALF);
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_DEPTH_TEST) };
    assert_eq!((s.samples, s.mali), (4, TAKEN));
    assert_ne!(s.depth_id, 0);
    let frames = gpu::frames();
    let zs = frames[frames.len() - 2].zs.expect("the 4x frame writes its depth back");
    assert_eq!(
        (zs.msaa, zs.surface_stride, zs.row_stride),
        (Msaa::SINGLE, 0, 4 * 8),
        "one D24S8 word a pixel, sample 0"
    );
    /* into a renderbuffer: refused, as from one sample */
    assert_eq!(resolve_blit(&fbo, true).0, GL_INVALID_OPERATION, "no ds_write on the Mali");
    let one = framebuffer(0, true).unwrap();
    assert_eq!(resolve_blit(&one, true).0, GL_INVALID_OPERATION, "the same at one sample");
    drop(one);
    DEPTH_ASKED.lock().unwrap().clear();
    /* into a depth texture: read through depth_resolve */
    let dst = framebuffer(0, false).unwrap();
    let mut tex = 0;
    // SAFETY: plain GL calls on names this test made.
    let e = unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_DEPTH_COMPONENT24 as GLint,
            8,
            8,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fbo.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
        let e = glGetError();
        glDeleteTextures(1, &tex);
        e
    };
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(
        DEPTH_ASKED.lock().unwrap().first(),
        Some(&s.depth_id),
        "the depth is read through depth_resolve, one sample a pixel"
    );
}

/// Respecifying a renderbuffer at its size gives it new contents: the stale
/// samples kept for the old storage are not drawn back. The new storage
/// comes back at the same GPU address (the mock hands a freed block to the
/// next allocation of its size), so keying on the address alone would find
/// them: the Mali drops the slot when the GL core releases the storage
/// (`texture_released`), and the next frame preloads the new, zeroed
/// renderbuffer into every sample.
#[cfg(feature = "msaa-fbo")]
#[test]
fn a_same_size_respecification_drops_the_stale_samples() {
    let _g = context();
    let fbo = framebuffer(4, false).expect("4 samples are legal storage");
    program();
    clear_black();
    let first = draw(&HALF);
    // SAFETY: the renderbuffer this test made, bound as the colour attachment.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, fbo.rbs[0]);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 8, 8);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    gpu::reset();
    /* the other half, with no clear: nothing of the first draw may come back */
    let second = draw(&OTHER_HALF);
    assert_eq!(second.gpu, first.gpu, "the premise: the same address and size");
    let frames = gpu::frames();
    assert_kept_and_resolved(&frames, second.gpu);
    let p = frames[0].preload.expect("a frame with no clear starts from the image");
    assert_eq!((p.samples, p.plane.pointer), (1, second.gpu), "the new storage into every sample, not the old samples");
    assert!(!p.flags.multisample_enable && !p.flags.evaluate_per_sample);
    let (e, dst) = resolve_blit(&fbo, false);
    assert_eq!(e, GL_NO_ERROR);
    check_halves(&read_pixels(dst.fb), |x, y| x + y > 7, |x, y| x + y == 7);
}

/// Two 4x FBOs drawn in turn each keep their own slot (a pool, not the one
/// store of the window path): a redraw of the first after the second still
/// preloads the first's samples.
#[cfg(feature = "msaa-fbo")]
#[test]
fn two_4x_fbos_keep_their_own_samples() {
    let _g = context();
    let a = framebuffer(4, false).expect("4 samples are legal storage");
    program();
    clear_black();
    let first = draw(&HALF);
    let store_a = assert_kept_and_resolved(&gpu::frames(), first.gpu);
    let b = framebuffer(4, false).expect("4 samples are legal storage");
    clear_black();
    gpu::reset();
    let other = draw(&OTHER_HALF);
    assert_ne!(other.gpu, first.gpu);
    assert_ne!(assert_kept_and_resolved(&gpu::frames(), other.gpu), store_a, "another slot");
    // SAFETY: a framebuffer this test made.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, a.fb) };
    gpu::reset();
    draw(&HALF);
    let frames = gpu::frames();
    assert_eq!(assert_kept_and_resolved(&frames, first.gpu), store_a);
    assert_eq!(frames[0].preload.map(|p| (p.samples, p.plane.pointer)), Some((4, store_a)));
    let (e, dst) = resolve_blit(&a, false);
    assert_eq!(e, GL_NO_ERROR);
    check_halves(&read_pixels(dst.fb), |x, y| x + y < 7, |x, y| x + y == 7);
    let (e, dst) = resolve_blit(&b, false);
    assert_eq!(e, GL_NO_ERROR);
    check_halves(&read_pixels(dst.fb), |x, y| x + y > 7, |x, y| x + y == 7);
}
