// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mali backend over a mock of its OSAL interface (src/osal/mali.rs):
//! every buffer at a GPU VA of the mock's choosing, far from any CPU
//! address, so a frame that used a CPU address as a VA would show; each
//! submit's stream read back from the buffer its VA names, as a GPU would.
//!
//! What a mock cannot show: pixels. The board run (tools/mali/panthor,
//! `panthor-gl`) is that check.

use super::*;
use crate::backend::{GlesAttrib, GLES_PRIM_TRIANGLES};
use crate::mali::cs::Instr;
use crate::osal::OsalOps;
use crate::shaderc::online::{codegen, store};
use crate::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use crate::testutil::OwnOsal;
use std::sync::Mutex;

const TRI_VS: &str = include_str!("../../tools/mali/testdata/tri.vert");
const TRI_FS: &str = include_str!("../../tools/mali/testdata/tri.frag");
const TV_VS: &str = include_str!("../../tools/mali/testdata/tri_varying.vert");
const TV_FS: &str = include_str!("../../tools/mali/testdata/tri_varying.frag");

/// The mock's first VA and the room between two buffers.
const VA0: u64 = 0x40_0000_0000;
const VA_STEP: u64 = 0x10_0000;
/// The OSAL's answers: the board's (panthor-frame on the T6, L1).
const GPU: OsalMaliGpu =
    OsalMaliGpu { gpu_id: 0xa867_0005, tiler_features: 0x809, cs_reg_count: 96, unpreserved_cs_reg_count: 4 };
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

struct Buf {
    id: usize,
    gpu: u64,
    size: usize,
    flags: u32,
    mem: Box<[u64]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Submit(i64),
    Wait(i64),
    /// A buffer the backend or the GL core freed, by its GPU VA.
    Free(u64),
    /// A freed buffer the mock gave back: its hold fence completed.
    Release(u64),
}

#[derive(Default)]
struct Mock {
    bufs: Vec<Buf>,
    allocs: usize,
    /// Each submit's stream VA and words, read from the buffer at that VA.
    submits: Vec<(u64, Vec<u64>)>,
    events: Vec<Event>,
    fences: i64,
    /// The fences that completed: only the one a wait asked for, or one a
    /// test completes by hand (a GPU that finished late).
    done: std::collections::BTreeSet<i64>,
    /// The fences whose waits time out (return 1) and leave them running.
    hang: std::collections::BTreeSet<i64>,
    /// The last fence waited for: waits come in submit order.
    last_wait: i64,
    /// Answer the queries (false: an OSAL without a Mali queue).
    mali: bool,
    /// Freed buffers held until the kick being recorded at their free
    /// completes (`gpu_free`'s contract, as tools/mali/panthor's OSAL and
    /// zephyr.c keep it): (id, VA, fence).
    held: Vec<(usize, u64, i64)>,
}

static MOCK: Mutex<Option<Mock>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Mock) -> R) -> R {
    f(MOCK.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the mock is installed"))
}

impl Mock {
    fn find(&self, gpu: u64, len: usize) -> Option<&Buf> {
        self.bufs.iter().find(|b| gpu >= b.gpu && gpu + len as u64 <= b.gpu + b.size as u64)
    }
    fn read(&self, gpu: u64, len: usize) -> Vec<u8> {
        let b = self.find(gpu, len).unwrap_or_else(|| panic!("{len} bytes at {gpu:#x}: no buffer holds them"));
        // SAFETY: inside the buffer's words.
        let bytes = unsafe { core::slice::from_raw_parts(b.mem.as_ptr().cast::<u8>(), b.size) };
        let at = (gpu - b.gpu) as usize;
        bytes[at..at + len].to_vec()
    }
    /// What a GPU writes: `bytes` at `gpu`.
    fn write(&mut self, gpu: u64, bytes: &[u8]) {
        let len = bytes.len();
        let b = self.bufs.iter_mut().find(|b| gpu >= b.gpu && gpu + len as u64 <= b.gpu + b.size as u64);
        let b = b.unwrap_or_else(|| panic!("{len} bytes at {gpu:#x}: no buffer holds them"));
        // SAFETY: inside the buffer's words.
        let mem = unsafe { core::slice::from_raw_parts_mut(b.mem.as_mut_ptr().cast::<u8>(), b.size) };
        let at = (gpu - b.gpu) as usize;
        mem[at..at + len].copy_from_slice(bytes);
    }
}

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, flags: u32, out: *mut OsalGpuMem) -> i32 {
    with(|m| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let cpu = mem.as_mut_ptr().cast::<c_void>();
        let gpu = VA0 + m.allocs as u64 * VA_STEP;
        assert!(size as u64 <= VA_STEP, "the mock spaces buffers {VA_STEP:#x} apart");
        m.allocs += 1;
        let id = m.allocs;
        m.bufs.push(Buf { id, gpu, size, flags, mem });
        // SAFETY: the caller's out-parameter.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: id as *mut c_void } };
    });
    0
}

unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller's descriptor.
    let m = unsafe { &mut *mem };
    if m.impl_.is_null() {
        return 0;
    }
    let id = m.impl_ as usize;
    let gpu = m.gpu;
    with(|k| {
        k.events.push(Event::Free(gpu));
        let fence = k.fences + 1;
        k.held.push((id, gpu, fence));
        k.release_held();
    });
    *m = OsalGpuMem::ZERO;
    0
}

unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    assert_eq!(len, core::mem::size_of::<OsalMaliKick>());
    // SAFETY: the backend passes an OsalMaliKick.
    let k = unsafe { *kick.cast::<OsalMaliKick>() };
    with(|m| {
        assert!(k.stream_gpu.is_multiple_of(64) && k.stream_bytes > 0 && k.stream_bytes.is_multiple_of(8));
        let bytes = m.read(k.stream_gpu, k.stream_bytes as usize);
        let words = bytes.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
        m.submits.push((k.stream_gpu, words));
        m.fences += 1;
        let f = m.fences;
        m.events.push(Event::Submit(f));
        f
    })
}

/// A wait completes exactly the fence it names, never an earlier or a
/// later one, and times out on a hung fence. Waits come in submit order
/// (a fence at or after the last one waited for) and only for a fence
/// that was submitted.
unsafe extern "C" fn m_wait(fence: i64, _timeout_us: i64) -> i32 {
    with(|m| {
        assert!(fence >= 1 && fence <= m.fences, "a wait for fence {fence}, which was never submitted");
        assert!(fence >= m.last_wait, "a wait for fence {fence} after one for fence {}", m.last_wait);
        m.last_wait = fence;
        m.events.push(Event::Wait(fence));
        if m.hang.contains(&fence) {
            return 1;
        }
        m.done.insert(fence);
        m.release_held();
        0
    })
}

impl Mock {
    /// The last fence before the first one not done (the OSAL's
    /// `completed`: every fence up to it completed).
    fn completed(&self) -> i64 {
        (1..=self.fences).find(|f| !self.done.contains(f)).map_or(self.fences, |f| f - 1)
    }

    /// Give back every held buffer whose fence has completed.
    fn release_held(&mut self) {
        let done = self.completed();
        let (gone, kept): (Vec<_>, Vec<_>) = self.held.drain(..).partition(|&(_, _, f)| f <= done);
        self.held = kept;
        for (id, gpu, _) in gone {
            self.events.push(Event::Release(gpu));
            self.bufs.retain(|b| b.id != id);
        }
    }
}

unsafe extern "C" fn m_completed() -> i64 {
    with(|m| {
        m.release_held();
        m.completed()
    })
}

unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    if !with(|m| m.mali) {
        return -ENOTSUP;
    }
    // SAFETY: the caller's out-parameter of `sz` bytes.
    unsafe {
        match what {
            OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
            OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => {
                *out.cast::<OsalMaliHeap>() = HEAP
            }
            _ => return -ENOTSUP,
        }
    }
    0
}

static OPS: OsalOps = OsalOps {
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
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}

/// The mock installed, the backend down, for the guard's life.
struct Rig {
    _osal: OwnOsal,
}

impl Rig {
    fn new(mali: bool) -> Rig {
        let osal = OwnOsal::new(&OPS, &yakogl_backend_mali);
        *MOCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Mock { mali, ..Mock::default() });
        Rig { _osal: osal }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        mali_shutdown();
        /* the buffers the backend freed are the OSAL's to give back */
        let left: Vec<(usize, usize)> =
            with(|m| m.bufs.iter().filter(|b| !m.held.iter().any(|h| h.0 == b.id)).map(|b| (b.id, b.size)).collect());
        *MOCK.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if !std::thread::panicking() {
            /* only the test's own buffers (a target) may outlive the backend */
            assert!(left.len() <= 1, "{} buffers left after shutdown (id, bytes): {left:?}", left.len());
        }
    }
}

/// A target buffer the GL core would own: 64x64 RGBA8.
fn target() -> u64 {
    let mut t = OsalGpuMem::ZERO;
    // SAFETY: a valid out-parameter.
    assert_eq!(unsafe { m_alloc(0, 64 * 64 * 4, OSAL_MEM_DATA, &mut t) }, 0);
    t.gpu
}

/// The board's triangle as the GL core resolves it: a 64x64 RGBA8 FBO
/// cleared to blue, six vertices of one vec2 attribute at location 0.
fn triangle(target_gpu: u64, vertices: &[f32; 12]) -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    d.target_gpu = target_gpu;
    d.target_w = 64;
    d.target_h = 64;
    d.target_format = backend::GLES_SURF_RGBA8888;
    d.do_clear = true;
    d.clear_rgba = [0.0, 0.0, 1.0, 1.0];
    d.prim = GLES_PRIM_TRIANGLES;
    d.index_count = 6;
    d.instance_count = 1;
    d.num_attribs = 1;
    d.attribs[0] = GlesAttrib {
        vi_reg: 0,
        components: 2,
        format: GLES_ATTRIBF_FLOAT32,
        stride: 8,
        cpu: vertices.as_ptr().cast(),
        cpu_bytes: 48,
        ..GlesAttrib::ZERO
    };
    d.viewport = [0, 0, 64, 64];
    d.depth_range = [0.0, 1.0];
    d.line_width = 1.0;
    d.color_write = true;
    d.color_mask = 0xf;
    d
}

const VERTICES: [f32; 12] = [-1.0, -1.0, 0.0, -1.0, -1.0, 1.0, 0.0, -1.0, 0.0, 1.0, -1.0, 1.0];

/// One draw or clear, then the flush that submits its frame.
fn draw_flush(d: &GlesDraw) -> i32 {
    // SAFETY: a valid draw whose records the caller keeps.
    let rc = unsafe { mali_draw(d) };
    if rc != 0 {
        return rc;
    }
    mali_flush()
}

/// Compile a vertex and a fragment shader with the Valhall back end into
/// keep, and run `f` on the two records while keep holds them.
fn with_programs<R>(vs: &str, fs: &str, f: impl FnOnce(&ShcShader, &ShcShader) -> R) -> R {
    let _k = store::test_keep();
    let one = |stage, src: &str| {
        store::with_scratch(|s| {
            codegen::compile_linked_blocks(stage, src.as_bytes(), 0, &[], s, store::keep()).map(|(sh, _)| sh)
        })
        .expect("the scratch is free")
        .unwrap_or_else(|e| panic!("{src}: {e:?}"))
    };
    let (v, fr) = (one(SHC_STAGE_VERTEX, vs), one(SHC_STAGE_FRAGMENT, fs));
    f(&v, &fr)
}

fn move48(w: u64) -> Option<(u32, u64)> {
    match Instr::decode(w) {
        Ok(Instr::Move48(m)) => Some((m.destination, m.immediate)),
        _ => None,
    }
}

/// The 64-bit register pair `reg` as the moves before `RUN_IDVS` left it
/// (a `MOVE48`, or the two `MOVE32`s `cs_move64_to` emits for a wider
/// value); None when no move wrote it.
fn reg_at_idvs(words: &[u64], reg: u32) -> Option<u64> {
    let run = words.iter().position(|&w| matches!(Instr::decode(w), Ok(Instr::RunIdvs(_))))?;
    let mut r = [None::<u32>; 128];
    for &w in &words[..run] {
        match Instr::decode(w) {
            Ok(Instr::Move48(m)) => {
                r[m.destination as usize] = Some(m.immediate as u32);
                r[m.destination as usize + 1] = Some((m.immediate >> 32) as u32);
            }
            Ok(Instr::Move32(m)) => r[m.destination as usize] = Some(m.immediate),
            _ => {}
        }
    }
    let lo = r[reg as usize]?;
    Some(u64::from(lo) | u64::from(r[reg as usize + 1].unwrap_or(0)) << 32)
}

/// Every 64-bit value the stream moves: each MOVE48's, and each pair of
/// MOVE32s into an even register and the next one.
fn moved_values(words: &[u64]) -> Vec<(u32, u64)> {
    let mut v = Vec::new();
    for (i, &w) in words.iter().enumerate() {
        match Instr::decode(w) {
            Ok(Instr::Move48(m)) => v.push((m.destination, m.immediate)),
            Ok(Instr::Move32(lo)) if lo.destination.is_multiple_of(2) => {
                if let Some(Ok(Instr::Move32(hi))) = words.get(i + 1).map(|&n| Instr::decode(n)) {
                    if hi.destination == lo.destination + 1 {
                        v.push((lo.destination, u64::from(lo.immediate) | u64::from(hi.immediate) << 32));
                    }
                }
            }
            _ => {}
        }
    }
    v
}

/// The OSAL interface's own words: `init` asks for the device and the heap,
/// takes every buffer from the OSAL (shader code executable), builds the
/// per-context objects in its context buffer and submits the queue-init
/// stream from its own buffer: `MOVE d72, <heap ctx>` and `HEAP_SET d72`,
/// the trace's first two words for the trace's heap context
/// (`frame_tests.rs`, `the_queue_init_stream_is_the_trace_s_heap_set`).
#[test]
fn init_takes_its_buffers_and_runs_the_heap_set_from_the_osal() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let (bufs, submits, events) = with(|m| {
        let b: Vec<(u64, usize, u32)> = m.bufs.iter().map(|b| (b.gpu, b.size, b.flags)).collect();
        (b, m.submits.clone(), m.events.clone())
    });
    /* context, geometry, init, two slots of desc data code spds cs, then
     * the occlusion counters */
    assert_eq!(bufs.len(), 3 + 2 * 5 + 1);
    let exec: Vec<bool> = bufs.iter().map(|b| b.2 & OSAL_MEM_GPU_EXEC != 0).collect();
    assert_eq!(exec, [false, false, false, false, false, true, false, false, false, false, true, false, false, false]);
    assert_eq!(bufs[1].1, frame::GEOMETRY_BUFFER_BYTES as usize);
    assert_eq!(bufs[13].1, QUERY_BYTES, "a counter block for each query handle");
    assert_eq!(events, [Event::Submit(1), Event::Wait(1)]);
    let (va, words) = &submits[0];
    assert_eq!(*va, bufs[2].0, "the init stream runs from the init buffer");
    assert_eq!(words.len(), 2);
    assert_eq!(move48(words[0]), Some((72, HEAP.ctx_gpu)));
    assert!(matches!(Instr::decode(words[1]), Ok(Instr::HeapSet(h)) if h.address == 72));
    /* the heap descriptor names the OSAL's chunk: size, base, bottom, top */
    let ctx = with(|m| m.read(bufs[0].0, 32));
    let q = |i: usize| u64::from_le_bytes(ctx[8 * i..8 * i + 8].try_into().unwrap());
    assert_eq!(q(0) >> 32, u64::from(HEAP.chunk_size));
    assert_eq!(q(1), HEAP.first_chunk_gpu);
    assert_eq!((q(2), q(3)), (HEAP.first_chunk_gpu + 64, HEAP.first_chunk_gpu + u64::from(HEAP.chunk_size)));
    /* a second init is the first's */
    assert_eq!(mali_init(), 0);
    assert_eq!(with(|m| m.submits.len()), 1);
}

/// An OSAL that cannot run a Mali queue: init refuses with the line that
/// says so and gives back what it took; a draw is then -ENODEV, after the
/// checks of what is not built.
#[test]
fn without_a_mali_osal_init_and_draws_are_refused() {
    let _rig = Rig::new(false);
    assert_eq!(mali_init(), -ENODEV);
    assert_eq!(with(|m| (m.bufs.len(), m.submits.len())), (0, 0));
    let t = target();
    let mut clear = triangle(t, &VERTICES);
    clear.index_count = 0;
    // SAFETY: a valid draw.
    assert_eq!(unsafe { mali_draw(&clear) }, -ENODEV);
    let mut blend = clear;
    blend.index_count = 6;
    blend.blend = true;
    // SAFETY: a valid draw.
    assert_eq!(unsafe { mali_draw(&blend) }, -ENOTSUP);
}

/// The draw path end to end on the mock: the triangle with tri's programs
/// as our compiler makes them. The submitted stream is the one the frame
/// builder makes for the same inputs at the same VAs (the host proof),
/// read back from the slot's own stream buffer; every block the stream
/// names is inside an OSAL buffer, none at a CPU address; the code the
/// SPDs name is the compiler's, byte for byte; the FAU blocks hold Mesa's
/// words (the viewport, the store offsets, the blend constants, 1.0h).
#[test]
fn a_draw_submits_the_frame_the_builder_makes_at_the_osal_s_vas() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let words = with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = triangle(t, &VERTICES);
        d.vs = vs;
        d.fs = fs;
        // SAFETY: a valid draw whose records keep holds.
        assert_eq!(unsafe { mali_draw(&d) }, 0);
        assert_eq!(with(|m| m.submits.len()), 1, "the frame stays open until the flush");
        assert_eq!(mali_flush(), 0);
        let (bufs, (va, words)) =
            with(|m| (m.bufs.iter().map(|b| b.gpu).collect::<Vec<_>>(), m.submits.last().cloned().unwrap()));
        let slot0 = &bufs[3..8];
        assert_eq!(va, slot0[4], "the frame runs from slot 0's stream buffer");

        /* the host proof: the same frame built by frame.rs into memory of
         * its own whose arenas are based at the slot's VAs */
        let (mut ctx_m, mut desc_m, mut data_m, mut code_m, mut spd_m) = (
            vec![0u8; CONTEXT_BYTES],
            vec![0u8; DESC_BYTES],
            vec![0u8; DATA_BYTES],
            vec![0u8; CODE_BYTES],
            vec![0u8; SPD_BYTES],
        );
        let mut expect = vec![0u64; CS_WORDS];
        let n;
        {
            let mut ctx_a = Arena::new("context", bufs[0], &mut ctx_m).unwrap();
            let ctx = frame::build_context(&mut ctx_a, HEAP.first_chunk_gpu, HEAP.chunk_size, bufs[1], 8).unwrap();
            let mut a = Arenas {
                desc: Arena::new("desc", slot0[0], &mut desc_m).unwrap(),
                data: Pool::new("data", slot0[1], &mut data_m).unwrap(),
                code: Arena::new("code", slot0[2], &mut code_m).unwrap(),
                spds: Arena::new("spds", slot0[3], &mut spd_m).unwrap(),
            };
            let (vm, fm) = stage_metas(&d).unwrap();
            // SAFETY: the records keep holds.
            let (vc, fc) = unsafe { (program::code_bytes(vs).unwrap(), program::code_bytes(fs).unwrap()) };
            let progs = (ShaderInput { code: vc, meta: vm.meta() }, ShaderInput { code: fc, meta: fm.meta() });
            let mut cs = Stream::new(&mut expect, 92);
            let target = frame::Target {
                gpu: t,
                width: 64,
                height: 64,
                row_stride: 256,
                format: frame::TargetFormat::Rgba8888,
                samples: 1,
            };
            let mut open = frame::begin_frame(&mut a.desc, &mut cs).unwrap();
            let attrs = MaliShader::of(vs).unwrap().attributes;
            add_gl_draw(&mut open, &d, progs, attrs, &ctx, &target, false, None, None, &mut a, &mut cs).unwrap();
            frame::end_frame(&open, &mut a.desc, &mut cs, &ctx, &target, d.clear_rgba, None).unwrap();
            n = cs.words().len();
        }
        assert_eq!(words, expect[..n], "the submitted words are the host proof's");
        /* and the descriptors are in the slot's desc buffer, as built */
        assert_eq!(with(|m| m.read(slot0[0], DESC_BYTES)), desc_m);

        /* no word names a CPU address, and every value in the mock's VA
         * window is inside a live OSAL buffer */
        let cpu: Vec<(u64, u64)> =
            with(|m| m.bufs.iter().map(|b| (b.mem.as_ptr() as u64, b.mem.as_ptr() as u64 + b.size as u64)).collect());
        for (dst, imm) in moved_values(&words) {
            let addr = imm & 0x00ff_ffff_ffff_ffc0;
            assert!(!cpu.iter().any(|&(lo, hi)| (lo..hi).contains(&addr)), "d{dst} = {imm:#x} is a CPU address");
            /* a value in the mock's VA window is an address: of a buffer */
            if (VA0..VA0 + VA_STEP * with(|m| m.allocs) as u64).contains(&addr) {
                assert!(with(|m| m.find(addr, 1).is_some()), "d{dst} = {imm:#x} is in no OSAL buffer");
            }
        }

        /* the programs: the SPD's binary is the compiler's code */
        let spd = reg_at_idvs(&words, 16).expect("the VS SPD");
        let binary = with(|m| m.read(spd + 8, 8));
        let binary = u64::from_le_bytes(binary.try_into().unwrap());
        // SAFETY: the records keep holds.
        let vc = unsafe { program::code_bytes(vs) }.unwrap();
        assert_eq!(with(|m| m.read(binary, vc.len())), vc);
        let spd = reg_at_idvs(&words, 20).expect("the FS SPD");
        let binary = u64::from_le_bytes(with(|m| m.read(spd + 8, 8)).try_into().unwrap());
        // SAFETY: as above.
        let fc = unsafe { program::code_bytes(fs) }.unwrap();
        assert_eq!(with(|m| m.read(binary, fc.len())), fc);
        words
    });
    /* the push blocks: the VS's eight words, four 64-bit slots; the FS's
     * five, padded to three slots */
    let fau = |reg| reg_at_idvs(&words, reg).expect("a FAU register");
    let (vf, ff) = (fau(8), fau(12));
    assert_eq!((vf >> 56, ff >> 56), (4, 3));
    let block = |f: u64, n: usize| -> Vec<u32> {
        with(|m| m.read(f & 0x00ff_ffff_ffff_ffff, 4 * n))
            .chunks(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    };
    let h = 32.0f32.to_bits();
    assert_eq!(block(vf, 8), [h, h, h, h, 0.5f32.to_bits(), 4, 8, 12]);
    assert_eq!(block(ff, 6), [0, 0, 0, 0, 0x3c00, 0]);
    /* no varying program for tri: no VARY_SPD, VARY_SIZE 0 */
    assert_eq!(reg_at_idvs(&words, 18), None);
    // SAFETY: a plain call.
    assert_eq!(mali_finish(), 0);
}

/// tri_varying: the varying program at the record's offset 256 gets its own
/// SPD (Mesa's `programs[2]`), named by VARY_SPD (d18), with VARY_SIZE the
/// vec3's 12 bytes and the secondary-shader flag set in RUN_IDVS's override;
/// the colour attribute is record 1 of two.
#[test]
fn a_draw_with_a_varying_names_the_varying_program() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let colours = [0.0f32, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    let words = with_programs(TV_VS, TV_FS, |vs, fs| {
        let mut d = triangle(t, &VERTICES);
        d.vs = vs;
        d.fs = fs;
        d.num_attribs = 2;
        d.attribs[1] = GlesAttrib {
            vi_reg: 1,
            components: 3,
            format: GLES_ATTRIBF_FLOAT32,
            stride: 12,
            cpu: colours.as_ptr().cast(),
            cpu_bytes: 72,
            ..GlesAttrib::ZERO
        };
        // SAFETY: a valid draw whose records keep holds.
        assert_eq!(unsafe { mali_draw(&d) }, 0);
        assert_eq!(mali_flush(), 0);
        with(|m| m.submits.last().cloned().unwrap().1)
    });
    let vspd = reg_at_idvs(&words, 16).expect("the VS SPD");
    let vary = reg_at_idvs(&words, 18).expect("the varying SPD");
    assert_eq!(vary, vspd + 32, "Mesa's programs[1] and [2] side by side");
    let bin = |spd: u64| u64::from_le_bytes(with(|m| m.read(spd + 8, 8)).try_into().unwrap());
    assert_eq!(bin(vary), bin(vspd) + 256);
    let size = words
        .iter()
        .filter_map(|&w| match Instr::decode(w) {
            Ok(Instr::Move32(m)) if m.destination == 48 => Some(m.immediate),
            _ => None,
        })
        .next_back();
    assert_eq!(size, Some(12));
    let run = words.iter().find_map(|&w| match Instr::decode(w) {
        Ok(Instr::RunIdvs(r)) => Some(r),
        _ => None,
    });
    use crate::mali::genxml::v10::PrimitiveFlags;
    let packed = |on| u32::from_le_bytes(PrimitiveFlags { secondary_shader: on, ..Default::default() }.pack().unwrap());
    let bit = packed(true) ^ packed(false);
    assert_ne!(bit, 0);
    assert!(run.is_some_and(|r| r.flags_override & bit == bit), "the secondary-shader flag");
    /* two attributes and two buffers in the VS resource table */
    let srt = reg_at_idvs(&words, 0).expect("the VS SRT") & !63;
    let entry = |i: u64| with(|m| m.read(srt + 16 * i, 16));
    let size = |e: Vec<u8>| u64::from_le_bytes(e[8..16].try_into().unwrap());
    assert_eq!((size(entry(1)), size(entry(2))), (64, 64), "2 attributes, 2 buffers of 32 bytes a descriptor");
}

/// A slot's memory is written again only after its last frame's fence: with
/// two slots, the third frame waits for the first. `finish` waits for the
/// rest, oldest first; `kick_pending` follows the waits.
#[test]
fn a_slot_is_rebuilt_only_after_its_fence() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let mut clear = triangle(t, &VERTICES);
    clear.index_count = 0;
    for _ in 0..3 {
        assert_eq!(draw_flush(&clear), 0);
    }
    use Event::*;
    assert_eq!(with(|m| m.events.clone()), [Submit(1), Wait(1), Submit(2), Submit(3), Wait(2), Submit(4)]);
    /* kick ids count from the process's first kick; the last three are
     * ours, and the kick being recorded (the next) is pending */
    let k = mali_kick_id() - 1;
    assert!(mali_kick_pending(k + 1));
    assert_eq!(with(|m| m.completed()), 2, "the waits completed fences 1 and 2 only");
    assert!(mali_kick_pending(k - 1) && mali_kick_pending(k) && !mali_kick_pending(k - 2));
    assert_eq!(mali_finish(), 0);
    assert_eq!(with(|m| m.events[6..].to_vec()), [Wait(3), Wait(4)]);
    assert!(!mali_kick_pending(k));
    /* the two slots' streams are their own buffers */
    let s = with(|m| m.submits.iter().map(|s| s.0).collect::<Vec<_>>());
    assert_eq!((s[1], s[2], s[3]), (s[1], s[2], s[1]));
    assert_ne!(s[1], s[2]);
}

/// A wait that times out does not free its slot: the GPU may still read
/// it. The draw that waited is lost (-EIO, counted); the next draw that
/// needs the slot is refused with SLOT_STILL_IN_FLIGHT without a wait,
/// without a submit and without writing the slot, and counted again. Once
/// the OSAL reports the fence complete (the GPU finished late) the slot is
/// used again; the other slot is never touched by any of it.
#[test]
fn a_slot_whose_wait_timed_out_stays_in_flight() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let mut clear = triangle(t, &VERTICES);
    clear.index_count = 0;
    /* fence 2 is slot 0's frame, and it hangs */
    with(|m| m.hang.insert(2));
    assert_eq!(draw_flush(&clear), 0);
    assert_eq!(draw_flush(&clear), 0);
    let slot0_cs = with(|m| m.submits[1].0);
    let stream = |m: &mut Mock| m.read(slot0_cs, 8 * CS_WORDS);
    let before = with(stream);
    let lost = backend::gpu_work_lost();

    /* the third frame needs slot 0: its wait times out */
    assert_eq!(draw_flush(&clear), -EIO);
    use Event::*;
    assert_eq!(with(|m| m.events.clone()), [Submit(1), Wait(1), Submit(2), Submit(3), Wait(2)]);
    assert_eq!(backend::gpu_work_lost(), lost + 1);
    // SAFETY: the backend is not running; one short-lived reference.
    let slot0 = || unsafe { STATE.get() }.slots[0].in_flight;
    assert!(slot0().is_some_and(|f| f.fence == 2 && f.wait_failed), "slot 0 stays in flight");
    let k2 = slot0().unwrap().kick;
    assert!(mali_kick_pending(k2));

    /* the fourth: refused by name, no wait, no submit, slot 0 unwritten */
    // SAFETY: as above.
    assert_eq!(wait_slot(unsafe { STATE.get() }, 0), Err((-EIO, SLOT_STILL_IN_FLIGHT)));
    assert_eq!(draw_flush(&clear), -EIO);
    assert_eq!(with(|m| (m.events.len(), m.submits.len())), (5, 3));
    assert_eq!(with(stream), before, "slot 0's stream was not written");
    assert_eq!(backend::gpu_work_lost(), lost + 2, "the timed-out draw and the refused one");
    assert!(slot0().is_some());
    /* finish does not wait for it again either */
    assert_eq!(mali_finish(), -EIO);
    assert_eq!(with(|m| m.events[5..].to_vec()), [Wait(3)]);

    /* the GPU finishes late: the slot is used again, without a wait */
    with(|m| m.done.insert(2));
    assert!(!mali_kick_pending(k2));
    assert_eq!(draw_flush(&clear), 0);
    assert_eq!(with(|m| m.events[6..].to_vec()), [Submit(4)]);
    assert_eq!(with(|m| m.submits[3].0), slot0_cs);
    assert_eq!(mali_finish(), 0);
    assert_eq!(backend::gpu_work_lost(), lost + 2);
}

/// The backend as `init` left it: no open frame, no plane, nothing in
/// flight (the mock's waits complete), the clear program as built.
fn fresh() {
    let _ = mali_finish();
    // SAFETY: the rig serialises the backend; one short-lived reference.
    let g = unsafe { STATE.get() };
    g.open = None;
    g.planes = [NO_PLANE; PLANES];
}

/// The outcome of `d` after `before`, each recorded on a fresh backend
/// through the function `mali_draw` calls, with the cause whole (`mali_draw`
/// returns only its errno, and the library logs only the first refusal of
/// the process).
fn why_after(before: &[GlesDraw], d: &GlesDraw) -> Result<(), Fail> {
    fresh();
    // SAFETY: as in `fresh`.
    let g = unsafe { STATE.get() };
    for b in before {
        record_in(g, b).expect("the draws before it are recorded");
    }
    let r = record_in(g, d);
    fresh();
    r
}

fn why(d: &GlesDraw) -> Result<(), Fail> {
    why_after(&[], d)
}

/// What GL object `id` (generation `gen`) holds in its `aspect` plane.
fn plane_of(g: &MaliState, id: u32, gen: u32, aspect: usize) -> Plane {
    g.planes
        .iter()
        .find(|p| p.id == id && p.aspect == aspect)
        .filter(|p| p.gen == gen)
        .map_or(Plane::UNDEFINED, |p| p.plane)
}

/// The window surface's D24S8 on the draw: one object on both points.
fn with_ds(mut d: GlesDraw) -> GlesDraw {
    d.ds_id = 0x8000_0000 | (d.target_gpu >> 12) as u32;
    (d.depth_id, d.stencil_id) = (d.ds_id, d.ds_id);
    (d.ds_w, d.ds_h) = (d.target_w, d.target_h);
    d
}

/// One case of the refusal table: what, the outcome, the outcome wanted.
type Case = (&'static str, Result<(), Fail>, Result<(), Fail>);

/// Every refusal line of this module and of the frame builder it reaches
/// is returned for exactly the input that names it, and the code maps its
/// kind to the errno the contract of `backend::draw` gives.
#[test]
fn each_refusal_is_the_exact_line_of_its_cause() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    // SAFETY: as in `fresh`.
    assert!(unsafe { STATE.get() }.clear_program.built, "the clear program builds at init");
    let t = target();
    let vertices = [0.0f32; 12];
    let base = triangle(t, &vertices);
    let not_built = |l| Err(Fail::Frame(FrameError::NotBuilt(l)));
    let invalid = |l| Err(Fail::Frame(FrameError::Invalid(l)));
    let built: Result<(), Fail> = Ok(());

    let mut no_records = base;
    no_records.vs = core::ptr::null();
    let mut reached = Vec::new();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut draw = base;
        (draw.vs, draw.fs) = (vs, fs);
        let mut clear = draw;
        clear.index_count = 0;
        let state = |f: fn(&mut GlesDraw)| {
            let mut d = draw;
            f(&mut d);
            d
        };
        let no_target = state(|d| d.target_gpu = 0);
        let two_targets = state(|d| d.num_targets = 2);
        let shifted = state(|d| d.targets[0].out_shift = 1);
        let multisampled = state(|d| d.samples = 4);
        let two_samples = state(|d| d.samples = 2);
        let msaa_depth = state(|d| (d.samples, d.depth_test, d.depth_func) = (4, true, backend::GLES_CMP_LESS));
        let rgba16f = state(|d| d.target_format = backend::GLES_SURF_RGBA16F);
        let r16f = state(|d| d.target_format = backend::GLES_SURF_R16F);
        let xrgb = state(|d| d.target_format = backend::GLES_SURF_XRGB8888);
        let rgb565 = state(|d| d.target_format = backend::GLES_SURF_RGB565);
        let huge_pitch = state(|d| d.target_pitch = u32::MAX / 2);
        let scissored = state(|d| d.clear_quad = true);
        let masked = state(|d| d.clear_color_mask = 0x7);
        let depth_test = state(|d| (d.depth_test, d.depth_func) = (true, backend::GLES_CMP_LESS));
        let depth_write = state(|d| d.depth_write = true);
        let stencil = state(|d| (d.stencil, d.stencil_func) = (true, backend::GLES_CMP_EQUAL));
        let blend = state(|d| {
            use crate::shaderc::{SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA, SHC_BLEND_ADD};
            d.blend = true;
            (d.blend_src_rgb, d.blend_dst_rgb) = (SHC_BF_SRC_ALPHA as u8, SHC_BF_INV_SRC_ALPHA as u8);
            (d.blend_src_alpha, d.blend_dst_alpha) = (SHC_BF_SRC_ALPHA as u8, SHC_BF_INV_SRC_ALPHA as u8);
            (d.blend_eq_rgb, d.blend_eq_alpha) = (SHC_BLEND_ADD as u8, SHC_BLEND_ADD as u8);
        });
        let mut min_blend = blend;
        (min_blend.blend_eq_rgb, min_blend.blend_eq_alpha) =
            (crate::shaderc::SHC_BLEND_MIN as u8, crate::shaderc::SHC_BLEND_MIN as u8);
        /* case 8's fifth frame: a constant colour of three values, which
         * the fixed-function unit cannot hold: its blend shader */
        let mut const_blend = blend;
        {
            use crate::shaderc::{SHC_BF_CONST_COLOR, SHC_BF_INVERT};
            let (c, ic) = (SHC_BF_CONST_COLOR as u8, (SHC_BF_CONST_COLOR | SHC_BF_INVERT) as u8);
            (const_blend.blend_src_rgb, const_blend.blend_dst_rgb) = (c, ic);
            (const_blend.blend_src_alpha, const_blend.blend_dst_alpha) = (c, ic);
            const_blend.blend_color = [0.25, 0.5, 0.75, 1.0];
        }
        /* the same written through a colour mask: no blend shader is built */
        let mut const_blend_masked = const_blend;
        const_blend_masked.color_mask = 0x7;
        /* case 211: the constant colour at 4x, Mesa's words; masked, none */
        let mut const_blend_4x = const_blend;
        const_blend_4x.samples = 4;
        let mut const_blend_4x_masked = const_blend_4x;
        const_blend_4x_masked.color_mask = 0x7;
        /* case 190's columns 3 (GL_MIN on the colour) and 4
         * (ONE_MINUS_DST_ALPHA): Mesa's words at 1x, none at 4x */
        let (blend_190_3, blend_190_4) = case_190_columns(&blend);
        let mut blend_190_3_4x = blend_190_3;
        blend_190_3_4x.samples = 4;
        /* an RGBA16F target is RAW64 in the tile buffer: no dump blends
         * into it or writes it through a mask */
        let mut rgba16f_blend = blend;
        rgba16f_blend.target_format = backend::GLES_SURF_RGBA16F;
        let rgba16f_masked = state(|d| (d.target_format, d.color_mask) = (backend::GLES_SURF_RGBA16F, 0x7));
        let mut bad_blend = blend;
        bad_blend.blend_src_rgb = 0;
        let cull = state(|d| d.cull = backend::GLES_CULL_CCW);
        let bad_cull = state(|d| d.cull = backend::GLES_CULL_CCW + 1);
        let polygon_offset = state(|d| d.polygon_offset = true);
        let colour_off = state(|d| d.color_write = false);
        let colour_mask = state(|d| d.color_mask = 0x7);
        let query = state(|d| d.occlusion_query = 1);
        let bad_query = state(|d| d.occlusion_query = backend::GLES_QUERY_HANDLES);
        /* the GL core's alpha-to-coverage is "on and the target
         * multisampled" (backend.h): case 198's draw, alpha masked, and the
         * same with no channel written, which still runs its shader */
        let a2c = state(|d| (d.samples, d.alpha_to_coverage, d.color_mask) = (4, true, 0x7));
        let a2c_colour_off = state(|d| (d.samples, d.alpha_to_coverage, d.color_write) = (4, true, false));
        let mut blended_clear = blend;
        blended_clear.index_count = 0;
        let window = state(|d| d.flip_y = true);
        let ibo_indices = state(|d| (d.index_buf_gpu, d.index_size) = (0x60_0000, 2));
        let staged_indices = state(|d| {
            static STAGED: [u16; 6] = [0, 1, 2, 0, 1, 2];
            (d.index_buf_gpu, d.index_size, d.index_cpu, d.index_bytes) = (0x61_0000, 2, STAGED.as_ptr().cast(), 12);
        });
        let u8_indices = state(|d| (d.index_buf_gpu, d.index_size) = (0x60_0000, 1));
        let no_clear = state(|d| d.do_clear = false);
        /* rows of 288 bytes: the preload's texture cannot read them */
        let odd_rows = state(|d| (d.do_clear, d.target_pitch) = (false, 72));
        let xrgb_no_clear = state(|d| (d.do_clear, d.target_format) = (false, backend::GLES_SURF_XRGB8888));
        let msaa_no_clear = state(|d| (d.do_clear, d.samples) = (false, 4));
        let int_attrib = state(|d| d.attribs[0].format = GLES_ATTRIBF_FLOAT32 + 1);
        let vbo_attrib = state(|d| {
            (d.attribs[0].cpu, d.attribs[0].cpu_bytes) = (core::ptr::null(), 0);
            (d.attribs[0].buffer_gpu, d.attribs[0].buffer_bytes) = (0x50_0000, 48);
        });
        let huge_vbo = state(|d| {
            (d.attribs[0].cpu, d.attribs[0].cpu_bytes) = (core::ptr::null(), 0);
            (d.attribs[0].buffer_gpu, d.attribs[0].buffer_bytes) = (0x50_0000, 1 << 32);
        });
        let no_data = state(|d| (d.attribs[0].cpu, d.attribs[0].cpu_bytes) = (core::ptr::null(), 0));
        let moved_attrib = state(|d| d.attribs[0].vi_reg = 3);
        let instanced = state(|d| d.instance_count = 40);
        let per_instance = state(|d| {
            (d.attribs[0].per_instance, d.attribs[0].divisor, d.instance_count) = (1, 1, 40);
            (d.attribs[0].cpu, d.attribs[0].cpu_bytes) = (core::ptr::null(), 0);
            (d.attribs[0].buffer_gpu, d.attribs[0].buffer_bytes, d.attribs[0].offset) = (0x50_0000, 960, 8);
        });
        let per_instance_staged = state(|d| (d.attribs[0].per_instance, d.attribs[0].divisor) = (1, 0));
        let divisor_two = state(|d| (d.attribs[0].per_instance, d.attribs[0].divisor, d.instance_count) = (1, 2, 4));
        let small_ds = {
            let mut d = with_ds(depth_test);
            d.ds_w = 32;
            d
        };
        let mut window_clear = clear;
        window_clear.flip_y = true;
        /* a masked clear after the frame's whole clear: a quad */
        let mut later_masked = clear;
        later_masked.clear_color_mask = 0x1;
        /* a whole colour clear after a draw: a quad too */
        let mut no_clear_draw = draw;
        no_clear_draw.do_clear = false;

        let cases: Vec<Case> = vec![
            ("a clear is built", why(&clear), built),
            ("a GL draw whose records are not Mali programs", why(&no_records), not_built(NO_MALI_PROGRAM)),
            ("a draw with programs is built", why(&draw), built),
            ("no target", why(&no_target), invalid(INVALID_NO_TARGET)),
            ("two targets", why(&two_targets), not_built(NOT_BUILT_TARGET_COUNT)),
            ("draw buffer 0 GL_NONE", why(&shifted), not_built(NOT_BUILT_OUTPUT_SHIFT)),
            ("a 4x target is built", why(&multisampled), built),
            ("a 2x target", why(&two_samples), not_built(frame::NOT_BUILT_SAMPLES)),
            ("a 4x target whose D24S8 the draw does not use", why(&with_ds(multisampled)), built),
            ("a 4x draw with a depth test is built", why(&with_ds(msaa_depth)), built),
            ("an RGBA16F target is built", why(&rgba16f), built),
            ("a blended draw into RGBA16F", why(&rgba16f_blend), not_built(frame::NOT_BUILT_FLOAT_BLEND)),
            ("a colour-masked draw into RGBA16F", why(&rgba16f_masked), not_built(frame::NOT_BUILT_FLOAT_BLEND)),
            ("R16F", why(&r16f), not_built(NOT_BUILT_TARGET_FORMAT)),
            ("an XRGB8888 target is built", why(&xrgb), built),
            ("an RGB565 target is built", why(&rgb565), built),
            ("row stride", why(&huge_pitch), invalid(INVALID_ROW_STRIDE)),
            ("a scissored clear: a preloaded frame and the quad", why(&scissored), built),
            ("a masked clear: a preloaded frame and the quad", why(&masked), built),
            ("a masked clear in a frame is a quad", why_after(&[clear], &later_masked), built),
            ("a clear after a draw is a quad", why_after(&[draw], &clear), built),
            ("a depth test with a D24S8", why(&with_ds(depth_test)), built),
            ("a depth test without one", why(&depth_test), invalid(frame::INVALID_NO_ZS)),
            ("a depth write without the test writes nothing", why(&depth_write), built),
            ("a stencil test with a D24S8", why(&with_ds(stencil)), built),
            ("a D24S8 smaller than the target", why(&small_ds), not_built(NOT_BUILT_ZS_SIZE)),
            ("a fixed-function blend is built", why(&blend), built),
            ("a min blend needs a blend shader", why(&min_blend), not_built(frame::NOT_BUILT_BLEND_SHADER)),
            ("a constant colour of three values runs in a blend shader", why(&const_blend), built),
            (
                "a constant colour's blend shader through a colour mask",
                why(&const_blend_masked),
                not_built(frame::NOT_BUILT_BLEND_SHADER),
            ),
            ("case 211's constant colour at 4x runs in Mesa's blend shader", why(&const_blend_4x), built),
            (
                "case 211's 4x blend shader through a colour mask",
                why(&const_blend_4x_masked),
                not_built(frame::NOT_BUILT_BLEND_SHADER),
            ),
            ("case 190's GL_MIN column runs in Mesa's blend shader", why(&blend_190_3), built),
            ("case 190's ONE_MINUS_DST_ALPHA column runs in Mesa's blend shader", why(&blend_190_4), built),
            ("case 190's GL_MIN column at 4x", why(&blend_190_3_4x), not_built(frame::NOT_BUILT_BLEND_SHADER)),
            ("a blend code that is not the core's", why(&bad_blend), invalid(INVALID_BLEND_CODE)),
            ("face culling", why(&cull), built),
            ("a cull code that is not the core's", why(&bad_cull), invalid(INVALID_CULL)),
            ("a draw with polygon offset is built", why(&polygon_offset), built),
            ("colour writes off: no fragment shader", why(&colour_off), built),
            ("colour mask RGB", why(&colour_mask), built),
            ("a draw an occlusion query counts is built", why(&query), built),
            ("a query handle past the counters", why(&bad_query), invalid(INVALID_QUERY_HANDLE)),
            ("a 4x draw with alpha-to-coverage is built", why(&a2c), built),
            ("alpha-to-coverage with colour writes off is built", why(&a2c_colour_off), built),
            ("a clear with blending on is built (no draw, no state)", why(&blended_clear), built),
            ("a draw into a window target is built", why(&window), built),
            ("indices in an element buffer are built", why(&ibo_indices), built),
            ("staged indices are built", why(&staged_indices), built),
            ("indices that are not the core's u16", why(&u8_indices), invalid(INVALID_INDEX_SIZE)),
            ("no clear: a preloaded frame", why(&no_clear), built),
            ("no clear, rows not a multiple of 64 bytes", why(&odd_rows), not_built(frame::NOT_BUILT_PRELOAD_STRIDE)),
            ("no clear on an XRGB8888 target: a preloaded frame", why(&xrgb_no_clear), built),
            ("no clear on a 4x target: a preloaded frame", why(&msaa_no_clear), built),
            ("a draw without a clear in an open frame", why_after(&[clear], &no_clear_draw), built),
            ("an attribute that is not float32", why(&int_attrib), not_built(NOT_BUILT_ATTRIBUTE)),
            ("a float32 attribute in a buffer object", why(&vbo_attrib), built),
            ("a buffer object of 4 GiB", why(&huge_vbo), not_built(NOT_BUILT_ATTRIBUTE_BUFFER_SIZE)),
            ("an attribute with neither staged data nor a buffer", why(&no_data), not_built(NOT_BUILT_ATTRIBUTE)),
            ("the shader's location is not staged", why(&moved_attrib), not_built(NOT_BUILT_ATTRIBUTE_RECORDS)),
            ("an instanced draw is built", why(&instanced), built),
            ("a divisor-1 attribute in a buffer object, 40 instances, is built", why(&per_instance), built),
            ("a staged per-instance attribute (divisor 0 read as 1) is built", why(&per_instance_staged), built),
            ("a divisor of 2", why(&divisor_two), not_built(frame::NOT_BUILT_INSTANCE_DIVISOR)),
            ("a clear of a window target is built", why(&window_clear), built),
        ];
        /* the clear quad with no clear program */
        // SAFETY: as in `fresh`.
        let g = unsafe { STATE.get() };
        let program = g.clear_program;
        g.clear_program.built = false;
        let no_program = why_after(&[clear], &later_masked);
        // SAFETY: as above.
        unsafe { STATE.get() }.clear_program = program;
        let mut cases = cases;
        cases.push(("the quad without its program", no_program, not_built(NOT_BUILT_CLEAR_PROGRAM)));
        /* a preloaded frame with no preload program */
        // SAFETY: as in `fresh`.
        let preload = unsafe { STATE.get() }.preload;
        // SAFETY: as above.
        unsafe { STATE.get() }.preload = NO_PRELOAD_PROGRAM;
        let no_preload = why(&no_clear);
        // SAFETY: as above.
        unsafe { STATE.get() }.preload = preload;
        cases.push(("no clear and no preload program", no_preload, not_built(NOT_BUILT_PRELOAD_PROGRAM)));
        for (what, got, want) in &cases {
            assert_eq!(got, want, "{what}");
            if let Err(Fail::Frame(FrameError::NotBuilt(l) | FrameError::Invalid(l))) = got {
                reached.push(*l);
            }
        }
    });
    /* the texture program (sel_t_tex): the look-up reads binding 0, its
     * texture a 16x16 RGBA8 store of the mock's */
    let tex_vs = include_str!("../../tools/mali/testdata/sel_t_tex.vert");
    let tex_fs = include_str!("../../tools/mali/testdata/sel_t_tex.frag");
    let uv = [0.0f32; 12];
    let texel_bytes = vec![0x5au8; 16 * 16 * 4];
    let mut texels = OsalGpuMem::ZERO;
    // SAFETY: a valid out-parameter.
    assert_eq!(unsafe { m_alloc(0, 16 * 16 * 4, OSAL_MEM_DATA, &mut texels) }, 0);
    with_programs(tex_vs, tex_fs, |vs, fs| {
        let mut draw = base;
        (draw.vs, draw.fs) = (vs, fs);
        draw.num_attribs = 2;
        draw.attribs[1] = GlesAttrib { vi_reg: 1, cpu: uv.as_ptr().cast(), ..draw.attribs[0] };
        draw.num_textures = 1;
        draw.textures[0] = backend::GlesTextureBind {
            unit: 0,
            format: backend::GLES_TEXFMT_RGBA8,
            layers: 1,
            base_gpu: texels.gpu,
            width: 16,
            height: 16,
            stride_texels: 16,
            bytes: 16 * 16 * 4,
            wrap_s: backend::GLES_WRAP_CLAMP_TO_EDGE,
            wrap_t: backend::GLES_WRAP_REPEAT,
            ..backend::GlesTextureBind::ZERO
        };
        let tex = |f: fn(&mut backend::GlesTextureBind)| {
            let mut d = draw;
            f(&mut d.textures[0]);
            d
        };
        let mut unbound = draw;
        unbound.num_textures = 0;
        /* an 8-wide texture as the GL core stores it for this backend:
         * rows of 64 bytes (`linear_row_align`), its 8 texels in 32 */
        let mut narrow = draw;
        (narrow.textures[0].width, narrow.textures[0].stride_texels) = (8, 16);
        /* an imported image at its producer's pitch: rows of 32 bytes */
        let mut imported = draw;
        (imported.textures[0].width, imported.textures[0].stride_texels) = (8, 8);
        imported.textures[0].cpu = texel_bytes.as_ptr().cast();
        /* the kick `why` records into: the one after every case before it */
        let feedback = || {
            let mut f = imported;
            f.textures[0].target_kick = mali_kick_id();
            f
        };
        let cases: Vec<Case> = vec![
            ("a textured draw is built", why(&draw), built),
            ("no texture object on the sampler: black", why(&unbound), built),
            ("a texture on another binding only: black", why(&tex(|t| t.unit = 1)), built),
            ("incomplete: black", why(&tex(|t| t.incomplete = 1)), built),
            /* a cube map as the GL core hands it: six faces stacked, the
             * height every face's (src/gl/draw.rs); its planes are the
             * faces' (frame_tex_tests.rs `a_cube_map_s_texture_is_the_board_s`) */
            (
                "a cube map of six 16x16 faces",
                why(&tex(|t| {
                    (t.type_, t.layers, t.height, t.bytes) = (backend::GLES_TEXTYPE_CUBE, 6, 96, 16 * 96 * 4)
                })),
                built,
            ),
            (
                "a cube map's chain of 5 levels, in place",
                why(&tex(|t| {
                    (t.type_, t.layers, t.height, t.bytes) = (backend::GLES_TEXTYPE_CUBE, 6, 96, 16 * 96 * 4);
                    (t.mip_filter, t.mip_levels) = (1, 5);
                })),
                built,
            ),
            (
                "a cube map of another face count",
                why(&tex(|t| (t.type_, t.layers) = (backend::GLES_TEXTYPE_CUBE, 1))),
                not_built(NOT_BUILT_TEXTURE_TYPE),
            ),
            (
                "a 2D array",
                why(&tex(|t| (t.type_, t.layers, t.height) = (backend::GLES_TEXTYPE_2D_ARRAY, 2, 32))),
                not_built(NOT_BUILT_TEXTURE_TYPE),
            ),
            (
                "a 3D texture",
                why(&tex(|t| (t.type_, t.layers, t.height) = (backend::GLES_TEXTYPE_3D, 2, 32))),
                not_built(NOT_BUILT_TEXTURE_TYPE),
            ),
            ("L8 storage", why(&tex(|t| t.format = backend::GLES_TEXFMT_L8)), not_built(NOT_BUILT_TEXTURE_FORMAT)),
            ("BGRA order", why(&tex(|t| t.bgra = 1)), built),
            ("no alpha", why(&tex(|t| t.no_alpha = 1)), built),
            ("a depth texture as luminance", why(&tex(|t| t.depth_luminance = 1)), built),
            ("R32F storage", why(&tex(|t| t.format = backend::GLES_TEXFMT_R32F)), built),
            ("a 16x16 chain of 5 levels, in place", why(&tex(|t| (t.mip_filter, t.mip_levels) = (1, 5))), built),
            (
                "a chain of an NPOT image",
                why(&tex(|t| (t.width, t.stride_texels, t.mip_levels) = (12, 16, 2))),
                not_built(NOT_BUILT_TEXTURE_NPOT_CHAIN),
            ),
            ("mirrored repeat", why(&tex(|t| t.wrap_t = backend::GLES_WRAP_MIRRORED_REPEAT)), built),
            ("texels off 64 bytes", why(&tex(|t| t.base_gpu += 16)), invalid(frame::INVALID_TEXTURE_ALIGN)),
            ("no texels a row", why(&tex(|t| t.width = 0)), not_built(frame::NOT_BUILT_TEXTURE_SIZE)),
            ("8 texels a row on a 64-byte pitch: in place", why(&narrow), built),
            ("an imported image's rows of 32 bytes: copied", why(&imported), built),
            (
                "rows of 32 bytes and no CPU view",
                why(&tex(|t| (t.width, t.stride_texels) = (8, 8))),
                invalid(INVALID_TEXTURE_STORAGE),
            ),
            ("a copy of the open frame's own target", why(&feedback()), not_built(NOT_BUILT_TEXTURE_FEEDBACK)),
        ];
        for (what, got, want) in &cases {
            assert_eq!(got, want, "{what}");
            if let Err(Fail::Frame(FrameError::NotBuilt(l) | FrameError::Invalid(l))) = got {
                reached.push(*l);
            }
        }
    });
    // SAFETY: the texels the mock gave above; no frame holds them now.
    unsafe { m_free(&mut texels) };
    let lines = [
        NO_MALI_PROGRAM,
        INVALID_NO_TARGET,
        NOT_BUILT_TARGET_COUNT,
        frame::NOT_BUILT_SAMPLES,
        NOT_BUILT_TARGET_FORMAT,
        frame::NOT_BUILT_FLOAT_BLEND,
        INVALID_ROW_STRIDE,
        NOT_BUILT_ZS_SIZE,
        frame::NOT_BUILT_BLEND_SHADER,
        INVALID_BLEND_CODE,
        INVALID_CULL,
        INVALID_QUERY_HANDLE,
        NOT_BUILT_CLEAR_PROGRAM,
        NOT_BUILT_ATTRIBUTE,
        NOT_BUILT_ATTRIBUTE_BUFFER_SIZE,
        NOT_BUILT_ATTRIBUTE_RECORDS,
        frame::NOT_BUILT_INSTANCE_DIVISOR,
        NOT_BUILT_PRELOAD_PROGRAM,
        frame::NOT_BUILT_PRELOAD_STRIDE,
        INVALID_INDEX_SIZE,
        frame::INVALID_NO_ZS,
        NOT_BUILT_TEXTURE_FORMAT,
        NOT_BUILT_TEXTURE_TYPE,
        NOT_BUILT_TEXTURE_NPOT_CHAIN,
        INVALID_TEXTURE_STORAGE,
        NOT_BUILT_TEXTURE_FEEDBACK,
        frame::INVALID_TEXTURE_ALIGN,
        frame::NOT_BUILT_TEXTURE_SIZE,
    ];
    for line in lines {
        assert!(reached.contains(&line), "no case reaches: {line}");
    }
    /* one line per cause: no two causes share a text */
    let all = [
        &lines[..],
        &[
            NOT_BUILT_ZS_PRELOAD,
            NOT_BUILT_ZS_ELSEWHERE,
            frame::NOT_BUILT_FRAME_ROOM,
            frame::INVALID_TEXTURE,
            frame::NOT_BUILT_NO_CLEAR,
            frame::INVALID_BLEND_SHADER,
        ],
    ]
    .concat();
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b);
        }
    }
    /* the kinds map to the errno of the contract */
    assert_eq!(frame_error(FrameError::Invalid(INVALID_NO_TARGET)), -EINVAL);
    assert_eq!(frame_error(FrameError::NotBuilt(frame::NOT_BUILT_BLEND_SHADER)), -ENOTSUP);
    assert_eq!(
        frame_error(FrameError::Arena(crate::mali::arena::ArenaError::Full { label: "t", need: 1, align: 1, free: 0 })),
        -ENOMEM
    );
    // SAFETY: null is allowed.
    assert_eq!(unsafe { mali_draw(core::ptr::null()) }, -EINVAL);
}

/// Case 190's columns 3 and 4 (gl_test.c `case_blendonce`) on `over`'s
/// SRC_ALPHA, ONE_MINUS_SRC_ALPHA: GL_MIN on the colour; ONE_MINUS_DST_ALPHA,
/// ONE_MINUS_SRC_ALPHA on the colour and ONE, ZERO on alpha.
fn case_190_columns(over: &GlesDraw) -> (GlesDraw, GlesDraw) {
    use crate::shaderc::{SHC_BF_INV_DST_ALPHA, SHC_BF_ONE, SHC_BF_ZERO, SHC_BLEND_MIN};
    let mut c3 = *over;
    c3.blend_eq_rgb = SHC_BLEND_MIN as u8;
    let mut c4 = *over;
    c4.blend_src_rgb = SHC_BF_INV_DST_ALPHA as u8;
    (c4.blend_src_alpha, c4.blend_dst_alpha) = (SHC_BF_ONE as u8, SHC_BF_ZERO as u8);
    (c3, c4)
}

/// The blend shaders the frame takes from Mesa's words reach the GPU whole:
/// for case 211's 4x constant colour and case 190's columns 3 and 4, the
/// draw's blend descriptor is in mode Shader and the code at the address it
/// names (its low 32 bits, in the fragment program's 4 GiB window) is
/// tools/mali/testdata/blend_case*.words word for word; DCD0 evaluates per
/// sample at 4x only.
#[test]
fn mesa_s_blend_shader_words_are_uploaded_where_the_descriptor_points() {
    use crate::mali::genxml::v10::{Blend, DcdFlags0};
    use crate::shaderc::{SHC_BF_CONST_COLOR, SHC_BF_INV_CONST_COLOR, SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA};
    let words_of = |text: &str| -> Vec<u64> {
        text.lines().map(|l| u64::from_str_radix(l.trim(), 16).expect("a hex word")).collect()
    };
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut over = triangle(t, &VERTICES);
        (over.vs, over.fs, over.blend) = (vs, fs, true);
        (over.blend_src_rgb, over.blend_dst_rgb) = (SHC_BF_SRC_ALPHA as u8, SHC_BF_INV_SRC_ALPHA as u8);
        (over.blend_src_alpha, over.blend_dst_alpha) = (SHC_BF_SRC_ALPHA as u8, SHC_BF_INV_SRC_ALPHA as u8);
        let mut case_211 = over;
        (case_211.blend_src_rgb, case_211.blend_dst_rgb) = (SHC_BF_CONST_COLOR as u8, SHC_BF_INV_CONST_COLOR as u8);
        (case_211.blend_src_alpha, case_211.blend_dst_alpha) = (SHC_BF_CONST_COLOR as u8, SHC_BF_INV_CONST_COLOR as u8);
        (case_211.blend_color, case_211.samples) = ([0.25, 0.5, 0.75, 1.0], 4);
        let (c3, c4) = case_190_columns(&over);
        for (what, d, text) in [
            ("case 211", case_211, include_str!("../../tools/mali/testdata/blend_case211.words")),
            ("case 190 column 3", c3, include_str!("../../tools/mali/testdata/blend_case190_col3.words")),
            ("case 190 column 4", c4, include_str!("../../tools/mali/testdata/blend_case190_col4.words")),
        ] {
            let before = with(|m| m.submits.len());
            assert_eq!(draw_flush(&d), 0, "{what}");
            let stream = with(|m| m.submits[before..].last().expect("a frame").1.clone());
            let blend_va = reg_at_idvs(&stream, 50).expect("a blend descriptor") & !63;
            let bd = with(|m| m.read(blend_va, Blend::LENGTH));
            /* the board's Mesa's words 0-2 (frame_blend_tests.rs,
             * frame_msaa_tests.rs): the destination loaded and enabled, no
             * constant, no equation, mode Shader (0) */
            assert_eq!(bd[..12], [1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "{what}: the blend descriptor");
            let pc = u64::from(u32::from_le_bytes(bd[12..16].try_into().unwrap()));
            assert_eq!(pc % 128, 0, "{what}: the upload's alignment");
            let want = words_of(text);
            /* the code arena's buffer holds the program at the low 32 bits
             * the descriptor names, in its own 4 GiB window */
            let code = with(|m| {
                m.bufs
                    .iter()
                    .map(|b| (b.gpu & !0xffff_ffff) | pc)
                    .find(|&va| m.find(va, 8 * want.len()).is_some())
                    .map(|va| m.read(va, 8 * want.len()))
            })
            .unwrap_or_else(|| panic!("{what}: no buffer at the shader's address"));
            let got: Vec<u64> = code.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
            assert_eq!(got, want, "{what}: the uploaded words");
            let flags = DcdFlags0::unpack(&reg_at_idvs(&stream, 57).unwrap().to_le_bytes()[..4].try_into().unwrap());
            assert_eq!(flags.unwrap().evaluate_per_sample, d.samples == 4, "{what}: per-sample evaluation");
        }
    });
}

/// The registers each RUN_IDVS of a stream reads.
fn regs_at_each_idvs(words: &[u64]) -> Vec<std::collections::BTreeMap<u32, u32>> {
    let mut r = std::collections::BTreeMap::new();
    let mut out = Vec::new();
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                r.insert(m.destination, m.immediate);
            }
            Ok(Instr::Move48(m)) => {
                r.insert(m.destination, m.immediate as u32);
                r.insert(m.destination + 1, (m.immediate >> 32) as u32);
            }
            Ok(Instr::RunIdvs(_)) => out.push(r.clone()),
            _ => {}
        }
    }
    out
}

/// The Depth/stencil descriptor a RUN_IDVS reads (d52).
fn zsd(r: &std::collections::BTreeMap<u32, u32>) -> crate::mali::genxml::v10::DepthStencil {
    let at = u64::from(r[&52]) | u64::from(r[&53]) << 32;
    let b: [u8; 32] = with(|m| m.read(at, 32)).try_into().unwrap();
    crate::mali::genxml::v10::DepthStencil::unpack(&b).unwrap()
}

/// zs.c's frame on the mock: the window's D24S8, cleared to depth 1.0 and
/// stencil 0x5a, two depth-tested draws. One frame and one submit at the
/// flush, the two draws in it with their own ZSD (LESS, writes on), the
/// FBD with the ZS/CRC extension naming a D24S8 buffer the backend made
/// (the target's size, 4 bytes a pixel) and the clear values, the render
/// target after it. After the frame the depth plane is live: a later frame
/// that tests depth without clearing it preloads the depth alone (its
/// stencil starts from the parked clear), a colour-only frame of the same
/// target is built and leaves the buffer alone (no ZS/CRC extension), and
/// a parked depth clear is the next frame's start.
#[test]
fn a_depth_tested_frame_writes_its_d24s8_and_a_later_one_preloads_it() {
    use crate::mali::genxml::v10::{FramebufferParameters, Func, PrePostFrameShaderMode, ZsTarget};
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = with_ds(triangle(t, &VERTICES));
        (d.vs, d.fs) = (vs, fs);
        (d.depth_test, d.depth_write, d.depth_func) = (true, true, backend::GLES_CMP_LESS);
        (d.do_clear_depth, d.clear_depth, d.do_clear_stencil, d.clear_stencil) = (true, 1.0, true, 0x5a);
        let mut second = d;
        (second.do_clear, second.do_clear_depth, second.do_clear_stencil) = (false, false, false);
        let submits = with(|m| m.submits.len());
        // SAFETY: valid draws whose records keep holds.
        assert_eq!(unsafe { mali_draw(&d) }, 0);
        // SAFETY: as above.
        assert_eq!(unsafe { mali_draw(&second) }, 0);
        assert_eq!(with(|m| m.submits.len()), submits, "both draws in the open frame");
        assert!(mali_kick_pending(mali_kick_id()), "the open frame is the kick being recorded");
        assert_eq!(mali_flush(), 0);
        let words = with(|m| m.submits.last().unwrap().1.clone());
        assert_eq!(with(|m| m.submits.len()), submits + 1, "one frame");
        let regs = regs_at_each_idvs(&words);
        assert_eq!(regs.len(), 2);
        for r in &regs {
            let z = zsd(r);
            assert_eq!((z.depth_function, z.depth_write_enable, z.stencil_test_enable), (Func::LESS, true, false));
        }
        let fbd = fbd_at_fragment(&words).expect("a fragment job");
        let params = FramebufferParameters::unpack(&with(|m| m.read(fbd, 64)).try_into().unwrap()).unwrap();
        assert!(params.has_zs_crc_extension && params.z_write_enable);
        assert_eq!((params.z_clear, params.s_clear), (1.0, 0x5a));
        let ext = ZsTarget::unpack(&with(|m| m.read(fbd + 128, 64)).try_into().unwrap()).unwrap();
        assert_eq!((ext.row_stride, ext.clean_tile_write_enable), (256, true));
        assert!(with(|m| m.find(ext.base, 64 * 64 * 4).is_some()), "the D24S8 is an OSAL buffer");
        let rt = with(|m| m.read(fbd + 192, 64));
        assert_eq!(u64::from_le_bytes(rt[32..40].try_into().unwrap()), t, "the render target after the extension");
        // SAFETY: as in `fresh`.
        let planes = |g: &MaliState| (plane_of(g, d.depth_id, 0, 0), plane_of(g, d.stencil_id, 0, 1));
        // SAFETY: as above.
        assert_eq!(planes(unsafe { STATE.get() }), (Plane::LIVE, Plane::cleared(0x5a)));

        /* a colour clear and a depth test: the depth is the last frame's,
         * preloaded alone (pre frame 1, the depth-only program); the
         * stencil starts from its parked clear, so the tiles are written
         * clean */
        let mut again = d;
        (again.do_clear_depth, again.do_clear_stencil) = (false, false);
        // SAFETY: as in `fresh`.
        let g = unsafe { STATE.get() };
        assert_eq!(record_in(g, &again), Ok(()));
        let p = g.open.and_then(|o| o.frame.zs_preload).expect("a Z/S preload");
        assert_eq!((p.z, p.s, p.cleared), (true, false, true));
        assert_eq!(mali_flush(), 0);
        let words = with(|m| m.submits.last().unwrap().1.clone());
        let fbd = fbd_at_fragment(&words).expect("a fragment job");
        let params = FramebufferParameters::unpack(&with(|m| m.read(fbd, 64)).try_into().unwrap()).unwrap();
        assert_eq!(
            (params.pre_frame_0, params.pre_frame_1),
            (PrePostFrameShaderMode::NEVER, PrePostFrameShaderMode::EARLY_ZS_ALWAYS)
        );
        assert_eq!((params.z_clear, params.s_clear), (0.0, 0x5a), "the stencil's parked value; no depth clear");
        let ext = ZsTarget::unpack(&with(|m| m.read(fbd + 128, 64)).try_into().unwrap()).unwrap();
        assert!(ext.clean_tile_write_enable);
        let dcd1 = with(|m| m.read(params.frame_shader_dcds + 128, 128));
        let dcd1 = crate::mali::genxml::v10::Draw::unpack(&dcd1.try_into().unwrap()).unwrap();
        let z = crate::mali::genxml::v10::DepthStencil::unpack(
            &with(|m| m.read(dcd1.depth_stencil, 32)).try_into().unwrap(),
        )
        .unwrap();
        assert_eq!(
            (z.depth_source, z.depth_write_enable, z.stencil_test_enable, z.stencil_from_shader),
            (crate::mali::genxml::v10::DepthSource::SHADER, true, false, false),
            "the depth from the shader, the stencil left alone"
        );
        let table = with(|m| m.read(dcd1.shader.resources & !63, 64));
        let textures = crate::mali::genxml::v10::Resource::unpack(&table[48..64].try_into().unwrap()).unwrap();
        assert_eq!(textures.size, 32, "one texture, the depth view");
        // SAFETY: as above.
        assert_eq!(planes(unsafe { STATE.get() }), (Plane::LIVE, Plane::cleared(0x5a)));
        /* the colour-only draw into the same target: no extension, the
         * buffer untouched, the planes as they were */
        let mut colour = again;
        colour.depth_test = false;
        // SAFETY: as above.
        assert_eq!(record_in(unsafe { STATE.get() }, &colour), Ok(()));
        assert_eq!(mali_flush(), 0);
        let words = with(|m| m.submits.last().unwrap().1.clone());
        let fbd = fbd_at_fragment(&words).expect("a fragment job");
        let params = FramebufferParameters::unpack(&with(|m| m.read(fbd, 64)).try_into().unwrap()).unwrap();
        assert!(!params.has_zs_crc_extension && !params.z_write_enable);
        // SAFETY: as above.
        assert_eq!(planes(unsafe { STATE.get() }), (Plane::LIVE, Plane::cleared(0x5a)));
        /* a depth clear alone, with no frame open: the plane's value, no
         * kick; the next frame's depth starts there */
        let mut depth_clear = again;
        (depth_clear.index_count, depth_clear.do_clear, depth_clear.do_clear_depth) = (0, false, true);
        depth_clear.clear_depth = 0.25;
        let kicks = with(|m| m.submits.len());
        assert_eq!(draw_flush(&depth_clear), 0);
        assert_eq!(with(|m| m.submits.len()), kicks, "a parked clear is no frame");
        // SAFETY: as above.
        assert_eq!(planes(unsafe { STATE.get() }).0, Plane::cleared(0.25f32.to_bits()));
        assert_eq!(draw_flush(&again), 0);
        let words = with(|m| m.submits.last().unwrap().1.clone());
        let fbd = fbd_at_fragment(&words).expect("a fragment job");
        let params = FramebufferParameters::unpack(&with(|m| m.read(fbd, 64)).try_into().unwrap()).unwrap();
        assert_eq!((params.z_clear, params.s_clear), (0.25, 0x5a), "both planes' parked values");
    });
    assert_eq!(mali_finish(), 0);
}

/// `YakoglBackend::depth_resolve` on this backend (`mali_depth_resolve`):
/// after a frame drew depth into a depth texture's attachment (a
/// `depth_id` with bit 30), the words the "GPU" left in the pair's D24S8
/// buffer come back as R32F texels, `unorm24_depth` of each word's low 24
/// bits (the stencil byte dropped), buffer row `y` as texel row `y`, at the
/// caller's pitch, texels past the buffer and past `w` left; -EBUSY while
/// the frame that draws it is still open; 1 for an object with no plane, id
/// 0 or `w > pitch`; a parked depth clear fills its value.
#[test]
fn the_rendered_depth_of_a_texture_resolves_into_r32f_texels() {
    use crate::mali::genxml::v10::ZsTarget;
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = with_ds(triangle(t, &VERTICES));
        /* one depth-stencil texture on both attachment points */
        let tex_id = 0x4000_0000 | 7;
        (d.depth_id, d.stencil_id) = (tex_id, tex_id);
        (d.vs, d.fs) = (vs, fs);
        (d.depth_test, d.depth_write, d.depth_func) = (true, true, backend::GLES_CMP_LESS);
        (d.do_clear_depth, d.clear_depth, d.do_clear_stencil, d.clear_stencil) = (true, 1.0, true, 0);
        let (w, h) = (d.target_w, d.target_h);
        let pitch = w + 16;
        let mut texels = vec![-1.0f32; (pitch * h) as usize];
        // SAFETY: a valid draw.
        assert_eq!(unsafe { mali_draw(&d) }, 0);
        // SAFETY: `texels` holds `h` rows of `pitch` floats.
        let resolve = |texels: &mut [f32], id: u32, w: u32, h: u32, pitch: u32| unsafe {
            assert!(texels.len() >= (h * pitch) as usize);
            mali_depth_resolve(id, texels.as_mut_ptr(), w, h, pitch)
        };
        assert_eq!(resolve(&mut texels, tex_id, w, h, pitch), -EBUSY, "the open frame draws into it");
        assert_eq!(mali_flush(), 0);
        let words = with(|m| m.submits.last().unwrap().1.clone());
        let fbd = fbd_at_fragment(&words).expect("a fragment job");
        let ext = ZsTarget::unpack(&with(|m| m.read(fbd + 128, 64)).try_into().unwrap()).unwrap();
        assert_eq!(ext.row_stride, 4 * w);
        /* what the GPU would have written back: depth x + 1000 y in the low
         * 24 bits, stencil 0xa5 in the high byte */
        let word = |x: u32, y: u32| (x + 1000 * y) & 0xff_ffff;
        let mut buf = Vec::with_capacity((4 * w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                buf.extend_from_slice(&(0xa5 << 24 | word(x, y)).to_le_bytes());
            }
        }
        with(|m| m.write(ext.base, &buf));
        assert_eq!(resolve(&mut texels, tex_id, w, h, pitch), 0);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(texels[(y * pitch + x) as usize], ds_retention::unorm24_depth(word(x, y)), "({x}, {y})");
            }
            assert_eq!(texels[(y * pitch + w) as usize], -1.0, "row {y}: the texels past w are left");
        }
        assert_eq!(texels[0], 0.0);
        assert_eq!(texels[(h - 1) as usize * pitch as usize + 63], ds_retention::unorm24_depth(63 + 1000 * (h - 1)));
        /* a level larger than the buffer: the texels outside it are left */
        let (bw, bh) = (w + 8, h + 8);
        let mut big = vec![-1.0f32; (bw * bh) as usize];
        assert_eq!(resolve(&mut big, tex_id, bw, bh, bw), 0);
        assert_eq!((big[0], big[w as usize], big[(h * bw) as usize]), (0.0, -1.0, -1.0));
        /* nothing held: another object, id 0; w past the pitch */
        assert_eq!(resolve(&mut texels, 0x4000_0000 | 8, w, h, pitch), 1);
        assert_eq!(resolve(&mut texels, 0, w, h, pitch), 1);
        assert_eq!(resolve(&mut texels, tex_id, pitch + 1, h, pitch), 1);
        /* a depth clear alone parks its value: the resolve fills it */
        let mut clear = d;
        (clear.index_count, clear.do_clear, clear.do_clear_depth, clear.do_clear_stencil) = (0, false, true, false);
        clear.clear_depth = 0.25;
        assert_eq!(draw_flush(&clear), 0);
        assert_eq!(resolve(&mut texels, tex_id, w, h, pitch), 0);
        for (i, &v) in texels.iter().enumerate() {
            assert_eq!(v, if (i as u32 % pitch) < w { 0.25 } else { -1.0 }, "texel {i}");
        }
    });
    assert_eq!(mali_finish(), 0);
}

/// A draw that carries depth texels (`GlesDraw::depth_texels`: the CPU
/// wrote the depth texture after a frame drew its depth) writes them into
/// the pair's D24S8 buffer before its frame opens, depth in the low 24
/// bits with each word's stencil byte kept, and the frame preloads the
/// depth: the draw tests against the texels. `mali_depth_resolve` then
/// reads them back bit for bit (24-bit depths, as resolved texels are).
/// The same object attached in another pair, whose buffer did not hold its
/// depth, is no longer refused as held elsewhere: the texels go into that
/// pair's buffer.
///
/// Revert check: without `seed_depth` the buffer keeps the "GPU"'s words
/// and the resolve answers them.
#[test]
fn a_draw_with_depth_texels_preloads_them_from_its_buffer() {
    use crate::mali::genxml::v10::ZsTarget;
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = with_ds(triangle(t, &VERTICES));
        let tex_id = 0x4000_0000 | 9;
        (d.depth_id, d.stencil_id) = (tex_id, tex_id);
        (d.vs, d.fs) = (vs, fs);
        (d.depth_test, d.depth_write, d.depth_func) = (true, true, backend::GLES_CMP_LESS);
        (d.do_clear_depth, d.clear_depth, d.do_clear_stencil, d.clear_stencil) = (true, 1.0, true, 0);
        assert_eq!(draw_flush(&d), 0);
        let words = with(|m| m.submits.last().unwrap().1.clone());
        let fbd = fbd_at_fragment(&words).expect("a fragment job");
        let ext = ZsTarget::unpack(&with(|m| m.read(fbd + 128, 64)).try_into().unwrap()).unwrap();
        let (w, h) = (d.target_w, d.target_h);
        /* what the GPU wrote back: depth 0x123456, stencil 0xa5 */
        let gpu: Vec<u8> = (0..w * h).flat_map(|_| (0xa512_3456u32).to_le_bytes()).collect();
        with(|m| m.write(ext.base, &gpu));
        let pitch = w + 16;
        let texels: Vec<f32> =
            (0..pitch * h).map(|i| ds_retention::unorm24_depth((i * 0x1_0203 + 1) & 0xff_ffff)).collect();
        let mut again = d;
        (again.do_clear, again.do_clear_depth, again.do_clear_stencil) = (false, false, false);
        (again.depth_texels, again.depth_texels_pitch) = (texels.as_ptr(), pitch);
        // SAFETY: as in `fresh`.
        let g = unsafe { STATE.get() };
        assert_eq!(record_in(g, &again), Ok(()));
        assert!(g.open.is_some_and(|o| o.frame.zs_preload.is_some_and(|p| p.z)), "the frame preloads the depth");
        let buf = with(|m| m.read(ext.base, (4 * w * h) as usize));
        for y in 0..h {
            for x in 0..w {
                let i = (4 * (y * w + x)) as usize;
                let word = u32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
                let want = 0xa500_0000 | ds_retention::depth_unorm24(texels[(y * pitch + x) as usize]);
                assert_eq!(word, want, "({x}, {y})");
            }
        }
        assert_eq!(mali_flush(), 0);
        let mut back = vec![-1.0f32; (pitch * h) as usize];
        // SAFETY: `back` holds `h` rows of `pitch` floats.
        assert_eq!(unsafe { mali_depth_resolve(tex_id, back.as_mut_ptr(), w, h, pitch) }, 0);
        for y in 0..h {
            for x in 0..w {
                let i = (y * pitch + x) as usize;
                assert_eq!(back[i].to_bits(), texels[i].to_bits(), "({x}, {y})");
            }
        }
        /* another pair of the same object: the texels go into its buffer */
        let mut moved = again;
        moved.ds_id ^= 1;
        // SAFETY: as above.
        let g = unsafe { STATE.get() };
        assert_eq!(record_in(g, &moved), Ok(()));
        assert!(g.open.is_some_and(|o| o.frame.zs_preload.is_some_and(|p| p.z)), "preloaded in the other pair");
    });
    assert_eq!(mali_finish(), 0);
}

/// The two depth/stencil preload refusals, after a frame drew depth into
/// the window's D24S8: without the Z/S preload programs (init could not
/// build them) a frame that tests that depth again is refused
/// [`NOT_BUILT_ZS_PRELOAD`]; with them, the same depth object attached in
/// another pair (another `ds_id`, whose buffer does not hold it) is refused
/// [`NOT_BUILT_ZS_ELSEWHERE`], and in its own pair it is preloaded.
#[test]
fn a_depth_preload_without_its_program_or_its_buffer_is_refused() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = with_ds(triangle(t, &VERTICES));
        (d.vs, d.fs) = (vs, fs);
        (d.depth_test, d.depth_write, d.depth_func) = (true, true, backend::GLES_CMP_LESS);
        (d.do_clear_depth, d.clear_depth) = (true, 1.0);
        assert_eq!(draw_flush(&d), 0);
        let mut again = d;
        again.do_clear_depth = false;
        // SAFETY: as in `fresh`.
        let g = unsafe { STATE.get() };
        let programs = g.zs_preload;
        g.zs_preload = [NO_PRELOAD_PROGRAM; 3];
        assert_eq!(record_in(g, &again), Err(Fail::Frame(FrameError::NotBuilt(NOT_BUILT_ZS_PRELOAD))));
        g.zs_preload = programs;
        let _ = mali_finish();
        let mut moved = again;
        moved.ds_id ^= 1;
        // SAFETY: as above.
        let g = unsafe { STATE.get() };
        assert_eq!(record_in(g, &moved), Err(Fail::Frame(FrameError::NotBuilt(NOT_BUILT_ZS_ELSEWHERE))));
        let _ = mali_finish();
        // SAFETY: as above.
        let g = unsafe { STATE.get() };
        assert_eq!(record_in(g, &again), Ok(()));
        assert!(g.open.is_some_and(|o| o.frame.zs_preload.is_some()), "preloaded in its own pair");
    });
    assert_eq!(mali_finish(), 0);
}

/// gl_test case 207 (msaazs)'s depth frame on the mock: a 4x target with
/// its D24S8 cleared and tested is one frame whose ZS target keeps 4
/// samples on chip and averages them into the single-sampled buffer
/// (MSAA AVERAGE, as the colour's). A later 4x frame that tests that depth
/// without clearing it preloads it with the single-sampled program, as
/// Mesa does for a 4x view of a 1x image (frame_preload_tests.rs,
/// `the_depth_preload_of_a_4x_target_is_the_board_s`).
#[test]
fn a_4x_depth_frame_averages_its_d24s8_and_preloads_it() {
    use crate::mali::genxml::v10::{FramebufferParameters, Msaa, ZsTarget};
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = with_ds(triangle(t, &VERTICES));
        (d.vs, d.fs, d.samples) = (vs, fs, 4);
        (d.depth_test, d.depth_write, d.depth_func) = (true, true, backend::GLES_CMP_LESS);
        (d.do_clear_depth, d.clear_depth, d.do_clear_stencil, d.clear_stencil) = (true, 1.0, true, 0);
        assert_eq!(draw_flush(&d), 0);
        let words = with(|m| m.submits.last().unwrap().1.clone());
        let fbd = fbd_at_fragment(&words).expect("a fragment job");
        let params = FramebufferParameters::unpack(&with(|m| m.read(fbd, 64)).try_into().unwrap()).unwrap();
        assert_eq!(params.sample_count, 4);
        assert!(params.has_zs_crc_extension && params.z_write_enable);
        let ext = ZsTarget::unpack(&with(|m| m.read(fbd + 128, 64)).try_into().unwrap()).unwrap();
        assert_eq!((ext.msaa, ext.row_stride), (Msaa::AVERAGE, 256));
        let mut again = d;
        (again.do_clear_depth, again.do_clear_stencil) = (false, false);
        // SAFETY: as in `fresh`.
        let g = unsafe { STATE.get() };
        assert_eq!(record_in(g, &again), Ok(()));
        assert!(g.open.is_some_and(|o| o.frame.zs_preload.is_some()), "the depth preloaded");
    });
    assert_eq!(mali_finish(), 0);
}

/// gl_test case 162 (dskeep) on the mock: 32 framebuffers each keep the
/// depth a draw left in its own buffer, so the 33rd is refused (no slot
/// holds nothing GL keeps); once the first's depth object is gone
/// (`attachment_gone`), its buffer holds nothing GL keeps and the 33rd
/// draws. On the board the slot stayed held and the 33rd was refused again
/// (glDrawArrays -12).
#[test]
fn a_gone_depth_object_gives_its_buffer_back() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let pair = |k: u32| {
            let mut d = triangle(t, &VERTICES);
            (d.vs, d.fs) = (vs, fs);
            (d.depth_test, d.depth_write, d.depth_func) = (true, true, backend::GLES_CMP_LESS);
            (d.do_clear_depth, d.clear_depth) = (true, 1.0);
            d.ds_id = 0x100 + k;
            d.depth_id = 0x200 + k;
            (d.ds_w, d.ds_h) = (d.target_w, d.target_h);
            d
        };
        for k in 0..ds_retention::DS_SLOTS as u32 {
            assert_eq!(draw_flush(&pair(k)), 0, "pair {k} keeps its depth");
        }
        let last = pair(ds_retention::DS_SLOTS as u32);
        assert_eq!(draw_flush(&last), -ENOMEM, "every slot holds a depth GL keeps");
        mali_attachment_gone(0x200);
        assert_eq!(draw_flush(&last), 0, "the gone object's buffer is free again");
    });
    assert_eq!(mali_finish(), 0);
}

/// A masked clear after the frame's whole clear is the clear program's quad
/// in the same frame, with Mesa's state (st_cb_clear.c): the clear's mask in
/// the blend, the destination loaded, depth and stencil off; the quad's
/// colour is the fragment stage's four uniform words.
#[test]
fn a_masked_clear_is_a_quad_in_the_open_frame() {
    use crate::mali::genxml::v10::*;
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let mut clear = triangle(t, &VERTICES);
    clear.index_count = 0;
    let mut masked = clear;
    (masked.clear_color_mask, masked.clear_rgba) = (0x1, [1.0, 1.0, 1.0, 1.0]);
    // SAFETY: valid clears.
    assert_eq!(unsafe { mali_draw(&clear) }, 0);
    assert_eq!(draw_flush(&masked), 0);
    let words = with(|m| m.submits.last().unwrap().1.clone());
    let regs = regs_at_each_idvs(&words);
    assert_eq!(regs.len(), 1, "one quad");
    let at = u64::from(regs[0][&50] & !63) | u64::from(regs[0][&51]) << 32;
    /* the board's masked clear packs this (frame_zs_tests.rs compares it
     * with mesa-mask.pandecode byte for byte) */
    let replace =
        BlendFunction { a: BlendOperandA::SRC, b: BlendOperandB::SRC, c: BlendOperandC::ZERO, ..Default::default() };
    let want = Blend {
        load_destination: true,
        enable: true,
        equation: BlendEquation { rgb: replace, alpha: replace, color_mask: 1 },
        internal: InternalBlend {
            mode: BlendMode::FIXED_FUNCTION,
            fixed_function: BlendFixedFunction {
                num_comps: 4,
                rt: 0,
                conversion: InternalConversion { memory_format: Format::RGBA8_TB.0 << 12, raw: false },
            },
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(with(|m| m.read(at, Blend::LENGTH)), want.pack().unwrap());
    let z = zsd(&regs[0]);
    assert!(!z.depth_write_enable && !z.stencil_test_enable);
    assert_eq!(push_words(&words, 12)[4..], [0x3c00_3c00, 0x3c00_3c00], "white as fp16 pairs");
    /* the frame's own clear stays the first clear's blue */
    let fbd = fbd_at_fragment(&words).expect("a fragment job");
    assert_eq!(with(|m| m.read(fbd + 128 + 48, 4)), 0xffff_0000u32.to_le_bytes());
}

/// gl_test case 107 (clearload) part 1 on the mock: a clear whose scissor
/// box misses the target opens a frame from the old image and draws
/// nothing; the whole clear after it is the frame's start, so the frame
/// does not preload (Mesa preloads only what the batch does not clear,
/// `pan_preload_needed`, pan_fb_preload.c:746-760). On the board the
/// preload painted the old red over the green clear.
#[test]
fn a_whole_clear_before_any_draw_drops_the_preload() {
    use crate::mali::genxml::v10::{FramebufferParameters, PrePostFrameShaderMode};
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let mut clear = triangle(t, &VERTICES);
    clear.index_count = 0;
    let mut missed = clear;
    (missed.clear_quad, missed.scissor, missed.scissor_rect) = (true, true, [-200, 0, 50, 50]);
    let mut green = clear;
    green.clear_rgba = [0.0, 1.0, 0.0, 1.0];
    // SAFETY: valid clears.
    assert_eq!(unsafe { mali_draw(&missed) }, 0);
    assert_eq!(draw_flush(&green), 0);
    let words = with(|m| m.submits.last().unwrap().1.clone());
    assert_eq!(regs_at_each_idvs(&words).len(), 0, "the missed box draws nothing");
    let fbd = fbd_at_fragment(&words).expect("a fragment job");
    let params = FramebufferParameters::unpack(&with(|m| m.read(fbd, 64)).try_into().unwrap()).unwrap();
    assert_eq!(
        (params.pre_frame_0, params.frame_shader_dcds),
        (PrePostFrameShaderMode::NEVER, 0),
        "no preload: the frame starts from the green clear (the board drew the old red over it)"
    );
    assert_eq!(with(|m| m.read(fbd + 128 + 48, 4)), 0xff00_ff00u32.to_le_bytes(), "the clear colour is green");
}

/// An imported image to copy (rows of 32 bytes) that a submitted frame still
/// renders into is waited for before its rows are read: the second draw's
/// recording waits for the first frame's fence, and only that one.
#[test]
fn a_texture_copied_while_a_render_into_it_runs_is_waited_for() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let tex_vs = include_str!("../../tools/mali/testdata/sel_t_tex.vert");
    let tex_fs = include_str!("../../tools/mali/testdata/sel_t_tex.frag");
    let (uv, texel_bytes) = ([0.0f32; 12], vec![0x5au8; 8 * 8 * 4]);
    let (mut ma, mut mb) = (OsalGpuMem::ZERO, OsalGpuMem::ZERO);
    // SAFETY: valid out-parameters.
    unsafe {
        assert_eq!(m_alloc(0, 64 * 64 * 4, OSAL_MEM_DATA, &mut ma), 0);
        assert_eq!(m_alloc(0, 64 * 64 * 4, OSAL_MEM_DATA, &mut mb), 0);
    }
    let (a, b) = (ma.gpu, mb.gpu);
    with_programs(tex_vs, tex_fs, |vs, fs| {
        let mut draw = triangle(a, &VERTICES);
        (draw.vs, draw.fs) = (vs, fs);
        draw.num_attribs = 2;
        draw.attribs[1] = GlesAttrib { vi_reg: 1, cpu: uv.as_ptr().cast(), ..draw.attribs[0] };
        draw.num_textures = 1;
        draw.textures[0] = backend::GlesTextureBind {
            unit: 0,
            format: backend::GLES_TEXFMT_RGBA8,
            layers: 1,
            base_gpu: b,
            width: 8,
            height: 8,
            stride_texels: 8,
            bytes: 8 * 8 * 4,
            cpu: texel_bytes.as_ptr().cast(),
            ..backend::GlesTextureBind::ZERO
        };
        /* the first frame renders into A, submitted and running */
        // SAFETY: a valid draw whose records `with_programs` keeps.
        assert_eq!(unsafe { mali_draw(&draw) }, 0);
        assert_eq!(mali_flush(), 0);
        let rendered = mali_kick_id() - 1;
        let fence = with(|m| m.fences);
        assert!(mali_kick_pending(rendered));
        /* the second samples A's texels from a copy */
        let mut second = draw;
        second.target_gpu = b;
        second.textures[0].base_gpu = a;
        second.textures[0].target_kick = rendered;
        let before = with(|m| m.events.len());
        // SAFETY: as in `fresh`.
        let copies = unsafe { STATE.get() }.row_copies;
        // SAFETY: as above.
        assert_eq!(unsafe { mali_draw(&second) }, 0);
        let events = with(|m| m.events[before..].to_vec());
        assert_eq!(events, [Event::Wait(fence)], "the render into the texture is waited for, nothing else");
        // SAFETY: as in `fresh`.
        assert_eq!(unsafe { STATE.get() }.row_copies, copies + 1, "the copy is counted");
        assert!(!mali_kick_pending(rendered));
        assert_eq!(mali_finish(), 0);
    });
    // SAFETY: the two targets the mock gave above; every frame completed.
    unsafe {
        m_free(&mut ma);
        m_free(&mut mb);
    }
}

/// gl_test case 11 (persist) on the mock, its 300 draws of one program
/// after one clear made 2000, then a flush. On the board each draw uploaded
/// its programs again and the frame's 128 KiB code arena was full near draw
/// 113 ("mali arena 'code' is full", glDrawArrays -12). Now a frame keeps
/// one copy of a program (every draw's fragment SPD, d20, is its frame's
/// first), and a draw the frame has no room for (here the 1489th; the 264th
/// before the frame was sized for case 198's 1024 draws) ends it: the draws
/// go on in a second frame that preloads the first's image.
#[test]
fn many_draws_of_one_program_upload_it_once_and_go_on_in_a_second_frame() {
    use crate::mali::genxml::v10::{FramebufferParameters, PrePostFrameShaderMode};
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = triangle(t, &VERTICES);
        (d.vs, d.fs) = (vs, fs);
        let mut more = d;
        more.do_clear = false;
        let before = with(|m| m.submits.len());
        // SAFETY: valid draws whose records keep holds.
        assert_eq!(unsafe { mali_draw(&d) }, 0);
        for i in 1..2000 {
            // SAFETY: as above.
            assert_eq!(unsafe { mali_draw(&more) }, 0, "draw {i}");
        }
        assert_eq!(mali_flush(), 0);
        let frames: Vec<Vec<u64>> = with(|m| m.submits[before..].iter().map(|s| s.1.clone()).collect());
        let counts: Vec<usize> = frames.iter().map(|w| regs_at_each_idvs(w).len()).collect();
        assert_eq!(counts, [1488, 512], "two frames, the first as full as its room");
        let spd = |r: &std::collections::BTreeMap<u32, u32>| (r[&20], r[&21]);
        for w in &frames {
            let regs = regs_at_each_idvs(w);
            assert!(regs.iter().all(|r| spd(r) == spd(&regs[0])), "one fragment program a frame");
        }
        let modes: Vec<PrePostFrameShaderMode> = frames
            .iter()
            .map(|w| {
                let fbd = fbd_at_fragment(w).expect("a fragment job");
                FramebufferParameters::unpack(&with(|m| m.read(fbd, 64)).try_into().unwrap()).unwrap().pre_frame_0
            })
            .collect();
        assert_eq!(modes, [PrePostFrameShaderMode::NEVER, PrePostFrameShaderMode::INTERSECT], "the second preloads");
    });
}

/// gl_test case 198 (a2cconst) on the mock: 1024 draws with
/// alpha-to-coverage, alpha masked, into a 4x target after one clear, then
/// a flush, are one frame, as Mesa runs them on the board
/// (0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs): a split would preload the
/// resolved colour into every sample of the second frame, where the case's
/// red quads then leave green. Every draw carries DCD0's coverage bits.
#[test]
fn case_198_s_1024_alpha_to_coverage_draws_are_one_4x_frame() {
    use crate::mali::genxml::v10::DcdFlags0;
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = triangle(t, &VERTICES);
        (d.vs, d.fs, d.samples, d.alpha_to_coverage, d.color_mask) = (vs, fs, 4, true, 0x7);
        let mut more = d;
        more.do_clear = false;
        let before = with(|m| m.submits.len());
        // SAFETY: valid draws whose records keep holds.
        assert_eq!(unsafe { mali_draw(&d) }, 0);
        for i in 1..1024 {
            // SAFETY: as above.
            assert_eq!(unsafe { mali_draw(&more) }, 0, "draw {i}");
        }
        assert_eq!(mali_flush(), 0);
        let frames: Vec<Vec<u64>> = with(|m| m.submits[before..].iter().map(|s| s.1.clone()).collect());
        assert_eq!(frames.len(), 1, "one frame");
        let regs = regs_at_each_idvs(&frames[0]);
        assert_eq!(regs.len(), 1024);
        assert!(regs.iter().all(|r| {
            let f = DcdFlags0::unpack(&r[&57].to_le_bytes()).unwrap();
            f.alpha_to_coverage && f.shader_modifies_coverage && f.multisample_enable
        }));
    });
}

/// A stage's push block (its FAU register `reg`, the words it counts).
fn push_words(words: &[u64], reg: u32) -> Vec<u32> {
    let f = reg_at_idvs(words, reg).expect("a FAU register");
    let n = 2 * (f >> 56) as usize;
    with(|m| m.read(f & 0x00ff_ffff_ffff_ffff, 4 * n))
        .chunks(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

/// A buffer-object attribute is fetched in place: the attribute buffer
/// table's Buffer descriptor is the VBO's whole store (`buffer_gpu`,
/// `buffer_bytes`), and the Attribute descriptor carries the core's offset
/// into it. Nothing is copied into the frame's data arena for it.
#[test]
fn a_buffer_object_attribute_is_fetched_in_place() {
    use crate::mali::genxml::v10::*;
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let mut vbo = OsalGpuMem::ZERO;
    // SAFETY: a valid out-parameter.
    assert_eq!(unsafe { m_alloc(0, 256, OSAL_MEM_DATA, &mut vbo) }, 0);
    let words = with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = triangle(t, &VERTICES);
        (d.vs, d.fs) = (vs, fs);
        d.attribs[0] = GlesAttrib {
            vi_reg: 0,
            components: 2,
            format: GLES_ATTRIBF_FLOAT32,
            stride: 8,
            offset: 16,
            buffer_gpu: vbo.gpu,
            buffer_bytes: 256,
            ..GlesAttrib::ZERO
        };
        assert_eq!(draw_flush(&d), 0);
        with(|m| m.submits.last().unwrap().1.clone())
    });
    let srt = reg_at_idvs(&words, 0).expect("the VS SRT") & !63;
    let entry = |i: u64| u64::from_le_bytes(with(|m| m.read(srt + 16 * i, 8)).try_into().unwrap()) & ((1 << 56) - 1);
    let (attrs, bufs) = (entry(1), entry(2));
    let want_buf = Buffer { address: vbo.gpu, size: 256, ..Default::default() }.pack().unwrap();
    assert_eq!(with(|m| m.read(bufs, Buffer::LENGTH)), want_buf, "the VBO's whole store");
    let want_attr = Attribute {
        table: 2,
        frequency: AttributeFrequency::VERTEX,
        format: Format::RG32F.0 << 12 | RgbComponentOrder::RGB1.0,
        offset: 16,
        buffer_index: 0,
        stride: 8,
        attribute_type: AttributeType::_1D,
        offset_enable: true,
        ..Default::default()
    }
    .pack()
    .unwrap();
    assert_eq!(with(|m| m.read(attrs, Attribute::LENGTH)), want_attr, "the core's offset into it");
    // SAFETY: the buffer allocated above; its frame completed (finish).
    assert_eq!(mali_finish(), 0);
    unsafe { m_free(&mut vbo) };
}

/// A draw into another target ends the open frame: two frames, one per
/// target, each submitted when it ends.
#[test]
fn a_draw_into_another_target_ends_the_open_frame() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t0 = target();
    let mut second = OsalGpuMem::ZERO;
    // SAFETY: a valid out-parameter.
    assert_eq!(unsafe { m_alloc(0, 64 * 64 * 4, OSAL_MEM_DATA, &mut second) }, 0);
    let t1 = second.gpu;
    let mut a = triangle(t0, &VERTICES);
    a.index_count = 0;
    let mut b = a;
    b.target_gpu = t1;
    let before = with(|m| m.submits.len());
    // SAFETY: valid clears.
    assert_eq!(unsafe { mali_draw(&a) }, 0);
    // SAFETY: as above.
    assert_eq!(unsafe { mali_draw(&b) }, 0);
    assert_eq!(with(|m| m.submits.len()), before + 1, "a's frame ended when b began");
    assert_eq!(mali_finish(), 0);
    assert_eq!(with(|m| m.submits.len()), before + 2);
    /* the rig allows the test one target of its own after shutdown */
    // SAFETY: the buffer allocated above, no longer used.
    unsafe { m_free(&mut second) };
}

/// The framebuffer descriptor RUN_FRAGMENT reads (d40's last value before
/// it, its tag bits off).
fn fbd_at_fragment(words: &[u64]) -> Option<u64> {
    let run = words.iter().position(|&w| matches!(Instr::decode(w), Ok(Instr::RunFragment(_))))?;
    words[..run].iter().rev().filter_map(|&w| move48(w)).find(|(d, _)| *d == 40).map(|(_, v)| v & !63)
}

mod gl {
    //! The GL program of milestone L2 on the mock: a 64x64 RGBA8 FBO,
    //! glClearColor(0, 0, 1, 1) + glClear, the triangle program compiled by
    //! glCompileShader (our Valhall back end, at run time), glDrawArrays of
    //! six vertices from a client array, glReadPixels. The board program
    //! (tools/mali/panthor, `panthor-gl`) makes the same calls.
    use super::*;
    use crate::gl::consts::*;
    use crate::gl::context;
    use crate::gl::draw::glDrawArrays;
    use crate::gl::framebuffer::{
        glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glDeleteRenderbuffers,
        glFramebufferRenderbuffer, glFramebufferTexture2D, glGenFramebuffers, glGenRenderbuffers, glReadPixels,
        glRenderbufferStorage,
    };
    use crate::gl::program::{
        glAttachShader, glCreateProgram, glGetProgramInfoLog, glGetProgramiv, glLinkProgram, glUseProgram,
    };
    use crate::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
    use crate::gl::state::{glClear, glClearColor, glDepthFunc, glEnable, glGetError, glViewport};
    use crate::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
    use crate::gl::types::*;
    use crate::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
    use std::ffi::CString;

    fn shader(ty: GLenum, src: &str) -> GLuint {
        let text = CString::new(src).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(ty);
            let p = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{src} did not compile");
            s
        }
    }

    /// Run the program with `fs` as the fragment shader (None: the clear
    /// only) and hand `check` the frames submitted after init and the GL
    /// error after glReadPixels, while the mock holds the buffers (the
    /// mock's GPU draws nothing, so the pixels are not checked here).
    fn run(fs: Option<&str>, check: impl FnOnce(&[Vec<u64>], GLenum)) {
        run_with(fs.map(|f| (TRI_VS, f)), &mut |_| {}, check)
    }

    /// `run` with a vertex shader of its own, and `set` called with the
    /// program before the draw (to set its uniforms).
    fn run_with(program: Option<(&str, &str)>, set: &mut dyn FnMut(GLuint), check: impl FnOnce(&[Vec<u64>], GLenum)) {
        let _rig = Rig::new(true);
        assert_eq!(backend::init(), 0);
        // SAFETY: the rig serialises the library; plain GL calls.
        unsafe {
            let ctx = context::gl_context_create(64, 64);
            assert!(!ctx.is_null());
            let (mut fbo, mut tex) = (0, 0);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glViewport(0, 0, 64, 64);
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            if let Some((vs, fs)) = program {
                let p = glCreateProgram();
                glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
                glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
                glLinkProgram(p);
                let mut ok: GLint = 0;
                glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
                let mut log = [0u8; 512];
                let mut n: GLsizei = 0;
                glGetProgramInfoLog(p, 512, &mut n, log.as_mut_ptr().cast());
                let log = String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned();
                assert_eq!(ok, 1, "the triangle program links: {log}");
                glUseProgram(p);
                glEnableVertexAttribArray(0);
                glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, VERTICES.as_ptr().cast());
                set(p);
                glDrawArrays(GL_TRIANGLES, 0, 6);
            }
            let mut px = vec![0u8; 64 * 64 * 4];
            glReadPixels(0, 0, 64, 64, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
            let e = glGetError();
            let frames: Vec<Vec<u64>> = with(|m| m.submits[1..].iter().map(|s| s.1.clone()).collect());
            check(&frames, e);
            context::gl_context_destroy(ctx);
        }
    }

    /// The fragment program's bytes as the frame names them.
    fn fs_code(words: &[u64], len: usize) -> Vec<u8> {
        let spd = reg_at_idvs(words, 20).expect("the FS SPD");
        let bin = u64::from_le_bytes(with(|m| m.read(spd + 8, 8)).try_into().unwrap());
        with(|m| m.read(bin, len))
    }

    /// The Valhall back end's code for a fragment source.
    fn fs_bytes(fs: &str) -> Vec<u8> {
        with_programs(TRI_VS, fs, |_, f| {
            // SAFETY: the record keep holds.
            unsafe { program::code_bytes(f) }.unwrap().to_vec()
        })
    }

    /// The frame of one GL draw (its clear fused into it, submitted at
    /// glReadPixels), built from the programs glCompileShader made:
    /// the fragment program the frame names is the Valhall back end's code
    /// for tri.frag, byte for byte, and changes with the source (green: other
    /// bytes, the compiler's for that source); the render target is the
    /// FBO's texture, cleared to blue; the clear alone is a frame with no
    /// draw. No GL error in any of the three.
    #[test]
    fn a_gl_draw_runs_the_programs_glcompileshader_made() {
        let green = TRI_FS.replace("vec4(1.0, 0.0, 0.0, 1.0)", "vec4(0.0, 1.0, 0.0, 1.0)");
        assert_ne!(green, TRI_FS);
        let (red_code, green_code) = (fs_bytes(TRI_FS), fs_bytes(&green));
        assert_ne!(red_code, green_code);
        for (fs, code) in [(TRI_FS, &red_code), (green.as_str(), &green_code)] {
            run(Some(fs), |frames, e| {
                assert_eq!(e, GL_NO_ERROR);
                assert_eq!(frames.len(), 1, "one frame: the clear fused into the draw");
                let w = &frames[0];
                assert_eq!(&fs_code(w, code.len()), code);
                let fbd = fbd_at_fragment(w).expect("a fragment job");
                let rt = with(|m| m.read(fbd + 128, 64));
                let base = u64::from_le_bytes(rt[32..40].try_into().unwrap());
                assert!(with(|m| m.find(base, 64 * 64 * 4).is_some()), "the target {base:#x} is an OSAL buffer");
                assert_eq!(rt[48..52], 0xffff_0000u32.to_le_bytes());
            });
        }
        run(None, |frames, e| {
            assert_eq!(e, GL_NO_ERROR);
            assert_eq!(frames.len(), 1, "the clear alone is one frame");
            assert!(!frames[0].iter().any(|&w| matches!(Instr::decode(w), Ok(Instr::RunIdvs(_)))));
        });
    }

    /// One stage's push block as the frame names it: its FAU register
    /// (`reg`), the words it counts.
    fn push_block(words: &[u64], reg: u32) -> Vec<u32> {
        let f = reg_at_idvs(words, reg).expect("a FAU register");
        let n = 2 * (f >> 56) as usize;
        with(|m| m.read(f & 0x00ff_ffff_ffff_ffff, 4 * n))
            .chunks(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    }

    /// glUniformMatrix4fv and glUniform4f through the GL API reach the push
    /// words the frame builder fills, in the reference's FAU order: the
    /// program is the shim references sel_u_mat4 (ctl_mat4.vert: `u_mvp *
    /// a_pos`) and sel_u_color (`gl_FragColor = u_color`), whose traces
    /// put the matrix's 16 words, column-major, after the six viewport
    /// words, and the colour's four fp16 halves after the four blend
    /// constants (tools/mali/testdata/sel_u_mat4.expect.json and
    /// sel_u_color.expect.json, drawn with --uniform-values). The same
    /// program drawn again with one matrix element changed changes that
    /// one word only (each run is a context and a frame of its own).
    #[test]
    fn gl_uniforms_reach_the_push_words_in_fau_order() {
        use crate::gl::program::{glGetUniformLocation, glUniform4f, glUniformMatrix4fv};
        let vs = include_str!("../../tools/mali/testdata/ctl_mat4.vert");
        let fs = include_str!("../../tools/mali/testdata/sel_u_color.frag");
        let first: [f32; 16] = core::array::from_fn(|c| 1.5 + c as f32);
        let mut second = first;
        second[6] = -7.25;
        let draw = |m: [f32; 16]| -> (Vec<u32>, Vec<u32>) {
            let mut set = |p: GLuint| {
                // SAFETY: plain GL calls on the bound program.
                unsafe {
                    let lm = glGetUniformLocation(p, c"u_mvp".as_ptr());
                    let lc = glGetUniformLocation(p, c"u_color".as_ptr());
                    assert!(lm >= 0 && lc >= 0, "both uniforms are active");
                    glUniformMatrix4fv(lm, 1, 0, m.as_ptr());
                    glUniform4f(lc, 0.25, 0.5, 0.75, 1.0);
                    assert_eq!(crate::gl::state::glGetError(), GL_NO_ERROR, "the uniform calls");
                }
            };
            let mut out = (Vec::new(), Vec::new());
            run_with(Some((vs, fs)), &mut set, |frames, e| {
                assert_eq!(e, GL_NO_ERROR);
                assert_eq!(frames.len(), 1, "one frame: the clear fused into the draw");
                out = (push_block(&frames[0], 8), push_block(&frames[0], 12));
            });
            out
        };
        let vs_words = |m: &[f32; 16]| -> Vec<u32> {
            let (h, half) = (32.0f32.to_bits(), 0.5f32.to_bits());
            let mut v = vec![h, h, h, h, half, half];
            v.extend(m.iter().map(|x| x.to_bits()));
            v.extend([0xc700_0000, 0x4700_0000, 4, 8, 12, 0]);
            v
        };
        /* the colour as fp16 pairs: 0.25 0.5 | 0.75 1.0 */
        let fs_words = vec![0, 0, 0, 0, 0x3800_3400, 0x3c00_3a00];
        let (v0, f0) = draw(first);
        let (v1, f1) = draw(second);
        assert_eq!(v0, vs_words(&first), "the VS push words");
        assert_eq!(v1, vs_words(&second), "the VS push words, the matrix changed");
        let changed: Vec<usize> = (0..v0.len()).filter(|&i| v0[i] != v1[i]).collect();
        assert_eq!(changed, [6 + 6], "only the changed element's word");
        assert_eq!((f0, f1), (fs_words.clone(), fs_words), "the FS push words");
    }

    /// femtovg's uniform block through the GL API reaches the fragment
    /// shader's push words as Mesa's board dump has them: gl_test case 30
    /// fvgin (femtovg's vertex shader and fvgtest_inner.frag, `gl_FragColor
    /// = frag[6]`, glUniform4fv of the 14 vec4s with frag[6] = (0, 1, 0, 1)),
    /// drawn by the board's Mesa 25.0.7 into 0dce654a8:tools/mali/board-ref/
    /// 2026-10-21-s3-ubo (tools/mali/board-dump/fvgin-2026-10-21.txt: the draw's command stream, the
    /// descriptor pool and the code buffer). Mesa's fragment FAU (r12) is
    /// the four words of frag[6]; ours are those four after the four blend
    /// constants Mesa 26.2.1 always pushes first (bi_pick_blend_constants,
    /// the shim reference fvgtest_inner.expect.json), so our FAU count (r13)
    /// is 4 slots to Mesa's 2 and our fragment program reads u2/u3 where
    /// the board's reads u0/u1: the two conversions' source bytes are the
    /// one difference of the 6 words. The vertex FAU (r8) is the same ten
    /// words in both.
    #[test]
    fn femtovg_s_uniform_block_reaches_the_fragment_push_words_as_mesa_s_board_dump() {
        use crate::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
        use crate::gl::program::{glGetUniformLocation, glUniform2f, glUniform4fv};
        use crate::mali::trace;
        const EXCERPT: &str = include_str!("../../tools/mali/board-dump/fvgin-2026-10-21.txt");
        let vs = include_str!("../../tools/mali/testdata/fvgtest_inner.vert");
        let fs = include_str!("../../tools/mali/testdata/fvgtest_inner.frag");
        let mut frag = [0f32; 56];
        frag[24..28].copy_from_slice(&[0.0, 1.0, 0.0, 1.0]);
        let vbo = std::cell::Cell::new(0 as GLuint);
        let mut set = |p: GLuint| {
            // SAFETY: plain GL calls on the bound program.
            unsafe {
                /* both attributes (vertex, tcoord) from one buffer object of
                 * the quad's vertices, fetched in place: a client-pointer
                 * attribute leaves its staged copy behind at shutdown */
                let mut b = 0;
                glGenBuffers(1, &mut b);
                glBindBuffer(GL_ARRAY_BUFFER, b);
                glBufferData(GL_ARRAY_BUFFER, 4 * VERTICES.len() as isize, VERTICES.as_ptr().cast(), GL_STATIC_DRAW);
                vbo.set(b);
                glVertexAttribPointer(0, 2, GL_FLOAT, 0, 8, core::ptr::null());
                glEnableVertexAttribArray(1);
                glVertexAttribPointer(1, 2, GL_FLOAT, 0, 8, core::ptr::null());
                let lv = glGetUniformLocation(p, c"viewSize".as_ptr());
                let lf = glGetUniformLocation(p, c"frag".as_ptr());
                assert!(lv >= 0 && lf >= 0, "both uniforms are active");
                glUniform2f(lv, 64.0, 64.0);
                glUniform4fv(lf, 14, frag.as_ptr());
                assert_eq!(crate::gl::state::glGetError(), GL_NO_ERROR, "the uniform calls");
            }
        };
        let (mut vs_push, mut fs_push, mut fs_bin, mut fs_slots) = (Vec::new(), Vec::new(), Vec::new(), 0u64);
        run_with(Some((vs, fs)), &mut set, |frames, e| {
            assert_eq!(e, GL_NO_ERROR);
            assert_eq!(frames.len(), 1, "one frame: the clear fused into the draw");
            vs_push = push_block(&frames[0], 8);
            fs_push = push_block(&frames[0], 12);
            fs_slots = reg_at_idvs(&frames[0], 12).expect("the FS FAU register") >> 56;
            fs_bin = fs_code(&frames[0], 48);
            // SAFETY: the context is current; the draw's frame completed at glReadPixels.
            unsafe { glDeleteBuffers(1, &vbo.get()) };
        });
        let one = 1.0f32.to_bits();
        assert_eq!(fs_push, [0, 0, 0, 0, 0, one, 0, one], "the FS push words: blend constants, then frag[6]");
        assert_eq!(fs_slots, 4, "four 64-bit FAU slots");
        /* Mesa's draw: the vertex FAU at r8, the fragment FAU at r12, two
         * slots (r13 0x2007fff), the fragment program at code + 0x80 */
        let m = trace::memory_of(EXCERPT, "fvgin");
        let regs = regs_at_each_idvs(&trace::cs_words(EXCERPT, "fvgin").iter().map(|w| w.word).collect::<Vec<_>>());
        let r = regs.first().expect("one RUN_IDVS");
        assert_eq!((r[&9], r[&13]), (0x0400_7fff, 0x0200_7fff), "Mesa's FAU counts: VS 4 slots, FS 2");
        let words = |va: u64, n: usize| -> Vec<u32> {
            trace::bytes(&m, va, 4 * n).chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
        };
        let vs_va = 0x7fff_0000_0000 | u64::from(r[&8]);
        let fs_va = 0x7fff_0000_0000 | u64::from(r[&12]);
        /* the VS push words: the viewport's four, 0.5 and viewSize in both
         * (Mesa's y scale negated: its pbuffer is flipped, this FBO is not;
         * its eighth word a slot's padding); then 4, 8, 12, the store offsets
         * 26.2.1 promotes to the FAU (the shim's ten words) and 25.0.7
         * keeps as immediates */
        let mut theirs_vs = words(vs_va, 8);
        assert_eq!(theirs_vs[2], (-32.0f32).to_bits(), "Mesa's y scale, flipped");
        theirs_vs[2] = 32.0f32.to_bits();
        assert_eq!(theirs_vs[7], 0, "a slot's padding");
        assert_eq!(vs_push.len(), 10, "the VS push words");
        assert_eq!(theirs_vs[..7], vs_push[..7], "the VS push words are Mesa's");
        assert_eq!(vs_push[7..], [4, 8, 12], "the promoted store offsets");
        assert_eq!(words(fs_va, 4), fs_push[4..], "Mesa's FS push words are ours after the blend constants");
        let mut theirs = trace::bytes(&m, 0x7fff_ffff_c080, 48);
        assert_eq!(theirs[0..2], [0x80, 0x81], "u0.w0, u0.w1");
        assert_eq!(theirs[8..10], [0x82, 0x83], "u1.w0, u1.w1");
        for (at, w) in [0usize, 1, 8, 9].into_iter().zip([0x84u8, 0x85, 0x86, 0x87]) {
            theirs[at] = w;
        }
        assert_eq!(fs_bin, theirs, "the fragment program, the FAU operands moved past the blend constants");
    }

    /// femtovg's solid fill through the GL API reaches the descriptors as
    /// Mesa's board dump has them: fill_color.nodiscard.noscissor with
    /// femtovg's vertex shader (#1384 S4: the stroke mask `min(1, (1 -
    /// |2u - 1|) * strokeMult) * min(1, v)` scales innerCol), viewSize (64,
    /// 64), frag[6] = (0, 1, 0, 1), frag[10].x = 1, drawn by the board's
    /// Mesa 25.0.7 into 0dce654a8:tools/mali/board-ref/2026-10-21-s4-alu
    /// (tools/mali/board-dump/fvgfc-2026-10-21.txt: the draw's command stream, the descriptor pool,
    /// the code buffer and the shader program descriptors). Mesa's vertex
    /// FAU (r8, 4 slots) is case 30's: the viewport's four words, 0.5,
    /// viewSize and a pad; ours the same seven (y scale +32: an FBO, not a
    /// flipped pbuffer) then the three store offsets 26.2.1 promotes. Mesa's
    /// fragment FAU (r12, 3 slots) is frag[6], strokeMult and a pad; ours
    /// (5 slots) those six after the four blend constants 26.2.1 always
    /// pushes first, so strokeMult's FAU operand byte is Mesa's plus 4
    /// (word 2) and innerCol's four lanes are read by the same FMA each
    /// with the other pair's lane plus 4 (25.0.7 scales w and z first,
    /// 26.2.1 y and x: words 6, 8, 11, 12). Both links move `ftcoord.x *
    /// 2 - 1` into the vertex shader: the varying program's LD_ATTR, FMA
    /// and LEA_BUF words are byte-equal; 25.0.7 then stores both lanes
    /// with one STORE.i64, 26.2.1 one lane at a time with the offset from
    /// the FAU. The fragment programs share 6 words outright (the varying
    /// load, the abs/sub, the two FMIN, the mask product, the first MKVEC)
    /// and the last three (BLEND, IADD_IMM, BRANCHZI); 25.0.7 converts to
    /// half with V2F32_TO_V2F16 where 26.2.1 uses an FADD to a half (words
    /// 7, 9, 13, 14), packs the halves straight into BLEND's r0 where
    /// 26.2.1 packs into r3 and moves (word 15, ATEST's source, two MOVs
    /// 25.0.7 has not: 20 words to 22). The position program's first nine
    /// words are byte-equal (the attribute load and the viewport
    /// arithmetic); 25.0.7 stores gl_Position with one STORE.i128, 26.2.1
    /// one dword at a time at the promoted offsets.
    #[test]
    fn femtovg_s_fill_color_reaches_the_descriptors_as_mesa_s_board_dump() {
        use crate::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
        use crate::gl::program::{glGetUniformLocation, glUniform2f, glUniform4fv};
        use crate::mali::trace;
        const EXCERPT: &str = include_str!("../../tools/mali/board-dump/fvgfc-2026-10-21.txt");
        let vs = include_str!("../../tools/mali/testdata/fvg_fill_color_nodiscard_noscissor.vert");
        let fs = include_str!("../../tools/mali/testdata/fvg_fill_color_nodiscard_noscissor.frag");
        let mut frag = [0f32; 56];
        frag[24..28].copy_from_slice(&[0.0, 1.0, 0.0, 1.0]);
        frag[40] = 1.0;
        let vbo = std::cell::Cell::new(0 as GLuint);
        let mut set = |p: GLuint| {
            // SAFETY: plain GL calls on the bound program.
            unsafe {
                let mut b = 0;
                glGenBuffers(1, &mut b);
                glBindBuffer(GL_ARRAY_BUFFER, b);
                glBufferData(GL_ARRAY_BUFFER, 4 * VERTICES.len() as isize, VERTICES.as_ptr().cast(), GL_STATIC_DRAW);
                vbo.set(b);
                glVertexAttribPointer(0, 2, GL_FLOAT, 0, 8, core::ptr::null());
                glEnableVertexAttribArray(1);
                glVertexAttribPointer(1, 2, GL_FLOAT, 0, 8, core::ptr::null());
                let lv = glGetUniformLocation(p, c"viewSize".as_ptr());
                let lf = glGetUniformLocation(p, c"frag".as_ptr());
                assert!(lv >= 0 && lf >= 0, "both uniforms are active");
                glUniform2f(lv, 64.0, 64.0);
                glUniform4fv(lf, 14, frag.as_ptr());
                assert_eq!(crate::gl::state::glGetError(), GL_NO_ERROR, "the uniform calls");
            }
        };
        let (mut vs_push, mut fs_push, mut fs_slots) = (Vec::new(), Vec::new(), 0u64);
        let (mut vs_bin, mut fs_bin) = (Vec::new(), Vec::new());
        run_with(Some((vs, fs)), &mut set, |frames, e| {
            assert_eq!(e, GL_NO_ERROR);
            assert_eq!(frames.len(), 1, "one frame: the clear fused into the draw");
            vs_push = push_block(&frames[0], 8);
            fs_push = push_block(&frames[0], 12);
            fs_slots = reg_at_idvs(&frames[0], 12).expect("the FS FAU register") >> 56;
            fs_bin = fs_code(&frames[0], 22 * 8);
            let spd = reg_at_idvs(&frames[0], 16).expect("the VS SPD");
            let bin = u64::from_le_bytes(with(|m| m.read(spd + 8, 8)).try_into().unwrap());
            vs_bin = with(|m| m.read(bin, 256 + 8 * 8));
            // SAFETY: the context is current; the draw's frame completed at glReadPixels.
            unsafe { glDeleteBuffers(1, &vbo.get()) };
        });
        let one = 1.0f32.to_bits();
        assert_eq!(
            fs_push,
            [0, 0, 0, 0, 0, one, 0, one, one, 0],
            "the FS push words: blend constants, frag[6], strokeMult"
        );
        assert_eq!(fs_slots, 5, "five 64-bit FAU slots");
        let m = trace::memory_of(EXCERPT, "fvgfc");
        let regs = regs_at_each_idvs(&trace::cs_words(EXCERPT, "fvgfc").iter().map(|w| w.word).collect::<Vec<_>>());
        let r = regs.first().expect("one RUN_IDVS");
        assert_eq!((r[&9], r[&13]), (0x0400_7fff, 0x0300_7fff), "Mesa's FAU counts: VS 4 slots, FS 3");
        let words = |va: u64, n: usize| -> Vec<u32> {
            trace::bytes(&m, va, 4 * n).chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
        };
        let vs_va = 0x7fff_0000_0000 | u64::from(r[&8]);
        let fs_va = 0x7fff_0000_0000 | u64::from(r[&12]);
        let mut theirs_vs = words(vs_va, 8);
        assert_eq!(theirs_vs[2], (-32.0f32).to_bits(), "Mesa's y scale, flipped");
        theirs_vs[2] = 32.0f32.to_bits();
        assert_eq!(theirs_vs[7], 0, "a slot's padding");
        assert_eq!(vs_push.len(), 10, "the VS push words");
        assert_eq!(theirs_vs[..7], vs_push[..7], "the VS push words are Mesa's");
        assert_eq!(vs_push[7..], [4, 8, 12], "the promoted store offsets");
        assert_eq!(words(fs_va, 6), fs_push[4..], "Mesa's FS push words are ours after the blend constants");
        /* the programs: Mesa's shader program descriptors (d16 the
         * position's, d20 the fragment's) name the code buffer's 0x000 and
         * 0x100; the varying program is at 0x080 */
        let bin_of = |spd: u64| u64::from_le_bytes(trace::bytes(&m, spd + 8, 8).try_into().unwrap());
        let (vs_spd, fs_spd) = (0x7fff_0000_0000 | u64::from(r[&16]), 0x7fff_0000_0000 | u64::from(r[&20]));
        assert_eq!((bin_of(vs_spd), bin_of(fs_spd)), (0x7fff_ffff_c000, 0x7fff_ffff_c100), "Mesa's code");
        let theirs_fs = trace::bytes(&m, 0x7fff_ffff_c100, 20 * 8);
        fn fs_word(b: &[u8], i: usize) -> &[u8] {
            &b[8 * i..8 * i + 8]
        }
        let text = |b: &[u8]| {
            let mut s = String::new();
            let w = u64::from_le_bytes(b.try_into().unwrap());
            crate::shaderc::online::mali::isa::describe_word(w, &mut s).expect("a Valhall word");
            s
        };
        for i in [0usize, 1, 3, 4, 5, 10] {
            assert_eq!(fs_word(&fs_bin, i), fs_word(&theirs_fs, i), "fragment word {i} is Mesa's");
        }
        for (ours, theirs) in [(19usize, 17usize), (20, 18), (21, 19)] {
            assert_eq!(fs_word(&fs_bin, ours), fs_word(&theirs_fs, theirs), "fragment word {ours} is Mesa's {theirs}");
        }
        /* the FAU operands: strokeMult's word is Mesa's plus the four blend
         * constants (word 2, its second byte); innerCol's four lanes times
         * the mask are the same FMA each, with the other pair's lane plus
         * the four (25.0.7 scales w and z first, 26.2.1 y and x) */
        assert_eq!(fs_word(&fs_bin, 2)[1], fs_word(&theirs_fs, 2)[1] + 4, "word 2: strokeMult's FAU word");
        for (ours, theirs) in [(6usize, 11usize), (8, 12), (11, 6), (12, 8)] {
            assert_eq!(
                fs_word(&fs_bin, ours)[0],
                fs_word(&theirs_fs, theirs)[0] + 4,
                "word {ours}: Mesa's {theirs} lane"
            );
        }
        for (i, at) in [(2usize, 1usize), (6, 0), (8, 0), (11, 0), (12, 0)] {
            let (o, t) = (fs_word(&fs_bin, i), fs_word(&theirs_fs, i));
            assert!(
                o.iter().zip(t).enumerate().all(|(b, (x, y))| b == at || x == y),
                "word {i}: no other byte differs"
            );
        }
        /* the conversions to half and the registers BLEND reads: each
         * version's own form */
        for i in [7usize, 9, 13, 14] {
            assert!(text(fs_word(&theirs_fs, i)).starts_with("V2F32_TO_V2F16 r"), "Mesa's word {i}");
            assert!(text(fs_word(&fs_bin, i)).starts_with("FADD.f32 r"), "our word {i}");
        }
        assert_eq!(text(fs_word(&theirs_fs, 15)), "MKVEC.v2i16.wait0126 r0, r0^.h0, r2^.h0");
        assert_eq!(text(fs_word(&fs_bin, 15)), "MKVEC.v2i16.wait0126 r3, r0^.h0, r2.h0");
        assert_eq!(text(fs_word(&theirs_fs, 16)), "ATEST.wait @r60, r60^, r1.h1, atest_datum.w0");
        assert_eq!(text(fs_word(&fs_bin, 16)), "ATEST.wait0 @r60, r60^, r2^.h0, atest_datum.w0");
        assert_eq!(
            (text(fs_word(&fs_bin, 17)), text(fs_word(&fs_bin, 18))),
            ("MOV.i32 r0, r1^".into(), "MOV.i32.wait r1, r3^".into())
        );
        let theirs_pos = trace::bytes(&m, 0x7fff_ffff_c000, 13 * 8);
        assert_eq!(vs_bin[..9 * 8], theirs_pos[..9 * 8], "the position program's attribute load and arithmetic");
        assert!(
            text(fs_word(&theirs_pos, 12)).starts_with("STORE.i128.istream.slot2.end @r0:r1:r2:r3"),
            "25.0.7's one store"
        );
        assert!(text(&vs_bin[24 * 8..25 * 8]).starts_with("STORE.i32.istream.slot2.end @r2"), "26.2.1's fourth store");
        let theirs_var = trace::bytes(&m, 0x7fff_ffff_c080, 4 * 8);
        assert_eq!(vs_bin[256..256 + 3 * 8], theirs_var[..3 * 8], "the moved term: LD_ATTR, FMA, LEA_BUF");
        assert_eq!(text(fs_word(&theirs_var, 1)), "FMA.f32 r0, r0^, 0x40000000, 0x3F800000.neg", "ftcoord.x * 2 - 1");
        assert!(
            text(fs_word(&theirs_var, 3)).starts_with("STORE.i64.estream.slot2.end @r0:r1"),
            "25.0.7's one store of both lanes"
        );
        assert!(
            text(&vs_bin[256 + 3 * 8..256 + 4 * 8]).starts_with("STORE.i32.estream.slot2 @r0"),
            "26.2.1's first of two"
        );
    }

    /// femtovg's solid fill under a scissor through the GL API reaches the
    /// descriptors as Mesa's board dump has them: fill_color.nodiscard
    /// (#1384 S5: scissorMask's `if (scissorRadius > 0.0)` kept as real
    /// blocks, since its sides read uniforms, which bifrost's peephole may
    /// not speculate, and run past the GL front end's limit of 8), the
    /// first frame of 0dce654a8:tools/mali/board-ref/2026-10-22-s5-if/fvgfcs.c
    /// (scissorMat the translation by (-32, -32), scissorExt (20, 12),
    /// scissorScale (1, 1), scissorRadius 0, innerCol (0, 1, 0, 1),
    /// strokeMult 1, viewSize (64, 64)), drawn by the board's Mesa 25.0.7
    /// (tools/mali/board-dump/fvgfcs-2026-10-22.txt: the draw's command stream, the shader program
    /// descriptors, the code buffer and the push constants). Mesa's
    /// fragment FAU (r12, 10 slots) is the 16 words the shader reads, in
    /// the order its uses are met, then a slot's pad; ours (10 slots) those
    /// 16 after the four blend constants 26.2.1 pushes first, so every FAU
    /// operand byte of ours is Mesa's plus 4 (two slots): with that one
    /// mapping, 25 bytes over 19 words, the first 40 words of the two
    /// fragment programs are byte-equal, the two BRANCHZ with their offsets
    /// (22 and 9) and the `.reconverge` marks included: the `if` is lowered
    /// as the board's Mesa lowers it. The tail is each version's own, as
    /// 2026-10-21-s4-alu found for the noscissor program: 25.0.7 scales
    /// innerCol's w and z first and converts to half with
    /// V2F32_TO_V2F16, 26.2.1 y and x first with an FADD to a half, packs
    /// into r3 and moves; BLEND, IADD_IMM and BRANCHZI are byte-equal. The
    /// vertex FAU (r8, 4 slots) is the viewport's four words (Mesa's y
    /// scale negated: its pbuffer is flipped), 0.5, viewSize and a pad,
    /// ours then the three store offsets 26.2.1 promotes; the position
    /// program's first nine words and the varying program's first two are
    /// byte-equal, the moved term's FMA differing by the `.wait0` 25.0.7
    /// sets on it (its MOV of r0 follows at once; 26.2.1 stores r0 last).
    /// The fragment SPD's first eight bytes (r60, r61 preloaded) are the
    /// same.
    #[test]
    fn femtovg_s_scissor_if_reaches_the_descriptors_as_mesa_s_board_dump() {
        use crate::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
        use crate::gl::program::{glGetUniformLocation, glUniform2f, glUniform4fv};
        use crate::mali::trace;
        const EXCERPT: &str = include_str!("../../tools/mali/board-dump/fvgfcs-2026-10-22.txt");
        let vs = include_str!("../../tools/mali/testdata/fvg_fill_color_nodiscard.vert");
        let fs = include_str!("../../tools/mali/testdata/fvg_fill_color_nodiscard.frag");
        let mut frag = [0f32; 56];
        frag[0] = 1.0;
        frag[5] = 1.0;
        frag[8..10].copy_from_slice(&[-32.0, -32.0]);
        frag[24..28].copy_from_slice(&[0.0, 1.0, 0.0, 1.0]);
        frag[32..36].copy_from_slice(&[20.0, 12.0, 1.0, 1.0]);
        frag[40] = 1.0;
        let vbo = std::cell::Cell::new(0 as GLuint);
        let mut set = |p: GLuint| {
            // SAFETY: plain GL calls on the bound program.
            unsafe {
                let mut b = 0;
                glGenBuffers(1, &mut b);
                glBindBuffer(GL_ARRAY_BUFFER, b);
                glBufferData(GL_ARRAY_BUFFER, 4 * VERTICES.len() as isize, VERTICES.as_ptr().cast(), GL_STATIC_DRAW);
                vbo.set(b);
                glVertexAttribPointer(0, 2, GL_FLOAT, 0, 8, core::ptr::null());
                glEnableVertexAttribArray(1);
                glVertexAttribPointer(1, 2, GL_FLOAT, 0, 8, core::ptr::null());
                let lv = glGetUniformLocation(p, c"viewSize".as_ptr());
                let lf = glGetUniformLocation(p, c"frag".as_ptr());
                assert!(lv >= 0 && lf >= 0, "both uniforms are active");
                glUniform2f(lv, 64.0, 64.0);
                glUniform4fv(lf, 14, frag.as_ptr());
                assert_eq!(crate::gl::state::glGetError(), GL_NO_ERROR, "the uniform calls");
            }
        };
        let (mut vs_push, mut fs_push, mut fs_slots, mut fs_spd) = (Vec::new(), Vec::new(), 0u64, Vec::new());
        let (mut vs_bin, mut fs_bin) = (Vec::new(), Vec::new());
        run_with(Some((vs, fs)), &mut set, |frames, e| {
            assert_eq!(e, GL_NO_ERROR);
            assert_eq!(frames.len(), 1, "one frame: the clear fused into the draw");
            vs_push = push_block(&frames[0], 8);
            fs_push = push_block(&frames[0], 12);
            fs_slots = reg_at_idvs(&frames[0], 12).expect("the FS FAU register") >> 56;
            fs_bin = fs_code(&frames[0], 56 * 8);
            let spd = reg_at_idvs(&frames[0], 20).expect("the FS SPD");
            fs_spd = with(|m| m.read(spd, 8));
            let spd = reg_at_idvs(&frames[0], 16).expect("the VS SPD");
            let bin = u64::from_le_bytes(with(|m| m.read(spd + 8, 8)).try_into().unwrap());
            vs_bin = with(|m| m.read(bin, 256 + 3 * 8));
            // SAFETY: the context is current; the draw's frame completed at glReadPixels.
            unsafe { glDeleteBuffers(1, &vbo.get()) };
        });
        let m = trace::memory_of(EXCERPT, "fvgfcs");
        let regs = regs_at_each_idvs(&trace::cs_words(EXCERPT, "fvgfcs").iter().map(|w| w.word).collect::<Vec<_>>());
        let r = regs.first().expect("one RUN_IDVS");
        assert_eq!((r[&9], r[&13]), (0x0400_7fff, 0x0a00_7fff), "Mesa's FAU counts: VS 4 slots, FS 10");
        let words = |va: u64, n: usize| -> Vec<u32> {
            trace::bytes(&m, va, 4 * n).chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
        };
        let vs_va = 0x7fff_0000_0000 | u64::from(r[&8]);
        let fs_va = 0x7fff_0000_0000 | u64::from(r[&12]);
        let f = |x: f32| x.to_bits();
        let theirs_fs = words(fs_va, 20);
        assert_eq!(
            theirs_fs,
            [
                f(1.0), f(-32.0), 0, f(-32.0), f(20.0), 0, f(1.0), f(1.0), 0, f(1.0), 0, f(1.0), 0, f(1.0), f(12.0), f(1.0), 0, 0, 0, 0
            ],
            "Mesa's FS push words: scissorMat's lanes in use order, scissorExt.x, scissorRadius, scissorScale, innerCol, scissorExt.y, strokeMult, a pad"
        );
        assert_eq!(fs_slots, 10, "ten 64-bit FAU slots");
        assert_eq!(fs_push.len(), 20);
        assert_eq!(fs_push[..4], [0, 0, 0, 0], "the blend constants first");
        assert_eq!(fs_push[4..], theirs_fs[..16], "then Mesa's sixteen");
        let mut theirs_vs = words(vs_va, 8);
        assert_eq!(theirs_vs[2], (-32.0f32).to_bits(), "Mesa's y scale, flipped");
        theirs_vs[2] = 32.0f32.to_bits();
        assert_eq!(theirs_vs[7], 0, "a slot's padding");
        assert_eq!(vs_push.len(), 10, "the VS push words");
        assert_eq!(theirs_vs[..7], vs_push[..7], "the VS push words are Mesa's");
        assert_eq!(vs_push[7..], [4, 8, 12], "the promoted store offsets");
        let bin_of = |spd: u64| u64::from_le_bytes(trace::bytes(&m, spd + 8, 8).try_into().unwrap());
        let (vs_spd, fs_spd_va) = (0x7fff_0000_0000 | u64::from(r[&16]), 0x7fff_0000_0000 | u64::from(r[&20]));
        assert_eq!((bin_of(vs_spd), bin_of(fs_spd_va)), (0x7fff_ffff_c000, 0x7fff_ffff_c100), "Mesa's code");
        assert_eq!(trace::bytes(&m, fs_spd_va, 8), [0x28, 0x01, 0x00, 0x80, 0x00, 0x30, 0x00, 0x00], "Mesa's FS SPD");
        assert_eq!(fs_spd, trace::bytes(&m, fs_spd_va, 8), "our FS SPD: r60, r61 preloaded");
        let theirs = trace::bytes(&m, 0x7fff_ffff_c100, 54 * 8);
        fn word(b: &[u8], i: usize) -> &[u8] {
            &b[8 * i..8 * i + 8]
        }
        let text = |b: &[u8]| {
            let mut s = String::new();
            let w = u64::from_le_bytes(b.try_into().unwrap());
            crate::shaderc::online::mali::isa::describe_word(w, &mut s).expect("a Valhall word");
            s
        };
        /* the first 40 words: byte-equal once a FAU operand (a source byte
         * naming u0..u7, 0x80..0x8f) is moved past the blend constants */
        let mut moved = 0;
        for i in 0..40 {
            let (o, t) = (word(&fs_bin, i), word(&theirs, i));
            for (k, (x, y)) in o.iter().zip(t).enumerate() {
                if k < 3 && (0x80..0x90).contains(y) {
                    assert_eq!(*x, y + 4, "word {i} byte {k}: the FAU operand two slots up");
                    moved += 1;
                } else {
                    assert_eq!(x, y, "word {i} byte {k} is Mesa's");
                }
            }
        }
        assert_eq!(moved, 25, "the FAU operand bytes over the first 40 words");
        assert_eq!(text(word(&theirs, 7)), "BRANCHZ.eq.reconverge r1^.h0, offset:22", "to the else side");
        assert_eq!(text(word(&theirs, 29)), "BRANCHZ.eq.reconverge 0x0, offset:9", "the then side's jump");
        assert!(text(word(&theirs, 38)).starts_with("FMA.f32.reconverge r1, r1^, r2^"), "the join's first word");
        /* the tail: innerCol's lanes in each version's order, the same FMA
         * into the same register with the other pair's lane two slots up */
        for (ours, mesa) in [(40usize, 45usize), (42, 46), (45, 40), (46, 42)] {
            let (o, t) = (word(&fs_bin, ours), word(&theirs, mesa));
            assert_eq!(o[0], t[0] + 4, "word {ours}: Mesa's {mesa} lane");
            assert_eq!(o[1..], word(&theirs, ours)[1..], "word {ours}: no other byte differs");
        }
        for i in [41usize, 43, 47, 48] {
            assert!(text(word(&theirs, i)).starts_with("V2F32_TO_V2F16 r"), "Mesa's word {i}");
            assert!(text(word(&fs_bin, i)).starts_with("FADD.f32 r"), "our word {i}");
        }
        assert_eq!(text(word(&theirs, 49)), "MKVEC.v2i16.wait0126 r0, r0^.h0, r2^.h0");
        assert_eq!(text(word(&fs_bin, 49)), "MKVEC.v2i16.wait0126 r3, r0^.h0, r2.h0");
        assert_eq!(text(word(&theirs, 50)), "ATEST.wait @r60, r60^, r1.h1, atest_datum.w0");
        assert_eq!(text(word(&fs_bin, 50)), "ATEST.wait0 @r60, r60^, r2^.h0, atest_datum.w0");
        assert_eq!(
            (text(word(&fs_bin, 51)), text(word(&fs_bin, 52))),
            ("MOV.i32 r0, r1^".into(), "MOV.i32.wait r1, r3^".into())
        );
        for (ours, mesa) in [(53usize, 51usize), (54, 52), (55, 53)] {
            assert_eq!(
                word(&fs_bin, ours),
                word(&theirs, mesa),
                "word {ours} is Mesa's {mesa}: BLEND, IADD_IMM, BRANCHZI"
            );
        }
        let theirs_pos = trace::bytes(&m, 0x7fff_ffff_c000, 13 * 8);
        assert_eq!(vs_bin[..9 * 8], theirs_pos[..9 * 8], "the position program's attribute load and arithmetic");
        let theirs_var = trace::bytes(&m, 0x7fff_ffff_c080, 3 * 8);
        assert_eq!(vs_bin[256..256 + 2 * 8], theirs_var[..2 * 8], "the varying program's two attribute loads");
        assert_eq!(
            text(word(&theirs_var, 2)),
            "FMA.f32.wait0 r2, r2^, 0x40000000, 0x3F800000.neg",
            "ftcoord.x * 2 - 1"
        );
        assert_eq!(text(&vs_bin[256 + 2 * 8..256 + 3 * 8]), "FMA.f32 r2, r2^, 0x40000000, 0x3F800000.neg");
    }

    /// The fragment shader of 0dce654a8:tools/mali/board-ref/2026-10-22-s4-fragcoord
    /// (fragcoord.c, `panthor-gl --fragcoord`): gl_FragCoord.xy copied to a
    /// highp variable, times 1/64, into red and green.
    const FRAGCOORD_FS: &str =
        "precision highp float; void main(){ vec2 q = gl_FragCoord.xy; gl_FragColor = vec4(q * 0.015625, 0.0, 1.0); }";

    /// fragcoord.c's draw through the GL API (a full-screen quad, 64x48)
    /// into a 64x48 RGBA8 FBO (`fbo`) or the context's default surface,
    /// which is stored top-down (flipped): `check` gets the frame it
    /// submits while the mock holds the buffers.
    fn fragcoord_frame(fbo: bool, check: impl FnOnce(&[u64])) {
        let _rig = Rig::new(true);
        assert_eq!(backend::init(), 0);
        let quad: [f32; 12] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0];
        // SAFETY: the rig serialises the library; plain GL calls.
        unsafe {
            let ctx = context::gl_context_create(64, 48);
            assert!(!ctx.is_null());
            if fbo {
                let (mut fb, mut tex) = (0, 0);
                glGenTextures(1, &mut tex);
                glBindTexture(GL_TEXTURE_2D, tex);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as GLint,
                    64,
                    48,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null(),
                );
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            }
            let p = glCreateProgram();
            glAttachShader(
                p,
                shader(GL_VERTEX_SHADER, "attribute vec2 pos; void main(){ gl_Position = vec4(pos, 0.0, 1.0); }"),
            );
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FRAGCOORD_FS));
            crate::gl::program::glBindAttribLocation(p, 0, c"pos".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "the program links");
            glUseProgram(p);
            glViewport(0, 0, 64, 48);
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, quad.as_ptr().cast());
            glDrawArrays(GL_TRIANGLES, 0, 6);
            let mut px = vec![0u8; 64 * 48 * 4];
            glReadPixels(0, 0, 64, 48, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
            assert_eq!(glGetError(), GL_NO_ERROR);
            let frames: Vec<Vec<u64>> = with(|m| m.submits[1..].iter().map(|s| s.1.clone()).collect());
            assert_eq!(frames.len(), 1, "one frame: the clear fused into the draw");
            check(&frames[0]);
            context::gl_context_destroy(ctx);
        }
    }

    /// gl_FragCoord (#1384 S4) through the GL API reaches the descriptors
    /// as Mesa's board dumps have them (0dce654a8:tools/mali/board-ref/
    /// 2026-10-22-s4-fragcoord, fragcoord.c through the board's Mesa
    /// 25.0.7: tools/mali/board-dump/fcfbo-2026-10-22.txt into a 64x48 FBO,
    /// tools/mali/board-dump/fcpb-2026-10-22.txt into
    /// the 64x48 pbuffer, which Mesa draws flipped). The y transform
    /// `gl_FbWposYTransform` is the draw's: Mesa's fragment FAU is (1, 0)
    /// into the FBO and (-1, 48) into the pbuffer, ours the same two words
    /// into the FBO and into the top-down default surface, after the four
    /// blend constants 26.2.1 pushes first (Mesa 25.0.7 has none) and
    /// before the constant 26.2.1 promotes (1/64, which 25.0.7 makes with an
    /// IADD_IMM). The fragment shader program descriptor preloads r59, r60,
    /// r61 in both (R48-R63 0x3800). The fragment programs' first four words
    /// (the two U16_TO_F32 of r59's halves, the two FADD_IMM of the centre)
    /// are byte-equal; the fifth, the y transform's FMA, reads the FAU words
    /// four further (the blend constants): its two operand bytes differ by 4.
    #[test]
    fn gl_fragcoord_reaches_the_descriptors_as_mesa_s_board_dumps() {
        use crate::mali::trace;
        for (fbo, excerpt, source, fau_va, want) in [
            (
                true,
                include_str!("../../tools/mali/board-dump/fcfbo-2026-10-22.txt"),
                "fcfbo",
                0x7fff_fffd_2280u64,
                [1.0f32, 0.0],
            ),
            (
                false,
                include_str!("../../tools/mali/board-dump/fcpb-2026-10-22.txt"),
                "fcpb",
                0x7fff_fffd_5280u64,
                [-1.0f32, 48.0],
            ),
        ] {
            let (mut fs_push, mut ours_spd, mut fs_bin) = (Vec::new(), Vec::new(), Vec::new());
            fragcoord_frame(fbo, |frame| {
                fs_push = push_block(frame, 12);
                let spd = reg_at_idvs(frame, 20).expect("the FS SPD");
                ours_spd = with(|m| m.read(spd, 32));
                fs_bin = fs_code(frame, 5 * 8);
            });
            let m = trace::memory_of(excerpt, source);
            let regs = regs_at_each_idvs(&trace::cs_words(excerpt, source).iter().map(|w| w.word).collect::<Vec<_>>());
            let r = regs.first().expect("one RUN_IDVS");
            let theirs_fau: Vec<u32> =
                trace::bytes(&m, fau_va, 8).chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
            let theirs_spd = trace::bytes(&m, 0x7fff_0000_0000 | u64::from(r[&20]), 32);
            let theirs_fs = trace::bytes(&m, 0x7fff_ffff_c080, 5 * 8);
            let want = want.map(f32::to_bits);
            assert_eq!(theirs_fau, want, "{source}: Mesa's fragment FAU is the y transform");
            assert_eq!(r[&13] >> 24, 1, "{source}: Mesa's fragment FAU count, one slot");
            assert_eq!(
                fs_push,
                [0, 0, 0, 0, want[0], want[1], 0.015625f32.to_bits(), 0],
                "{source}: the blend constants, Mesa's two words, 1/64 and a slot's pad"
            );
            assert_eq!(ours_spd[..8], theirs_spd[..8], "{source}: the SPD's first word, R48-R63 0x3800 in it");
            assert_eq!(u16::from_le_bytes([ours_spd[4], ours_spd[5]]), 0x3800, "{source}: r59, r60, r61 preloaded");
            assert_eq!(ours_spd[16..], theirs_spd[16..], "{source}: the SPD past the binary's address");
            let word = |b: &[u8], i: usize| b[8 * i..8 * i + 8].to_vec();
            for i in [0usize, 2, 3] {
                assert_eq!(word(&fs_bin, i), word(&theirs_fs, i), "{source}: fragment word {i} is Mesa's");
            }
            /* word 1, `U16_TO_F32 r1, r59.h1`: 25.0.7 writes the half as the
             * two-bit swizzle `.h11` (bits 28-29), 26.2.1 as the one-bit lane
             * (bit 28; the shim's bytes, tests.rs) */
            let (o, t) = (word(&fs_bin, 1), word(&theirs_fs, 1));
            assert_eq!((o[3], t[3]), (0x10, 0x30), "{source}: word 1's half select");
            assert!(o.iter().zip(&t).enumerate().all(|(b, (x, y))| b == 3 || x == y), "{source}: word 1");
            /* word 4, `FMA.f32 r1, r1, T.x, T.y`: the FAU words four further */
            let (o, t) = (word(&fs_bin, 4), word(&theirs_fs, 4));
            assert_eq!((o[1], o[2]), (t[1] + 4, t[2] + 4), "{source}: word 4's FAU operands");
            assert!(o.iter().zip(&t).enumerate().all(|(b, (x, y))| b == 1 || b == 2 || x == y), "{source}: word 4");
        }
    }

    /// A texture through the GL API reaches the fragment shader's
    /// descriptors: sel_t_tex's program (`texture2D(u_tex, v_uv)`), an NxN
    /// RGBA8 texture from glTexImage2D on unit 1 named by glUniform1i, linear
    /// minification, nearest magnification, S clamped and T repeated. The
    /// fragment resource table's entry 3 is that sampler and entry 4 the
    /// texture descriptor, whose plane names the texels glTexImage2D wrote
    /// (each texel its own value), 64 bytes a row: the GL core's storage
    /// itself for N = 16 (rows of 64 bytes), a copy in the frame for N = 8
    /// and 2 (rows of 32 and 8 bytes; TEXTURE_ROW_ALIGN).
    #[test]
    fn a_gl_texture_reaches_the_fragment_shader_s_descriptors() {
        for side in [16usize, 8, 2] {
            gl_texture_of_side(side);
        }
    }

    /// A mip chain through the GL API: an NxN texture, every level from
    /// glTexImage2D with its own bytes, GL_LINEAR_MIPMAP_NEAREST. The
    /// descriptor names every level (log2 N + 1), the sampler's mipmap mode
    /// is nearest, and each plane's rows are its level's texels: the GL
    /// core's chain in place for N = 32 (level 0's pitch, 128 bytes, for
    /// every level, each on a 64-byte boundary: MIP_LEVEL_ALIGN), each level
    /// copied with 64-byte rows for N = 8.
    #[test]
    fn a_gl_mip_chain_reaches_one_plane_a_level() {
        for side in [32usize, 8] {
            gl_mip_chain_of_side(side);
        }
    }

    fn gl_mip_chain_of_side(n: usize) {
        use crate::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
        use crate::gl::program::{glGetAttribLocation, glGetUniformLocation, glUniform1i};
        use crate::gl::texture::glTexParameteri;
        use crate::mali::genxml::v10::{GenericPlane, MipmapMode, Resource, Sampler, Texture};
        let vs = include_str!("../../tools/mali/testdata/sel_t_tex.vert");
        let fs = include_str!("../../tools/mali/testdata/sel_t_tex.frag");
        let levels = n.trailing_zeros() as usize + 1;
        let level_texels =
            |l: usize| -> Vec<u8> { (0..(n >> l) * (n >> l) * 4).map(|i| (i * 5 + 17 * l + 1) as u8).collect() };
        let vertices: Vec<f32> = VERTICES.chunks(2).flat_map(|p| [p[0], p[1], 0.5, 0.5]).collect();
        let (name, vbo) = (std::cell::Cell::new(0), std::cell::Cell::new(0));
        let mut set = |p: GLuint| {
            // SAFETY: plain GL calls on the bound program; the arrays live
            // past the draw.
            unsafe {
                let mut t = 0;
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
                for l in 0..levels {
                    let (s, texels) = ((n >> l) as GLsizei, level_texels(l));
                    let f = GL_RGBA as GLint;
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        l as GLint,
                        f,
                        s,
                        s,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        texels.as_ptr().cast(),
                    );
                }
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_NEAREST as GLint);
                name.set(t);
                glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
                let mut b = 0;
                glGenBuffers(1, &mut b);
                glBindBuffer(GL_ARRAY_BUFFER, b);
                glBufferData(GL_ARRAY_BUFFER, 4 * vertices.len() as isize, vertices.as_ptr().cast(), GL_STATIC_DRAW);
                vbo.set(b);
                let a = glGetAttribLocation(p, c"a_uv".as_ptr());
                glVertexAttribPointer(0, 2, GL_FLOAT, 0, 16, core::ptr::null());
                glEnableVertexAttribArray(a as GLuint);
                glVertexAttribPointer(a as GLuint, 2, GL_FLOAT, 0, 16, 8 as *const c_void);
                assert_eq!(crate::gl::state::glGetError(), GL_NO_ERROR, "the texture calls");
            }
        };
        run_with(Some((vs, fs)), &mut set, |frames, e| {
            assert_eq!(e, GL_NO_ERROR);
            assert_eq!(frames.len(), 1, "{n}: one frame");
            let srt = reg_at_idvs(&frames[0], 4).expect("the fragment SRT") & !63;
            let table = with(|m| m.read(srt, 7 * Resource::LENGTH));
            let entry = |i: usize| Resource::unpack(table[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
            let s = Sampler::unpack(with(|m| m.read(entry(3).address, 32)).as_slice().try_into().unwrap()).unwrap();
            assert_eq!(s.mipmap_mode, MipmapMode::NEAREST);
            let t = Texture::unpack(with(|m| m.read(entry(4).address, 32)).as_slice().try_into().unwrap()).unwrap();
            assert_eq!((t.levels, t.maximum_lod), (levels as u32, (levels - 1) as f32), "{n}");
            let mut base = None;
            for l in 0..levels {
                let raw = with(|m| m.read(t.surfaces + 32 * l as u64, 32));
                let p = GenericPlane::unpack(raw.as_slice().try_into().unwrap()).unwrap();
                let s = n >> l;
                assert_eq!((p.width, p.height), (s as u32, s as u32), "{n}: level {l}");
                if n == 32 {
                    /* in place: level 0's pitch, each level on 64 bytes */
                    let b = *base.get_or_insert(p.pointer);
                    let want: u64 = (0..l).map(|k| ((128 * (n >> k)) as u64).next_multiple_of(64)).sum();
                    assert_eq!((p.row_stride, p.pointer - b), (128, want), "{n}: level {l}");
                } else {
                    assert_eq!(p.row_stride, 64, "{n}: level {l} copied with 64-byte rows");
                }
                let rows: Vec<u8> = (0..s)
                    .flat_map(|y| with(|m| m.read(p.pointer + u64::from(p.row_stride) * y as u64, 4 * s)))
                    .collect();
                assert_eq!(rows, level_texels(l), "{n}: level {l}'s rows are its glTexImage2D texels");
            }
            // SAFETY: plain GL calls; the frame has run.
            unsafe {
                glDeleteTextures(1, &name.get());
                glDeleteBuffers(1, &vbo.get());
            }
        });
    }

    /// The GL core's block and sRGB storage through the GL API reaches the
    /// descriptors as fmt.c's dumps show Mesa's (frame_tex_tests.rs): a
    /// 16x8 GL_ETC1_RGB8_OES texture (RGBA8 UNORM RGB1, clump ETC2 RGB8,
    /// swizzle RGB1, interleaved; its rows of two 32-byte block rows copied
    /// into 64-byte rows), a 16x16 GL_COMPRESSED_RGBA8_ETC2_EAC one (clump
    /// ETC2 RGBA8, rows of 64 bytes: in place) and a 16x16 GL_SRGB8_ALPHA8
    /// one (the sRGB bit, RAW32). Each plane's rows are the uploaded bytes.
    #[test]
    fn gl_block_and_srgb_storage_reach_the_descriptors() {
        use crate::mali::genxml::v10::{ClumpFormat, Format, RgbComponentOrder};
        let rgba8 = |o: RgbComponentOrder| Format::RGBA8_UNORM.0 << 12 | o.0;
        let etc1: Vec<u8> = (0..8 * 8).map(|i| (i * 3 + 1) as u8).collect();
        let etc2: Vec<u8> = (0..16 * 16).map(|i| (i * 7 + 5) as u8).collect();
        let srgb: Vec<u8> = (0..16 * 16 * 4).map(|i| (i * 11 + 2) as u8).collect();
        let cases = [
            StorageCase {
                fmt: 0x8D64,
                size: (16, 8),
                bytes: &etc1,
                word: rgba8(RgbComponentOrder::RGB1),
                clump: ClumpFormat::ETC2_RGB8,
                swizzle: frame::SWIZZLE_RGB1,
                rows: (32, 2),
            },
            StorageCase {
                fmt: 0x9278,
                size: (16, 16),
                bytes: &etc2,
                word: rgba8(RgbComponentOrder::RGBA),
                clump: ClumpFormat::ETC2_RGBA8,
                swizzle: frame::SWIZZLE_RGBA,
                rows: (64, 4),
            },
            StorageCase {
                fmt: 0x8C43,
                size: (16, 16),
                bytes: &srgb,
                word: rgba8(RgbComponentOrder::RGBA) | 1 << 20,
                clump: ClumpFormat::RAW32,
                swizzle: frame::SWIZZLE_RGBA,
                rows: (64, 16),
            },
        ];
        for c in &cases {
            gl_storage_case(c);
        }
    }

    /// One upload of `gl_block_and_srgb_storage_reach_the_descriptors`: the
    /// internal format, the size, the bytes, the descriptor words wanted and
    /// the plane's (bytes a row used, rows).
    struct StorageCase<'a> {
        fmt: GLenum,
        size: (usize, usize),
        bytes: &'a [u8],
        word: u32,
        clump: crate::mali::genxml::v10::ClumpFormat,
        swizzle: u32,
        rows: (usize, usize),
    }

    fn gl_storage_case(c: &StorageCase) {
        let StorageCase { fmt, size: (w, h), bytes, word, clump, swizzle, rows: (row, rows) } = *c;
        use crate::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
        use crate::gl::program::{glGetAttribLocation, glGetUniformLocation, glUniform1i};
        use crate::gl::texture::{glCompressedTexImage2D, glTexParameteri};
        use crate::mali::genxml::v10::{GenericPlane, Resource, Texture};
        let vs = include_str!("../../tools/mali/testdata/sel_t_tex.vert");
        let fs = include_str!("../../tools/mali/testdata/sel_t_tex.frag");
        let vertices: Vec<f32> = VERTICES.chunks(2).flat_map(|p| [p[0], p[1], 0.5, 0.5]).collect();
        let (name, vbo) = (std::cell::Cell::new(0), std::cell::Cell::new(0));
        let mut set = |p: GLuint| {
            // SAFETY: plain GL calls on the bound program; the arrays live
            // past the draw.
            unsafe {
                let mut t = 0;
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
                let (gw, gh) = (w as GLsizei, h as GLsizei);
                if fmt == 0x8C43 {
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        fmt as GLint,
                        gw,
                        gh,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        bytes.as_ptr().cast(),
                    );
                } else {
                    glCompressedTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        fmt,
                        gw,
                        gh,
                        0,
                        bytes.len() as GLsizei,
                        bytes.as_ptr().cast(),
                    );
                }
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                assert_eq!(crate::gl::state::glGetError(), GL_NO_ERROR, "{fmt:#x}: the upload");
                name.set(t);
                glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
                let mut b = 0;
                glGenBuffers(1, &mut b);
                glBindBuffer(GL_ARRAY_BUFFER, b);
                glBufferData(GL_ARRAY_BUFFER, 4 * vertices.len() as isize, vertices.as_ptr().cast(), GL_STATIC_DRAW);
                vbo.set(b);
                let a = glGetAttribLocation(p, c"a_uv".as_ptr());
                glVertexAttribPointer(0, 2, GL_FLOAT, 0, 16, core::ptr::null());
                glEnableVertexAttribArray(a as GLuint);
                glVertexAttribPointer(a as GLuint, 2, GL_FLOAT, 0, 16, 8 as *const c_void);
            }
        };
        run_with(Some((vs, fs)), &mut set, |frames, e| {
            assert_eq!(e, GL_NO_ERROR);
            assert_eq!(frames.len(), 1, "{fmt:#x}: one frame");
            let srt = reg_at_idvs(&frames[0], 4).expect("the fragment SRT") & !63;
            let table = with(|m| m.read(srt, 7 * Resource::LENGTH));
            let te = Resource::unpack(table[64..80].try_into().unwrap()).unwrap();
            let t = Texture::unpack(with(|m| m.read(te.address, 32)).as_slice().try_into().unwrap()).unwrap();
            assert_eq!(
                (t.format, t.swizzle, t.texel_interleave),
                (word, swizzle, clump != ClumpFormat::RAW32),
                "{fmt:#x}"
            );
            let p = GenericPlane::unpack(with(|m| m.read(t.surfaces, 32)).as_slice().try_into().unwrap()).unwrap();
            assert_eq!((p.clump_format, p.row_stride, p.width, p.height), (clump, 64, w as u32, h as u32), "{fmt:#x}");
            let got: Vec<u8> = (0..rows).flat_map(|y| with(|m| m.read(p.pointer + 64 * y as u64, row))).collect();
            assert_eq!(got, bytes, "{fmt:#x}: the plane's rows are the uploaded bytes");
            // SAFETY: plain GL calls; the frame has run.
            unsafe {
                glDeleteTextures(1, &name.get());
                glDeleteBuffers(1, &vbo.get());
            }
        });
    }

    fn gl_texture_of_side(n: usize) {
        use crate::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
        use crate::gl::program::{glGetAttribLocation, glGetUniformLocation, glUniform1i};
        use crate::gl::texture::{glActiveTexture, glTexParameteri};
        use crate::mali::genxml::v10::{GenericPlane, Resource, Sampler, Texture, WrapMode};
        let vs = include_str!("../../tools/mali/testdata/sel_t_tex.vert");
        let fs = include_str!("../../tools/mali/testdata/sel_t_tex.frag");
        let texels: Vec<u8> = (0..n * n * 4).map(|i| (i * 7 + 3) as u8).collect();
        /* both attributes from one buffer object, (x, y, s, t) a vertex: a
         * client array would be staged by the GL core for the draw */
        let vertices: Vec<f32> = VERTICES.chunks(2).flat_map(|p| [p[0], p[1], 0.5, 0.5]).collect();
        let (name, vbo) = (std::cell::Cell::new(0), std::cell::Cell::new(0));
        let mut set = |p: GLuint| {
            // SAFETY: plain GL calls on the bound program; the arrays live
            // past the draw.
            unsafe {
                let mut t = 0;
                glActiveTexture(GL_TEXTURE1);
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as GLint,
                    n as GLsizei,
                    n as GLsizei,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    texels.as_ptr().cast(),
                );
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT as GLint);
                name.set(t);
                glActiveTexture(GL_TEXTURE0);
                glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 1);
                let mut b = 0;
                glGenBuffers(1, &mut b);
                glBindBuffer(GL_ARRAY_BUFFER, b);
                glBufferData(GL_ARRAY_BUFFER, 4 * vertices.len() as isize, vertices.as_ptr().cast(), GL_STATIC_DRAW);
                vbo.set(b);
                let a = glGetAttribLocation(p, c"a_uv".as_ptr());
                assert!(a >= 0, "a_uv is active");
                glVertexAttribPointer(0, 2, GL_FLOAT, 0, 16, core::ptr::null());
                glEnableVertexAttribArray(a as GLuint);
                glVertexAttribPointer(a as GLuint, 2, GL_FLOAT, 0, 16, 8 as *const c_void);
                assert_eq!(crate::gl::state::glGetError(), GL_NO_ERROR, "the texture calls");
            }
        };
        run_with(Some((vs, fs)), &mut set, |frames, e| {
            assert_eq!(e, GL_NO_ERROR);
            assert_eq!(frames.len(), 1, "one frame: the clear fused into the draw");
            let srt = reg_at_idvs(&frames[0], 4).expect("the fragment SRT") & !63;
            let table = with(|m| m.read(srt, 7 * Resource::LENGTH));
            let entry = |i: usize| Resource::unpack(table[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
            let (se, te) = (entry(3), entry(4));
            assert_eq!((se.size, te.size), (32, 32), "one sampler, one texture");
            let s = Sampler::unpack(with(|m| m.read(se.address, 32)).as_slice().try_into().unwrap()).unwrap();
            assert_eq!((s.minify_nearest, s.magnify_nearest), (false, true));
            assert_eq!((s.wrap_mode_s, s.wrap_mode_t), (WrapMode::CLAMP_TO_EDGE, WrapMode::REPEAT));
            let t = Texture::unpack(with(|m| m.read(te.address, 32)).as_slice().try_into().unwrap()).unwrap();
            assert_eq!((t.width, t.height, t.levels), (n as u32, n as u32, 1));
            let p = GenericPlane::unpack(with(|m| m.read(t.surfaces, 32)).as_slice().try_into().unwrap()).unwrap();
            assert_eq!((p.width, p.height, p.row_stride, p.size), (n as u32, n as u32, 64, n as u32 * 64), "{n}x{n}");
            let rows: Vec<u8> = (0..n).flat_map(|y| with(|m| m.read(p.pointer + 64 * y as u64, 4 * n))).collect();
            assert_eq!(rows, texels, "{n}x{n}: the plane's rows are glTexImage2D's texels");
            // SAFETY: a plain GL call; the frame has run.
            unsafe {
                glDeleteTextures(1, &name.get());
                glDeleteBuffers(1, &vbo.get());
            }
        });
    }

    /// What an attachment's going does to the open frame. The GL core calls
    /// the backend (`texture_released` before it frees texel storage,
    /// `attachment_gone` for a depth/stencil object) and frees the storage
    /// at once. The backend ends the open frame there (submit, no wait),
    /// so the draws into the attachment still run. The OSAL holds the
    /// freed memory until the kick being recorded at the free has completed
    /// (the mock keeps gpu_free's contract as tools/mali/panthor's OSAL
    /// does), so the order is: the frame's submit, the free, the frame's
    /// completion, the release. Checked for the colour texture
    /// (glDeleteTextures), the depth/stencil renderbuffer
    /// (glDeleteRenderbuffers) and the share group's release (the last
    /// context destroyed).
    #[test]
    fn an_open_frame_runs_before_its_attachment_is_released() {
        #[derive(Clone, Copy, PartialEq, Debug)]
        enum Gone {
            Texture,
            Renderbuffer,
            Group,
        }
        for gone in [Gone::Texture, Gone::Renderbuffer, Gone::Group] {
            let _rig = Rig::new(true);
            assert_eq!(backend::init(), 0);
            // SAFETY: the rig serialises the library; plain GL calls.
            unsafe {
                let ctx = context::gl_context_create(64, 64);
                assert!(!ctx.is_null());
                let (mut fbo, mut tex, mut rb) = (0, 0, 0);
                glGenTextures(1, &mut tex);
                glBindTexture(GL_TEXTURE_2D, tex);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as GLint,
                    64,
                    64,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null(),
                );
                glGenFramebuffers(1, &mut fbo);
                glBindFramebuffer(GL_FRAMEBUFFER, fbo);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8_OES, 64, 64);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
                glViewport(0, 0, 64, 64);
                glClearColor(0.0, 0.0, 1.0, 1.0);
                glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
                let p = glCreateProgram();
                glAttachShader(p, shader(GL_VERTEX_SHADER, TRI_VS));
                glAttachShader(p, shader(GL_FRAGMENT_SHADER, TRI_FS));
                glLinkProgram(p);
                glUseProgram(p);
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LESS);
                glEnableVertexAttribArray(0);
                glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, VERTICES.as_ptr().cast());
                glDrawArrays(GL_TRIANGLES, 0, 6);
                assert_eq!(glGetError(), GL_NO_ERROR);
                let target = STATE.get().open.map(|o| o.target.gpu).expect("the draw's frame is open");
                let (before, submits) = with(|m| (m.events.len(), m.fences));
                match gone {
                    Gone::Texture => glDeleteTextures(1, &tex),
                    Gone::Renderbuffer => glDeleteRenderbuffers(1, &rb),
                    Gone::Group => context::gl_context_destroy(ctx),
                }
                let frame = submits + 1;
                assert!(STATE.get().open.is_none(), "{gone:?}: no frame left open");
                /* its draws ran: the frame submitted holds the draw */
                let (stream, freed) = with(|m| {
                    let w = m.submits.last().unwrap().1.clone();
                    (w, m.events[before..].iter().any(|e| matches!(e, Event::Free(_))))
                });
                assert!(
                    stream.iter().any(|&w| matches!(Instr::decode(w), Ok(Instr::RunIdvs(_)))),
                    "{gone:?}: the draw ran"
                );
                /* the next kick completes what was held for it: a frame of
                 * another target */
                let t2 = super::target();
                let mut other = triangle(t2, &VERTICES);
                other.index_count = 0;
                assert_eq!(draw_flush(&other), 0);
                assert_eq!(mali_finish(), 0);
                let after: Vec<Event> = with(|m| m.events[before..].to_vec());
                let at = |e: Event| after.iter().position(|&x| x == e);
                let (sub, wait) = (at(Event::Submit(frame)), at(Event::Wait(frame)));
                let first_free = after.iter().position(|e| matches!(e, Event::Free(_)));
                let first_release = after.iter().position(|e| matches!(e, Event::Release(_)));
                assert!(sub.is_some() && wait.is_some(), "{gone:?}: the frame was submitted and completed: {after:?}");
                if gone == Gone::Renderbuffer {
                    /* the D24S8 renderbuffer holds no GPU storage of the GL
                     * core's on this backend (the backend keeps the depth
                     * and stencil itself): nothing is freed there */
                    assert!(!freed, "{gone:?}: nothing to free: {after:?}");
                } else {
                    assert!(sub < first_free, "{gone:?}: submitted before the free: {after:?}");
                    assert!(first_free < wait, "{gone:?}: freed before the frame completed (no wait): {after:?}");
                    assert!(wait < first_release, "{gone:?}: released only after the frame completed: {after:?}");
                }
                if gone == Gone::Texture {
                    let (free_t, rel_t) = (at(Event::Free(target)), at(Event::Release(target)));
                    assert!(free_t.is_some() && free_t < rel_t, "{gone:?}: the target itself went back: {after:?}");
                }
                let mut t2m = OsalGpuMem { gpu: t2, ..OsalGpuMem::ZERO };
                t2m.impl_ = with(|m| m.bufs.iter().find(|b| b.gpu == t2).map(|b| b.id)).unwrap() as *mut c_void;
                m_free(&mut t2m);
                if gone != Gone::Group {
                    context::gl_context_destroy(ctx);
                }
            }
        }
    }
}

/// Each RUN_IDVS's index buffer (d54), in stream order.
fn index_buffers(words: &[u64]) -> Vec<u64> {
    let mut d54 = [0u32; 2];
    let mut out = Vec::new();
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move48(m)) if m.destination == 54 => d54 = [m.immediate as u32, (m.immediate >> 32) as u32],
            Ok(Instr::RunIdvs(_)) => out.push(u64::from(d54[0]) | u64::from(d54[1]) << 32),
            _ => {}
        }
    }
    out
}

/// Staged indices live in one scratch the GL core rewrites for every draw
/// (`GlesDraw::index_cpu`), and the open frame runs its draws later: each
/// draw's indices are copied into the frame, so two draws of one frame
/// read their own, as Mesa copies user indices into the batch's pool
/// (pan_helpers.c:92-99). Indices in an element buffer are read in place.
#[test]
fn staged_indices_are_the_draw_s_own_and_an_element_buffer_is_read_in_place() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let mut scratch = [0u16; 6];
    let words = with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = triangle(t, &VERTICES);
        (d.vs, d.fs) = (vs, fs);
        (d.index_buf_gpu, d.index_size, d.index_cpu, d.index_bytes) = (0x61_0000, 2, scratch.as_ptr().cast(), 12);
        for first in [[0u16, 1, 2, 3, 4, 5], [5, 4, 3, 2, 1, 0]] {
            scratch = first;
            // SAFETY: a valid draw whose records keep holds; `scratch`
            // outlives the call.
            assert_eq!(unsafe { mali_draw(&d) }, 0);
            /* the frame is open: the later draws do not clear (a clear
             * after a draw is the clear quad, a draw of its own) */
            d.do_clear = false;
        }
        let mut e = d;
        (e.index_cpu, e.index_bytes) = (core::ptr::null(), 0);
        // SAFETY: as above.
        assert_eq!(unsafe { mali_draw(&e) }, 0);
        assert_eq!(mali_flush(), 0);
        with(|m| m.submits.last().cloned().unwrap().1)
    });
    let at = index_buffers(&words);
    assert_eq!(at.len(), 3, "three draws in one frame");
    let read = |gpu: u64| with(|m| m.read(gpu, 12));
    let bytes = |i: [u16; 6]| i.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
    assert_eq!(read(at[0]), bytes([0, 1, 2, 3, 4, 5]), "the first draw's own indices");
    assert_eq!(read(at[1]), bytes([5, 4, 3, 2, 1, 0]), "the second draw's own indices");
    assert!(at[0] != 0x61_0000 && at[1] != 0x61_0000 && at[0] != at[1], "copies, not the scratch");
    assert_eq!(at[2], 0x61_0000, "an element buffer, in place");
}

/// The first reason is kept whole and a later one does not replace it: the
/// verdict row of panthor-gltest names the reason the case met first, with
/// no cut at the log line limit (the string here is longer than it).
#[test]
fn the_first_refusal_is_kept_whole() {
    let r = FirstRefusal::new();
    assert_eq!(r.get(), None, "nothing refused yet");
    let long: &'static str = Box::leak("x".repeat(crate::osal::OSAL_LOG_LINE_MAX * 2).into_boxed_str());
    r.note(long);
    r.note(frame::NOT_BUILT_INSTANCE_DIVISOR);
    assert_eq!(r.get(), Some(long));
}

/// An occlusion query end to end on the mock, as Mesa's panfrost does it
/// on v10 (0dce654a8:tools/mali/board-ref/2026-10-20-query): `query_begin` zeroes
/// the handle's counter block and puts nothing in the stream; a draw
/// recorded while it counts names the block in `OQ` (d46) with DCD0's
/// Occlusion Query Predicate, an uncounted draw writes neither; once the
/// frame ran, `query_result` sums the block as the hardware left it (the
/// board's: 1 in the first qword of a query a sample passed).
#[test]
fn an_occlusion_query_names_its_counter_and_reads_it_back() {
    let _rig = Rig::new(true);
    assert_eq!(mali_query_begin(3), -ENODEV, "before init");
    assert_eq!(mali_init(), 0);
    let counters = with(|m| m.bufs[13].gpu);
    let block = counters + 3 * 512;
    /* a block a past query left counted */
    with(|m| m.write(block, &[0xa5; 512]));
    assert_eq!(mali_query_begin(3), 0);
    assert_eq!(with(|m| m.read(block, 512)), [0u8; 512], "zeroed");
    assert_eq!(with(|m| m.submits.len()), 1, "nothing submitted: the queue init only");
    for bad in [0, backend::GLES_QUERY_HANDLES] {
        assert_eq!(mali_query_begin(bad), -EINVAL, "handle {bad}");
        // SAFETY: a valid out-parameter.
        assert_eq!(unsafe { mali_query_result(bad, &mut 0) }, -EINVAL, "handle {bad}");
    }
    let t = target();
    let words = with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = triangle(t, &VERTICES);
        (d.vs, d.fs) = (vs, fs);
        let counted = GlesDraw { occlusion_query: 3, do_clear: false, ..d };
        // SAFETY: valid draws whose records keep holds.
        unsafe {
            assert_eq!(mali_draw(&d), 0);
            assert_eq!(mali_draw(&counted), 0);
        }
        assert_eq!(mali_flush(), 0);
        with(|m| m.submits.last().cloned().unwrap().1)
    });
    let mut at_idvs = Vec::new();
    let (mut oq, mut dcd0) = (None, 0);
    for &w in &words {
        match Instr::decode(w) {
            Ok(Instr::Move48(m)) if m.destination == 46 => oq = Some(m.immediate),
            Ok(Instr::Move32(m)) if m.destination == 57 => dcd0 = m.immediate,
            Ok(Instr::RunIdvs(_)) => at_idvs.push((oq.take(), dcd0)),
            _ => {}
        }
    }
    assert_eq!(at_idvs.len(), 2, "two draws in one frame");
    assert_eq!((at_idvs[0].0, at_idvs[0].1 >> 14 & 3), (None, 0), "the uncounted draw");
    assert_eq!((at_idvs[1].0, at_idvs[1].1 >> 14 & 3), (Some(block), 1), "the counted draw: PREDICATE");
    let result = |rc| {
        let mut n = 7u64;
        // SAFETY: a valid out-parameter.
        assert_eq!(unsafe { mali_query_result(3, &mut n) }, rc);
        n
    };
    assert_eq!(result(0), 0, "no sample passed");
    /* what the board wrote for q1 (visible): 1 in qword 0 */
    with(|m| m.write(block, &1u64.to_le_bytes()));
    assert_eq!(result(0), 1);
    /* a counter on more than one core adds up (COUNTER mode, which ES 3.0's
     * targets never ask for, writes one qword a core) */
    with(|m| m.write(block + 18 * 8, &2u64.to_le_bytes()));
    assert_eq!(result(0), 3);
    /* the next query on the handle starts from 0 again */
    assert_eq!(mali_query_begin(3), 0);
    assert_eq!(result(0), 0);
    // SAFETY: null is allowed.
    assert_eq!(unsafe { mali_query_result(3, core::ptr::null_mut()) }, 0);
}

/// The first attribute buffer of each draw of a stream: its Buffer
/// descriptor's (address, size), from the VS resource table (d0) entry 2.
fn attribute_buffers(words: &[u64]) -> Vec<(u64, u32)> {
    use crate::mali::genxml::v10::Buffer;
    regs_at_each_idvs(words)
        .iter()
        .map(|r| {
            let srt = (u64::from(r[&0]) | u64::from(r[&1]) << 32) & !63;
            let table = u64::from_le_bytes(with(|m| m.read(srt + 32, 8)).try_into().unwrap()) & ((1 << 56) - 1);
            let raw = with(|m| m.read(table, Buffer::LENGTH));
            let size = u32::from_le_bytes(raw[4..8].try_into().unwrap());
            let address = u64::from_le_bytes(raw[8..16].try_into().unwrap());
            assert_eq!(raw, Buffer { address, size, ..Default::default() }.pack().unwrap(), "a Simple buffer");
            (address, size)
        })
        .collect()
}

/// `n` vec2 vertices, each draw's own (`seed`), as the GL core stages a
/// client array.
fn client_array(n: usize, seed: f32) -> Vec<f32> {
    (0..2 * n).map(|i| seed + (i % 997) as f32 / 1024.0).collect()
}

fn bytes_of_f32(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// A draw of `vertices` (vec2 each) staged as a client array.
fn client_draw(t: u64, vertices: &[f32]) -> GlesDraw {
    let mut d = triangle(t, &VERTICES);
    d.index_count = (vertices.len() / 2) as u32;
    d.attribs[0].cpu = vertices.as_ptr().cast();
    d.attribs[0].cpu_bytes = (4 * vertices.len()) as u32;
    d
}

/// gl_test case 197 (vtxgrid) stages more client vertex data for one draw
/// than the frame's first data chunk holds (516,128 bytes against 256 KiB;
/// the board's "mali arena 'data' is full", 0dce654a8:tools/mali/board-ref/
/// 2026-10-23-tex-coord). The data pool grows by a chunk that holds the
/// block, as Mesa's pool and stream uploader take a new BO
/// (0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/mesa-vtxgrid.pandecode:
/// the draw's arrays in a BO of their own): the draw is in the frame, its
/// Buffer descriptor names the copy in the new chunk, and the copy is the
/// draw's bytes. The chunk is the slot's from then on: the slot's next
/// frame takes no more memory and puts the data where the first did.
#[test]
fn a_draw_whose_client_data_passes_the_first_data_chunk_grows_the_pool() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let n = 40_002;
    let v = client_array(n, 0.25);
    assert!(4 * v.len() > DATA_BYTES);
    let data0 = with(|m| m.bufs[4].gpu);
    let mut frames = Vec::new();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = client_draw(t, &v);
        (d.vs, d.fs) = (vs, fs);
        for _ in 0..3 {
            let allocs = with(|m| m.allocs);
            assert_eq!(draw_flush(&d), 0);
            let (words, grown) = with(|m| {
                let grown: Vec<(u64, usize)> =
                    m.bufs.iter().filter(|b| b.id > allocs).map(|b| (b.gpu, b.size)).collect();
                (m.submits.last().unwrap().1.clone(), grown)
            });
            frames.push((attribute_buffers(&words), grown));
        }
    });
    let want = bytes_of_f32(&v);
    for (k, (bufs, _)) in frames.iter().enumerate() {
        assert_eq!(bufs.len(), 1, "frame {k}: the draw is in it");
        let (gpu, size) = bufs[0];
        assert_eq!(size as usize, want.len());
        assert_ne!(gpu, data0, "not in the first chunk");
        assert_eq!(with(|m| m.read(gpu, want.len())), want, "frame {k}: the draw's bytes");
    }
    /* one chunk for each slot, of the block rounded up to pages; slot 0's
     * second frame (the third) reuses its chunk */
    let chunk = (want.len() + 15).next_multiple_of(4096);
    assert_eq!(frames[0].1, [(frames[0].0[0].0, chunk)]);
    assert_eq!(frames[1].1, [(frames[1].0[0].0, chunk)]);
    assert_eq!(frames[2].1, []);
    assert_eq!(frames[2].0[0].0, frames[0].0[0].0);
    assert_eq!(mali_finish(), 0);
}

/// Several draws whose client data together pass the first data chunk are
/// one frame, not two (a 4x frame must not split): the draws that fit stay
/// in the first chunk, where they were, and the next goes to a grown chunk
/// of the slab's size (the first chunk's, `MAX2(slab_size, sz)`); every
/// draw reads its own bytes.
#[test]
fn draws_that_together_pass_the_first_data_chunk_stay_one_frame() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let arrays: Vec<Vec<f32>> = (0..4).map(|k| client_array(12_000, k as f32)).collect();
    assert!(arrays.iter().map(|a| 4 * a.len()).sum::<usize>() > DATA_BYTES);
    let data0 = with(|m| m.bufs[4].gpu);
    let (words, grown) = with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let allocs = with(|m| m.allocs);
        let before = with(|m| m.submits.len());
        for (k, a) in arrays.iter().enumerate() {
            let mut d = client_draw(t, a);
            (d.vs, d.fs, d.do_clear) = (vs, fs, k == 0);
            // SAFETY: a valid draw whose records keep holds.
            assert_eq!(unsafe { mali_draw(&d) }, 0, "draw {k}");
        }
        assert_eq!(mali_flush(), 0);
        assert_eq!(with(|m| m.submits.len()), before + 1, "one frame");
        with(|m| {
            let grown: Vec<(u64, usize)> = m.bufs.iter().filter(|b| b.id > allocs).map(|b| (b.gpu, b.size)).collect();
            (m.submits.last().unwrap().1.clone(), grown)
        })
    });
    let bufs = attribute_buffers(&words);
    assert_eq!(bufs.len(), 4);
    assert_eq!(grown.len(), 1, "one chunk grown");
    assert_eq!(grown[0].1, DATA_BYTES, "of the slab's size");
    /* the first two in the first chunk, from its start; then the chunk */
    assert_eq!(bufs[0].0, data0);
    assert!((data0..data0 + DATA_BYTES as u64).contains(&bufs[1].0));
    assert_eq!(bufs[2].0, grown[0].0);
    assert!((grown[0].0..grown[0].0 + DATA_BYTES as u64).contains(&bufs[3].0));
    for (k, (a, &(gpu, size))) in arrays.iter().zip(&bufs).enumerate() {
        let want = bytes_of_f32(a);
        assert_eq!(size as usize, want.len());
        assert_eq!(with(|m| m.read(gpu, want.len())), want, "draw {k}'s bytes");
    }
    assert_eq!(mali_finish(), 0);
}

/// A frame whose data fits the first chunk takes no more memory, and its
/// data starts at the chunk's first byte, as the single data arena put it
/// (the host proof of `a_draw_submits_the_frame_the_builder_makes_at_the_osal_s_vas`
/// pins the whole frame).
#[test]
fn a_frame_that_fits_its_first_data_chunk_takes_no_more_memory() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    let data0 = with(|m| m.bufs[4].gpu);
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let v = client_array(30_000, 0.5);
        assert!(4 * v.len() < DATA_BYTES);
        let mut d = client_draw(t, &v);
        (d.vs, d.fs) = (vs, fs);
        let allocs = with(|m| m.allocs);
        assert_eq!(draw_flush(&d), 0);
        assert_eq!(with(|m| m.allocs), allocs, "no chunk grown");
        let words = with(|m| m.submits.last().unwrap().1.clone());
        assert_eq!(attribute_buffers(&words), [(data0, (4 * v.len()) as u32)]);
    });
    assert_eq!(mali_finish(), 0);
}
