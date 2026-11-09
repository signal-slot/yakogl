// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Collabora ltd.
//! Z4 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "Submit (Z4)" and
//! the milestones): one frame run on CSG 0 / CS 0 after Z3, in one run. The
//! frame is checked first ([`check_frame`]); then Z3 ([`fw_up`]: Z1's
//! power-on, Z2's AS0, the firmware's boot), the interface read again
//! (reads only), the stream's user pages and the group's suspend buffers
//! mapped in AS0 beside the firmware's sections, the group's VM built and
//! enabled in address space 1 ([`GROUP_AS`]) over the frame's range with the
//! ring and the sync object after it, the tiler heap made at the VAs the
//! frame names ([`heap`]), CSG 0 / CS 0 started ([`sched::start`]), the
//! frame's blocks and command stream written at their VAs, one job submitted
//! that calls the stream ([`sched::submit`]), its sync object waited for
//! with panthor's 5 s timeout ([`sched::wait`]), the group stopped and
//! terminated ([`sched::stop_and_terminate`]), and the target read back by
//! the CPU.
//!
//! Derived from Linux v6.18's drivers/gpu/drm/panthor (Copyright 2023
//! Collabora ltd.), used under the MIT option of its `GPL-2.0 or MIT`
//! licence (the design page's decision 1): the order of panthor_sched.c's
//! group and queue creation and panthor_mmu.c's map-then-flush. Lines below
//! are of v6.18 (`VERSION = 6`, `PATCHLEVEL = 18`, `SUBLEVEL = 0`). Where
//! this differs from panthor it says so.
//!
//! # Memory
//!
//! - AS0 (the MCU's VM): the 8 KiB of user input and output pages
//!   (`panthor_fw_alloc_queue_iface_mem`, panthor_fw.c:448-467), then the
//!   suspend buffers of the sizes CSG 0's control block gives
//!   (panthor_sched.c:3506-3520), each rounded up to 4 KiB, at the caller's
//!   [`McuBacking`]. They are added to the tables Z3 built (Z3's pool, its
//!   `tables_used` pages kept), the pages cleaned, and AS0 flushed over the
//!   range with `FLUSH_PT`, as panthor flushes after a map into a live VM
//!   (`panthor_vm_map_pages`, panthor_mmu.c). The user pages are uncached
//!   for the GPU (panthor_fw.c:451), the suspend buffers cached
//!   (panthor_fw.c:480-487).
//! - AS1 (the group's VM, panthor_mmu.c:759): the frame's range
//!   `[va_base, va_end)` mapped writable, executable and GPU-cached (a user
//!   buffer as panthor binds it without `MAP_NOEXEC` or `MAP_UNCACHED`),
//!   then [`QUEUE_VM_BYTES`] at `va_end`: the ring ([`RING_SIZE`], the size
//!   of the design page's panthor session) and a page whose first 16 bytes
//!   are the sync object, both no-exec and uncached (panthor_sched.c:
//!   3352-3358, 3522-3529). One [`GroupBacking`], mapped uncached for the
//!   CPU, backs all of it, in that order. The tables are built in the
//!   caller's second pool, cleaned, and the address space enabled as
//!   `panthor_mmu_as_enable` does (`AS_TRANSTAB`, `AS_MEMATTR`,
//!   `AS_TRANSCFG`, `UPDATE`), with Z2's `AS_MEMATTR` word
//!   ([`MEMATTR_LIVE`], panthor's for AS0 and AS1).
//! - The heap: the pool buffer at `heap_ctx_va` (heap 0's context is the
//!   pool's first slot, panthor_heap.c:331-338) and one chunk of
//!   `heap_chunk_size` at `heap_chunk_va`, both inside the frame's range.
//!   One initial chunk and at most one, not Mesa's 5 and 64: the frame names
//!   one first chunk, and Z4 grows no heap (`CS_TILER_OOM` is refused).
//!
//! # The frame, and what is read back
//!
//! [`Frame`] is the shape of driver/rust/src/board/nanopc_t6_frame.rs's
//! consts (generated from tools/mali/panthor/src/l1.rs): the blocks at their
//! VAs, the command stream the job calls, the heap's and the target's VAs,
//! the range they all lie in, and the generator's digest, copied into the
//! record as it is. Before the blocks the target is filled with [`FILL`]
//! (l1.rs's `FILL`), so a frame that never reached the target reads as
//! fill, not as a clear; after the group's end the CPU reads the target
//! through the uncached mapping, `width * height` RGBA8 pixels, and counts
//! opaque red ([`RED`]), opaque blue ([`BLUE`]), the fill and the rest.
//!
//! # What it returns
//!
//! One [`SubmitUp`], `#[repr(C)]` of 32- and 64-bit words with no padding,
//! so the C side takes it as it is. The first step that refuses names
//! itself in `step` with its refusal and value; Z3's own record is embedded
//! whole (`fw`). After a refusal past the start the group is still stopped
//! and terminated (sched.rs: "the caller terminates it") and both results
//! are kept; a stop or terminate that fails after a completed job is the
//! step itself ([`STEP_STOP`], [`STEP_TERMINATE`]).
//!
//! Two choices of ours: the job's `latest_flush` is `GPU_LATEST_FLUSH_ID`
//! (panthor_regs.h, 0x38) read right before the submit, the value Mesa
//! reads from panthor's flush-id page; and the job's sequence number is
//! [`SEQNO`], 1, the first of a fresh queue (panthor_sched.c:3165-3169).

use super::nanopc_t6_fw::{self, fw_up, FwUp};
use super::nanopc_t6_gpu::{Bases, GpuUp};
use super::nanopc_t6_mmu::MEMATTR_LIVE;
use super::nanopc_t6_sections::{Backing, McuMem, Placement};
use crate::gpu::mali::csf::{self, Cs, CsfRefusal, Csg, Iface, McuMemory};
use crate::gpu::mali::fw;
use crate::gpu::mali::heap::{self, Allocation, HeapError, HeapMemory, HeapObject, HeapParams, HeapPool};
use crate::gpu::mali::mmu::{self, FlushOp};
use crate::gpu::mali::pgtable::{Prot, Table, Tables, PAGE, VA_BITS};
use crate::gpu::mali::sched::{
    self, Ended, Group, GroupVm, Job, Queue, RingbufInput, RingbufOutput, Started, SyncObj64, GROUP_AS,
    QUEUE_IFACE_SIZE, QUEUE_OUTPUT_OFFSET, QUEUE_PRIORITY, RING_MIN_SIZE,
};
use crate::gpu::mali::{self, Ident};
use crate::mmio::Mmio;
use core::mem::offset_of;

/// Which step stopped the run ([`SubmitUp::step`]), in the run's order.
pub const STEP_NONE: u32 = 0;
/// [`check_frame`] refused the frame or the group's memory (`refusal`: the
/// [`FrameRefusal`]). Nothing was touched.
pub const STEP_FRAME: u32 = 1;
/// Z3 stopped (`refusal`: [`FwUp::step`]; `fw` holds its record).
pub const STEP_FW: u32 = 2;
/// Reading the interface again after Z3 refused (`refusal`: the
/// [`CsfRefusal`]).
pub const STEP_IFACE: u32 = 3;
/// `AS_PRESENT` has no address space 1 (`refusal` 1, `detail` the word).
pub const STEP_IDENT: u32 = 4;
/// The [`McuBacking`] is too small for the user pages and suspend buffers
/// (`refusal` 1, `detail` the bytes needed), or not 4 KiB aligned
/// (`refusal` 2, `detail` its MCU address).
pub const STEP_QUEUE_MEM: u32 = 5;
/// Adding the queue memory to AS0's tables refused (`refusal`: the
/// [`crate::gpu::mali::pgtable::PtRefusal`], `detail` the MCU address).
pub const STEP_AS0_MAP: u32 = 6;
/// A clean of table pages failed (`refusal` 1 AS0's, 2 the group VM's;
/// `detail` the rc).
pub const STEP_CLEAN: u32 = 7;
/// AS0's `FLUSH_PT` after the map stopped (`refusal`: the
/// [`mali::MaliRefusal`]).
pub const STEP_AS0_FLUSH: u32 = 8;
/// The group VM's tables refused (`refusal`: the `PtRefusal`, `detail` the
/// VA).
pub const STEP_VM_MAP: u32 = 9;
/// Address space 1's enable stopped (`refusal`: the [`mali::MaliRefusal`]).
pub const STEP_VM_ENABLE: u32 = 10;
/// The tiler heap was refused (`refusal`, `detail`: [`heap_error_code`]).
pub const STEP_HEAP: u32 = 11;
/// [`sched::start`] refused (`refusal`: the [`sched::SchedRefusal`]).
pub const STEP_START: u32 = 12;
/// [`sched::submit`] refused (`refusal`: the `SchedRefusal`).
pub const STEP_SUBMIT: u32 = 13;
/// [`sched::wait`] refused (`refusal`: the `SchedRefusal`; `JobTimeout` 19
/// for the 5 s timeout).
pub const STEP_WAIT: u32 = 14;
/// The job completed, but CS 0's stop was refused (`refusal`: the
/// `SchedRefusal`).
pub const STEP_STOP: u32 = 15;
/// The job completed, but CSG 0's terminate was refused (`refusal`: the
/// `SchedRefusal`): the GPU needs the reset.
pub const STEP_TERMINATE: u32 = 16;

/// The ring: panthor's smallest, the size of the design page's panthor
/// session (sched.rs's module doc).
pub const RING_SIZE: u32 = RING_MIN_SIZE;
/// What the group VM holds after the frame's range: the ring, then one page
/// holding the 16-byte sync object.
pub const QUEUE_VM_BYTES: u64 = RING_SIZE as u64 + PAGE;
/// The bytes of heap 0's context the frame's blocks may not overlap: one
/// slot of the G610's 64-byte L2 line (heap.rs, `ctx_stride`).
pub const HEAP_CTX_BYTES: u64 = 64;
/// `GPU_LATEST_FLUSH_ID` (panthor_regs.h): the job's `latest_flush`.
pub const LATEST_FLUSH_ID: usize = 0x38;
/// The job's sequence number: the first of the queue.
pub const SEQNO: u64 = 1;
/// The target's pixels as the CPU reads them (RGBA8, little-endian words):
/// l1.rs's fill pattern, opaque red, opaque blue.
pub const FILL: u32 = u32::from_le_bytes([0x5a, 0xa5, 0x3c, 0xc3]);
pub const RED: u32 = u32::from_le_bytes([0xff, 0, 0, 0xff]);
pub const BLUE: u32 = u32::from_le_bytes([0, 0, 0xff, 0xff]);

/// The frame the run submits: the consts of the generated
/// driver/rust/src/board/nanopc_t6_frame.rs, every VA in the group's VM.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// The blocks (descriptors, shaders, data), each written at its VA.
    pub blocks: &'a [(u64, &'a [u8])],
    /// The command stream the job's `CALL` runs, 8-byte aligned.
    pub cs_va: u64,
    pub cs: &'a [u64],
    /// The heap context `HEAP_SET` names (the pool buffer's first slot) and
    /// the first chunk, of `heap_chunk_size` bytes.
    pub heap_ctx_va: u64,
    pub heap_chunk_va: u64,
    pub heap_chunk_size: u32,
    /// The RGBA8 target: `target_bytes` at `target_va`, `width` x `height`
    /// pixels of 4 bytes, rows `width * 4` apart.
    pub target_va: u64,
    pub target_bytes: u32,
    pub width: u32,
    pub height: u32,
    /// The range every VA above lies in: `[va_base, va_end)`, 4 KiB aligned.
    pub va_base: u64,
    pub va_end: u64,
    /// The generator's digest of the frame, copied into the record.
    pub digest: u64,
}

/// Why [`check_frame`] refused, with the value that decided it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FrameRefusal {
    /// `va_base`/`va_end` not 4 KiB aligned, empty, or the range with the
    /// queue's [`QUEUE_VM_BYTES`] after it leaving [`VA_BITS`] (the value:
    /// `va_base`).
    Range = 1,
    /// The [`GroupBacking`] is smaller than the range and the queue's bytes,
    /// or not 4 KiB aligned (the value: the bytes needed).
    Memory = 2,
    /// The command stream, a block, the target or the heap chunk has no
    /// bytes, or the stream more than `u32::MAX` (the value: its VA).
    Empty = 3,
    /// The command stream not 8-byte or the target not 4-byte aligned (the
    /// value: the VA).
    Unaligned = 4,
    /// A range outside `[va_base, va_end)` (the value: its VA).
    Outside = 5,
    /// Two ranges overlap (the value: the later one's VA, in the order
    /// stream, target, heap context, heap chunk, blocks).
    Overlap = 6,
    /// `target_bytes` below `width * height * 4` (the value: `target_bytes`).
    Target = 7,
}

/// The group VM's memory: `size` bytes at physical `pa`, which the CPU
/// reaches at `cpu`, mapped uncached. The frame's range at its start, the
/// ring and the sync object after it ([`QUEUE_VM_BYTES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupBacking {
    pa: u64,
    cpu: usize,
    size: u64,
}

impl GroupBacking {
    /// # Safety
    ///
    /// `size` bytes at `cpu` are memory this run may read and write through
    /// plain pointers (the tiler heap's creation writes its context and
    /// chunk header so), mapped uncached for the CPU, the same bytes as
    /// physical `pa`, and used by nothing else while the run lasts.
    pub const unsafe fn new(pa: u64, cpu: usize, size: u64) -> GroupBacking {
        GroupBacking { pa, cpu, size }
    }
    pub const fn pa(&self) -> u64 {
        self.pa
    }
    pub const fn cpu(&self) -> usize {
        self.cpu
    }
    pub const fn size(&self) -> u64 {
        self.size
    }
}

/// The queue's AS0 memory: `size` bytes at physical `pa` the CPU reaches at
/// `cpu` (uncached), mapped at MCU address `va`, beside the firmware's
/// sections: the user input page, the output page, then the suspend
/// buffers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct McuBacking {
    pub va: u32,
    pub pa: u64,
    pub cpu: usize,
    pub size: u64,
}

/// The memory Z4 adds to Z3's: the queue's in AS0, the group VM's, and the
/// group VM's table pages at physical `vm_tables_pa`.
pub struct Z4Mem<'a> {
    pub queue: McuBacking,
    pub vm: GroupBacking,
    pub vm_pool: &'a mut [Table],
    pub vm_tables_pa: u64,
}

fn overlap(a: (u64, u64), b: (u64, u64)) -> bool {
    a.0 < b.0 + b.1 && b.0 < a.0 + a.1
}

/// The frame and the group VM's memory checked, before anything is touched:
/// the range, the memory, every range of the frame inside the range and
/// none overlapping another.
pub fn check_frame(f: &Frame, vm: &GroupBacking) -> Result<(), (FrameRefusal, u64)> {
    let top = 1u64 << VA_BITS;
    if f.va_base >= f.va_end
        || !f.va_base.is_multiple_of(PAGE)
        || !f.va_end.is_multiple_of(PAGE)
        || f.va_end.checked_add(QUEUE_VM_BYTES).is_none_or(|e| e > top)
    {
        return Err((FrameRefusal::Range, f.va_base));
    }
    let need = f.va_end - f.va_base + QUEUE_VM_BYTES;
    if vm.size < need || !vm.pa.is_multiple_of(PAGE) || !(vm.cpu as u64).is_multiple_of(PAGE) {
        return Err((FrameRefusal::Memory, need));
    }
    if f.cs.is_empty() || f.cs.len() > (u32::MAX / 8) as usize {
        return Err((FrameRefusal::Empty, f.cs_va));
    }
    if !f.cs_va.is_multiple_of(8) {
        return Err((FrameRefusal::Unaligned, f.cs_va));
    }
    if !f.target_va.is_multiple_of(4) {
        return Err((FrameRefusal::Unaligned, f.target_va));
    }
    if u64::from(f.target_bytes) < u64::from(f.width) * u64::from(f.height) * 4 {
        return Err((FrameRefusal::Target, u64::from(f.target_bytes)));
    }
    let range = |i: usize| -> (u64, u64) {
        match i {
            0 => (f.cs_va, 8 * f.cs.len() as u64),
            1 => (f.target_va, u64::from(f.target_bytes)),
            2 => (f.heap_ctx_va, HEAP_CTX_BYTES),
            3 => (f.heap_chunk_va, u64::from(f.heap_chunk_size)),
            _ => (f.blocks[i - 4].0, f.blocks[i - 4].1.len() as u64),
        }
    };
    for i in 0..f.blocks.len() + 4 {
        let (va, len) = range(i);
        if len == 0 {
            return Err((FrameRefusal::Empty, va));
        }
        if va < f.va_base || va.checked_add(len).is_none_or(|e| e > f.va_end) {
            return Err((FrameRefusal::Outside, va));
        }
        if (0..i).any(|j| overlap((va, len), range(j))) {
            return Err((FrameRefusal::Overlap, va));
        }
    }
    Ok(())
}

/// [`HeapError`] as a number, in its order, with the value it carries (the
/// first of its fields; the allocator's error for an allocation).
pub fn heap_error_code(e: &HeapError<u32>) -> (u32, u64) {
    match *e {
        HeapError::ZeroInitialChunks => (1, 0),
        HeapError::InitialAboveMax { initial_chunk_count, .. } => (2, u64::from(initial_chunk_count)),
        HeapError::ChunkSizeNotPageAligned { chunk_size } => (3, u64::from(chunk_size)),
        HeapError::ChunkSizeBelowMin { chunk_size } => (4, u64::from(chunk_size)),
        HeapError::ChunkSizeAboveMax { chunk_size } => (5, u64::from(chunk_size)),
        HeapError::ChunkListTooShort { len, .. } => (6, len as u64),
        HeapError::L2LineSize { l2_features } => (7, u64::from(l2_features)),
        HeapError::PoolAlloc(e) => (8, u64::from(e)),
        HeapError::ChunkAlloc { err, .. } => (9, u64::from(err)),
        HeapError::ShortAllocation { size, .. } => (10, size),
        HeapError::MisalignedAllocation { gpu } => (11, gpu),
        HeapError::NoCpuAddress { gpu } => (12, gpu),
        HeapError::PoolFull { heaps } => (13, u64::from(heaps)),
        HeapError::UnknownHeap { heap_gpu_va } => (14, heap_gpu_va),
        HeapError::InFlightAboveTarget { renderpasses_in_flight, .. } => (15, u64::from(renderpasses_in_flight)),
        HeapError::MaxChunksReached { chunk_count, .. } => (16, u64::from(chunk_count)),
        HeapError::UnknownChunk { chunk_gpu_va } => (17, chunk_gpu_va),
    }
}

/// The group VM as the CPU sees it: `[va_base, va_end + QUEUE_VM_BYTES)` at
/// the backing's CPU address.
struct VmMap {
    base: u64,
    end: u64,
    cpu: usize,
}

impl VmMap {
    fn at(&self, va: u64) -> usize {
        self.cpu + (va - self.base) as usize
    }
}

impl GroupVm for VmMap {
    fn cpu(&self, va: u64, len: u32) -> Option<usize> {
        let in_range = va >= self.base && va.checked_add(u64::from(len)).is_some_and(|e| e <= self.end);
        in_range.then(|| self.at(va))
    }
}

/// The heap's two buffers where the frame names them, inside the frame's
/// range: the pool at `heap_ctx_va`, the chunk at `heap_chunk_va`. The
/// error: 1 the pool, 2 the chunk not inside the range.
struct VmHeap<'v> {
    vm: &'v VmMap,
    frame_end: u64,
    pool_va: u64,
    chunk_va: u64,
}

// SAFETY: every allocation is inside the GroupBacking (VmMap covers only
// it), whose constructor's contract gives plain-pointer access to those
// bytes, the GPU's at the same VA in the group VM, uncached; the pool and
// the chunk do not overlap (check_frame, and the heap's own size checks).
unsafe impl HeapMemory for VmHeap<'_> {
    type Error = u32;
    fn alloc(&mut self, size: u64, what: HeapObject) -> Result<Allocation, u32> {
        let (va, err) = match what {
            HeapObject::Pool => (self.pool_va, 1),
            HeapObject::Chunk => (self.chunk_va, 2),
        };
        if va < self.vm.base || va.checked_add(size).is_none_or(|e| e > self.frame_end) {
            return Err(err);
        }
        Ok(Allocation { cpu: self.vm.at(va), gpu: va, size })
    }
    fn free(&mut self, _: Allocation) {}
}

/// The MCU's memory for [`sched::start`]: the queue's [`McuBacking`], then
/// Z3's sections.
struct QueueMcu<'a, M: McuMemory> {
    fw: &'a M,
    q: McuBacking,
}

impl<M: McuMemory> McuMemory for QueueMcu<'_, M> {
    fn cpu(&self, va: u32, len: u32) -> Option<usize> {
        let (va, q) = (u64::from(va), u64::from(self.q.va));
        if va >= q && va + u64::from(len) <= q + self.q.size {
            return Some(self.q.cpu + (va - q) as usize);
        }
        self.fw.cpu(va as u32, len)
    }
    fn clean(&self, cpu: usize, len: usize) {
        self.fw.clean(cpu, len)
    }
}

fn read64(m: &impl Mmio, a: usize) -> u64 {
    u64::from(m.read32(a)) | u64::from(m.read32(a + 4)) << 32
}

fn write_bytes(m: &impl Mmio, cpu: usize, bytes: &[u8]) {
    if !cpu.is_multiple_of(4) {
        for (i, &b) in bytes.iter().enumerate() {
            m.write8(cpu + i, b);
        }
        return;
    }
    let (words, rest) = bytes.as_chunks::<4>();
    for (i, w) in words.iter().enumerate() {
        m.write32(cpu + 4 * i, u32::from_le_bytes(*w));
    }
    for (i, &b) in rest.iter().enumerate() {
        m.write8(cpu + 4 * words.len() + i, b);
    }
}

/// What [`submit_up`] read. The C side's `struct yd_t6_submit_up`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubmitUp {
    /// `STEP_*` of the step that stopped, [`STEP_NONE`] when all ran; its
    /// refusal number and the value it is about.
    pub step: u32,
    pub refusal: u32,
    pub detail: u64,

    /// Z3's record, whole.
    pub fw: FwUp,

    /// `AS_PRESENT` and `L2_FEATURES` after Z3.
    pub as_present: u32,
    pub l2_features: u32,
    /// AS0's table pages after the queue memory's map (Z3's and the new),
    /// and the group VM's.
    pub as0_tables_used: u32,
    pub vm_tables_used: u32,
    /// CSG 0's `suspend_size` and `protm_suspend_size`.
    pub suspend_size: u32,
    pub protm_suspend_size: u32,

    /// The heap: its context (`HEAP_SET`'s operand), its first chunk, the
    /// chunk size and the pool's slot stride.
    pub heap_ctx: u64,
    pub heap_chunk: u64,
    pub heap_chunk_size: u32,
    pub heap_stride: u32,

    /// After the start: CSG 0's `req` and `ack`, CS 0's `req` and `ack`, the
    /// microseconds until each acknowledge.
    pub csg_req: u32,
    pub csg_ack: u32,
    pub cs_req: u32,
    pub cs_ack: u32,
    pub csg_ack_us: u32,
    pub cs_ack_us: u32,

    /// The job's `latest_flush`; the `CSG_SYNC_UPDATE` events the wait
    /// acknowledged.
    pub latest_flush: u32,
    pub sync_updates: u32,
    /// After the wait: the input page's `insert`, the output page's
    /// `extract`, the sync object's `seqno` and `status`, the wait's
    /// microseconds (the elapsed time from the doorbell).
    pub insert: u64,
    pub extract: u64,
    pub seqno: u64,
    pub sync_status: u32,
    pub wait_us: u32,
    /// On a refused wait: CS 0's `status_cmd_ptr`, and `fault_info` or
    /// `fatal_info`.
    pub status_cmd_ptr: u64,
    pub info: u64,

    /// The group's end: the stop's microseconds and refusal (0 for none),
    /// the terminate's, and the events cleared after it (CSG 0's, CS 0's
    /// `cs_irq` bit, CS 0's).
    pub stop_us: u32,
    pub stop_refusal: u32,
    pub terminate_us: u32,
    pub terminate_refusal: u32,
    pub cleared: [u32; 3],

    /// The target read back: opaque red, opaque blue, the fill, the rest;
    /// the pixels read (0 when the run stopped before the blocks).
    pub red: u32,
    pub blue: u32,
    pub fill: u32,
    pub other: u32,
    pub pixels: u32,

    /// [`Frame::digest`].
    pub digest: u64,
}

const _: () = assert!(size_of::<SubmitUp>() == 8 * 49);
const _: () = assert!(align_of::<SubmitUp>() == 8);
const _: () = assert!(offset_of!(SubmitUp, step) == 0);
const _: () = assert!(offset_of!(SubmitUp, refusal) == 4);
const _: () = assert!(offset_of!(SubmitUp, detail) == 8);
const _: () = assert!(offset_of!(SubmitUp, fw) == 16);
const _: () = assert!(offset_of!(SubmitUp, as_present) == 16 + size_of::<FwUp>());
const _: () = assert!(offset_of!(SubmitUp, as_present) == 208);
const _: () = assert!(offset_of!(SubmitUp, l2_features) == 212);
const _: () = assert!(offset_of!(SubmitUp, as0_tables_used) == 216);
const _: () = assert!(offset_of!(SubmitUp, vm_tables_used) == 220);
const _: () = assert!(offset_of!(SubmitUp, suspend_size) == 224);
const _: () = assert!(offset_of!(SubmitUp, protm_suspend_size) == 228);
const _: () = assert!(offset_of!(SubmitUp, heap_ctx) == 232);
const _: () = assert!(offset_of!(SubmitUp, heap_chunk) == 240);
const _: () = assert!(offset_of!(SubmitUp, heap_chunk_size) == 248);
const _: () = assert!(offset_of!(SubmitUp, heap_stride) == 252);
const _: () = assert!(offset_of!(SubmitUp, csg_req) == 256);
const _: () = assert!(offset_of!(SubmitUp, csg_ack) == 260);
const _: () = assert!(offset_of!(SubmitUp, cs_req) == 264);
const _: () = assert!(offset_of!(SubmitUp, cs_ack) == 268);
const _: () = assert!(offset_of!(SubmitUp, csg_ack_us) == 272);
const _: () = assert!(offset_of!(SubmitUp, cs_ack_us) == 276);
const _: () = assert!(offset_of!(SubmitUp, latest_flush) == 280);
const _: () = assert!(offset_of!(SubmitUp, sync_updates) == 284);
const _: () = assert!(offset_of!(SubmitUp, insert) == 288);
const _: () = assert!(offset_of!(SubmitUp, extract) == 296);
const _: () = assert!(offset_of!(SubmitUp, seqno) == 304);
const _: () = assert!(offset_of!(SubmitUp, sync_status) == 312);
const _: () = assert!(offset_of!(SubmitUp, wait_us) == 316);
const _: () = assert!(offset_of!(SubmitUp, status_cmd_ptr) == 320);
const _: () = assert!(offset_of!(SubmitUp, info) == 328);
const _: () = assert!(offset_of!(SubmitUp, stop_us) == 336);
const _: () = assert!(offset_of!(SubmitUp, stop_refusal) == 340);
const _: () = assert!(offset_of!(SubmitUp, terminate_us) == 344);
const _: () = assert!(offset_of!(SubmitUp, terminate_refusal) == 348);
const _: () = assert!(offset_of!(SubmitUp, cleared) == 352);
const _: () = assert!(offset_of!(SubmitUp, red) == 364);
const _: () = assert!(offset_of!(SubmitUp, blue) == 368);
const _: () = assert!(offset_of!(SubmitUp, fill) == 372);
const _: () = assert!(offset_of!(SubmitUp, other) == 376);
const _: () = assert!(offset_of!(SubmitUp, pixels) == 380);
const _: () = assert!(offset_of!(SubmitUp, digest) == 384);

impl SubmitUp {
    fn stopped(&mut self, step: u32, refusal: u32, detail: u64) {
        self.step = step;
        self.refusal = refusal;
        self.detail = detail;
    }
    /// The group's end, kept; a failure is the step only when nothing
    /// stopped before it.
    fn ended(&mut self, e: &Ended) {
        match e.stop {
            Ok(us) => self.stop_us = us,
            Err((why, v)) => {
                self.stop_refusal = why as u32;
                if self.step == STEP_NONE {
                    self.stopped(STEP_STOP, why as u32, u64::from(v));
                }
            }
        }
        match e.terminate {
            Ok(t) => {
                self.terminate_us = t.us;
                self.cleared = [t.cleared.csg_events, t.cleared.cs_irq, t.cleared.cs_events];
            }
            Err((why, v)) => {
                self.terminate_refusal = why as u32;
                if self.step == STEP_NONE {
                    self.stopped(STEP_TERMINATE, why as u32, u64::from(v));
                }
            }
        }
    }
    /// The target's pixels, counted.
    fn read_back(&mut self, m: &impl Mmio, cpu: usize, f: &Frame) {
        let n = f.width as usize * f.height as usize;
        for i in 0..n {
            match m.read32(cpu + 4 * i) {
                RED => self.red += 1,
                BLUE => self.blue += 1,
                FILL => self.fill += 1,
                _ => self.other += 1,
            }
        }
        self.pixels = n as u32;
    }
}

/// Group 0 and its stream 0, found again as [`csf::boot`] found them: reads
/// only, after Z3.
fn rediscover(m: &impl Mmio, mem: &impl McuMemory, img: &fw::Image) -> Result<(Iface, Csg, Cs), (CsfRefusal, u32)> {
    let iface = csf::discover(m, mem, img)?;
    let csg = iface.csg(m, 0)?;
    let cs = iface.cs(m, &csg, 0)?;
    Ok((iface, csg, cs))
}

const fn page_up(n: u32) -> u64 {
    (n as u64).next_multiple_of(PAGE)
}

/// The run (see the module). `m`, `b`, `blob`, `backings`, `pool`,
/// `tables_pa`, `timer_hz`, `clean`, `delay_us` and `on_handover` are
/// [`fw_up`]'s, and Z4 goes on with them: `pool` must keep Z3's tables (its
/// pages after Z3's `tables_used` hold the queue memory's tables; they were
/// zeroed by Z3's build), `clean` cleans the table pages Z4 writes. `mem`
/// is the memory Z4 adds ([`Z4Mem`]); `frame` the frame.
#[allow(clippy::too_many_arguments)]
pub fn submit_up(
    m: &impl Mmio,
    b: Bases,
    blob: &[u8],
    backings: &[Backing],
    pool: &mut [Table],
    tables_pa: u64,
    timer_hz: u32,
    mem: Z4Mem<'_>,
    frame: &Frame<'_>,
    clean: &impl Fn(usize, usize) -> i32,
    delay_us: &mut impl FnMut(u32),
    on_handover: &mut impl FnMut(&GpuUp),
) -> SubmitUp {
    let mut o = SubmitUp { digest: frame.digest, ..SubmitUp::default() };
    if let Err((why, v)) = check_frame(frame, &mem.vm) {
        o.stopped(STEP_FRAME, why as u32, v);
        return o;
    }

    // Z3
    o.fw = fw_up(m, b, blob, backings, pool, tables_pa, timer_hz, clean, delay_us, on_handover);
    if o.fw.step != nanopc_t6_fw::STEP_NONE {
        o.stopped(STEP_FW, o.fw.step, 0);
        return o;
    }
    // Z3 has parsed and placed both already: neither refuses here
    let Ok(img) = fw::parse(blob) else {
        o.stopped(STEP_IFACE, 0, 0);
        return o;
    };
    let Ok(placement) = Placement::new(img.sections(), backings) else {
        o.stopped(STEP_IFACE, 0, 0);
        return o;
    };
    let fw_mem = McuMem::new(&placement, clean);
    let (iface, csg, cs) = match rediscover(m, &fw_mem, &img) {
        Ok(v) => v,
        Err((why, v)) => {
            o.stopped(STEP_IFACE, why as u32, u64::from(v));
            return o;
        }
    };
    let id: Ident = mali::ident(m, b.gpu);
    o.as_present = id.as_present;
    o.l2_features = id.l2_features;
    if id.as_present & (1 << GROUP_AS) == 0 {
        o.stopped(STEP_IDENT, 1, u64::from(id.as_present));
        return o;
    }

    // AS0: the user pages, then the suspend buffers
    o.suspend_size = csg.suspend_size;
    o.protm_suspend_size = csg.protm_suspend_size;
    let q = mem.queue;
    let (susp, protm) = (page_up(csg.suspend_size), page_up(csg.protm_suspend_size));
    let need = u64::from(QUEUE_IFACE_SIZE) + susp + protm;
    if !u64::from(q.va).is_multiple_of(PAGE) || !q.pa.is_multiple_of(PAGE) || !(q.cpu as u64).is_multiple_of(PAGE) {
        o.stopped(STEP_QUEUE_MEM, 2, u64::from(q.va));
        return o;
    }
    if q.size < need || u64::from(q.va) + need > 1 << 32 {
        o.stopped(STEP_QUEUE_MEM, 1, need);
        return o;
    }
    let (input_va, output_va) = (q.va, q.va + QUEUE_OUTPUT_OFFSET);
    let suspend_va = q.va + QUEUE_IFACE_SIZE;
    let protm_va = suspend_va + susp as u32;
    {
        let used = o.fw.tables_used as usize;
        if used == 0 || used > pool.len() {
            o.stopped(STEP_AS0_MAP, crate::gpu::mali::pgtable::PtRefusal::BadPool as u32, u64::from(q.va));
            return o;
        }
        // Z3's tables, reopened: its pages in use kept, the rest still zero
        let mut t = Tables { pages: &mut *pool, pa: tables_pa, used };
        let user = Prot { write: true, exec: false, cached: false };
        let suspend = Prot { write: true, exec: false, cached: true };
        if let Err(why) = t.map(u64::from(q.va), q.pa, u64::from(QUEUE_IFACE_SIZE), user) {
            o.stopped(STEP_AS0_MAP, why as u32, u64::from(q.va));
            return o;
        }
        if susp + protm != 0 {
            let pa = q.pa + u64::from(QUEUE_IFACE_SIZE);
            if let Err(why) = t.map(u64::from(suspend_va), pa, susp + protm, suspend) {
                o.stopped(STEP_AS0_MAP, why as u32, u64::from(suspend_va));
                return o;
            }
        }
        o.as0_tables_used = t.used() as u32;
        let pages = t.used_pages();
        let rc = clean(pages.as_ptr() as usize, size_of_val(pages));
        if rc != 0 {
            o.stopped(STEP_CLEAN, 1, rc as u64);
            return o;
        }
    }
    // panthor_vm_map_pages: the flush after a map into a live VM
    if let Err((why, v)) = mmu::hw_do_operation(m, b.gpu, 0, u64::from(q.va), need, FlushOp::Pt, delay_us) {
        o.stopped(STEP_AS0_FLUSH, why as u32, u64::from(v));
        return o;
    }

    // AS1: the frame's range, then the ring and the sync object
    let vm = mem.vm;
    let len = frame.va_end - frame.va_base;
    let root = {
        let mut t = match Tables::new(mem.vm_pool, mem.vm_tables_pa) {
            Ok(t) => t,
            Err(why) => {
                o.stopped(STEP_VM_MAP, why as u32, mem.vm_tables_pa);
                return o;
            }
        };
        let user = Prot { write: true, exec: true, cached: true };
        let queue = Prot { write: true, exec: false, cached: false };
        for (va, pa, size, prot) in
            [(frame.va_base, vm.pa, len, user), (frame.va_end, vm.pa + len, QUEUE_VM_BYTES, queue)]
        {
            if let Err(why) = t.map(va, pa, size, prot) {
                o.vm_tables_used = t.used() as u32;
                o.stopped(STEP_VM_MAP, why as u32, va);
                return o;
            }
        }
        o.vm_tables_used = t.used() as u32;
        let pages = t.used_pages();
        let rc = clean(pages.as_ptr() as usize, size_of_val(pages));
        if rc != 0 {
            o.stopped(STEP_CLEAN, 2, rc as u64);
            return o;
        }
        t.root_pa()
    };
    let transcfg = mmu::transcfg(VA_BITS, false);
    if let Err((why, v)) = mmu::as_enable(m, b.gpu, GROUP_AS, root, transcfg, MEMATTR_LIVE, delay_us) {
        o.stopped(STEP_VM_ENABLE, why as u32, u64::from(v));
        return o;
    }
    let vmap = VmMap { base: frame.va_base, end: frame.va_end + QUEUE_VM_BYTES, cpu: vm.cpu };

    // the tiler heap
    let mut hm =
        VmHeap { vm: &vmap, frame_end: frame.va_end, pool_va: frame.heap_ctx_va, chunk_va: frame.heap_chunk_va };
    let mut list = [Allocation::default(); 1];
    let params = HeapParams {
        initial_chunk_count: 1,
        chunk_size: frame.heap_chunk_size,
        max_chunks: 1,
        target_in_flight: HeapParams::MESA.target_in_flight,
    };
    let made = HeapPool::create(&mut hm, id.l2_features)
        .and_then(|mut hp| heap::create(&mut hp, &mut hm, &params, &mut list).map(|h| (hp.stride(), h.osal())));
    match made {
        Ok((stride, h)) => {
            o.heap_ctx = h.ctx_gpu;
            o.heap_chunk = h.first_chunk_gpu;
            o.heap_chunk_size = h.chunk_size;
            o.heap_stride = stride;
        }
        Err(e) => {
            let (code, v) = heap_error_code(&e);
            o.stopped(STEP_HEAP, code, v);
            return o;
        }
    }

    // CSG 0 / CS 0
    let group = Group::all_cores(
        &id,
        (csg.suspend_size != 0).then_some(suspend_va),
        (csg.protm_suspend_size != 0).then_some(protm_va),
    );
    let queue = Queue {
        ring_va: frame.va_end,
        ring_size: RING_SIZE,
        input_va,
        output_va,
        syncobj_va: frame.va_end + u64::from(RING_SIZE),
        priority: QUEUE_PRIORITY,
    };
    let qmem = QueueMcu { fw: &fw_mem, q };
    let end = |o: &mut SubmitUp, delay_us: &mut _| {
        o.ended(&sched::stop_and_terminate(m, b.gpu, &iface, &csg, &cs, delay_us));
        o.read_back(m, vmap.at(frame.target_va), frame);
    };
    let s: Started = match sched::start(m, b.gpu, &qmem, &vmap, &id, &iface, &csg, &cs, &group, &queue, delay_us) {
        Ok(s) => s,
        Err((why, v)) => {
            o.stopped(STEP_START, why as u32, u64::from(v));
            o.ended(&sched::stop_and_terminate(m, b.gpu, &iface, &csg, &cs, delay_us));
            return o;
        }
    };
    o.csg_req = s.csg_req;
    o.csg_ack = s.csg_ack;
    o.cs_req = s.cs_req;
    o.cs_ack = s.cs_ack;
    o.csg_ack_us = s.csg_ack_us;
    o.cs_ack_us = s.cs_ack_us;

    // the frame at its VAs: the target filled, the blocks, the stream
    let target = vmap.at(frame.target_va);
    for i in 0..frame.target_bytes as usize / 4 {
        m.write32(target + 4 * i, FILL);
    }
    for &(va, bytes) in frame.blocks {
        write_bytes(m, vmap.at(va), bytes);
    }
    let cs_cpu = vmap.at(frame.cs_va);
    for (i, &w) in frame.cs.iter().enumerate() {
        m.write32(cs_cpu + 8 * i, w as u32);
        m.write32(cs_cpu + 8 * i + 4, (w >> 32) as u32);
    }

    // one job, its completion
    o.latest_flush = m.read32(b.gpu + LATEST_FLUSH_ID);
    let job = Job { cs_start: frame.cs_va, cs_size: 8 * frame.cs.len() as u32, latest_flush: o.latest_flush };
    let sub = match sched::submit(m, b.gpu, &cs, &queue, &s, &job, SEQNO) {
        Ok(sub) => sub,
        Err((why, v)) => {
            o.stopped(STEP_SUBMIT, why as u32, u64::from(v));
            end(&mut o, delay_us);
            return o;
        }
    };
    let ended = match sched::wait(m, b.gpu, &iface, &csg, &cs, &s, &sub, delay_us) {
        Ok(w) => {
            o.seqno = w.seqno;
            o.wait_us = w.us;
            o.sync_updates = w.sync_updates;
            None
        }
        Err(r) => {
            o.stopped(STEP_WAIT, r.refused.0 as u32, u64::from(r.refused.1));
            o.seqno = r.seqno;
            o.wait_us = r.us;
            o.status_cmd_ptr = r.status_cmd_ptr;
            o.info = r.info;
            r.ended
        }
    };
    o.insert = read64(m, s.input_cpu + offset_of!(RingbufInput, insert));
    o.extract = read64(m, s.output_cpu + offset_of!(RingbufOutput, extract));
    o.sync_status = m.read32(s.syncobj_cpu + offset_of!(SyncObj64, status));
    match ended {
        // the timeout stopped and terminated the group itself
        Some(e) => {
            o.ended(&e);
            o.read_back(m, target, frame);
        }
        None => end(&mut o, delay_us),
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::nanopc_t6_fw::tests::{backings, B, T6, TABLES_PA};
    use crate::board::nanopc_t6_gpu;
    use crate::gpu::mali::csf::tests::{GPU, MEM};
    use crate::gpu::mali::csf::{
        CsInput, CsOutput, CsgInput, CsgOutput, GlbInput, GlbOutput, CSG_REQ_MASK, CSG_STATE_MASK, CSG_STATE_TERMINATE,
        CSG_SYNC_UPDATE, CS_REQ_MASK, CS_STATE_MASK, CS_STATE_STOP, RK3588_TIMER_HZ,
    };
    use crate::gpu::mali::fw::synth;
    use crate::gpu::mali::mmu::{mmu_as, AS_COMMAND, AS_COMMAND_UPDATE, AS_TRANSTAB};
    use crate::gpu::mali::pgtable::{PtRefusal, ENTRIES};
    use crate::gpu::mali::sched::job::{self, mov48, sync_add64};
    use crate::gpu::mali::sched::SchedRefusal;
    use std::alloc::{alloc_zeroed, dealloc, Layout};
    use std::cell::{Cell, RefCell};

    /// The interface blocks where csf.rs's model puts group 0 and stream 0.
    const GLB_IN: usize = MEM + 0x0400_2000;
    const GLB_OUT: usize = MEM + 0x0400_6000;
    const CSG_IN: usize = MEM + 0x0400_3000;
    const CSG_OUT: usize = MEM + 0x0400_7000;
    const CS_IN: usize = MEM + 0x0400_3400;
    const CS_OUT: usize = MEM + 0x0400_7200;

    /// The synthetic frame's group VM: 2 MiB from the kernel range's start.
    const VA: u64 = 0x8000_0000_0000;
    const VA_END: u64 = VA + 0x20_0000;
    const DESC_VA: u64 = VA + 0x1000;
    const DATA_VA: u64 = VA + 0x2000;
    /// The 8 bytes the frame's stream adds 1 to.
    const USER_SYNC_VA: u64 = VA + 0x3000;
    const CS_VA: u64 = VA + 0x4000;
    const TARGET_VA: u64 = VA + 0x8000;
    const HEAP_CTX_VA: u64 = VA + 0x1_5000;
    const CHUNK_VA: u64 = VA + 0x10_0000;
    const CHUNK_SIZE: u32 = 0x2_0000;
    const W: u32 = 64;
    const H: u32 = 64;
    const SYNC_VA: u64 = VA_END + RING_SIZE as u64;

    const VM_TABLES_PA: u64 = 0x1048_0000;
    const VM_PA: u64 = 0x1100_0000;
    const VM_BYTES: u64 = VA_END - VA + QUEUE_VM_BYTES;
    const QUEUE_MCU_VA: u32 = 0x0500_0000;
    const QUEUE: McuBacking = McuBacking { va: QUEUE_MCU_VA, pa: 0x1080_0000, cpu: MEM + 0x0500_0000, size: 0x2000 };

    const DESC: [u8; 64] = {
        let mut b = [0u8; 64];
        let mut i = 0;
        while i < 64 {
            b[i] = 0xc0 | i as u8;
            i += 1;
        }
        b
    };
    /// Not a multiple of 4: the tail goes byte by byte.
    const DATA: [u8; 13] = *b"thirteen byte";
    const USER_SYNC: [u8; 8] = [0; 8];
    /// A stream that only adds 1 to the 8 bytes at USER_SYNC_VA.
    const CS: [u64; 3] = [mov48(0, USER_SYNC_VA), mov48(2, 1), sync_add64(0, 2)];
    const BLOCKS: [(u64, &[u8]); 3] = [(DESC_VA, &DESC), (DATA_VA, &DATA), (USER_SYNC_VA, &USER_SYNC)];

    fn frame() -> Frame<'static> {
        Frame {
            blocks: &BLOCKS,
            cs_va: CS_VA,
            cs: &CS,
            heap_ctx_va: HEAP_CTX_VA,
            heap_chunk_va: CHUNK_VA,
            heap_chunk_size: CHUNK_SIZE,
            target_va: TARGET_VA,
            target_bytes: W * H * 4,
            width: W,
            height: H,
            va_base: VA,
            va_end: VA_END,
            digest: 0x0123_4567_89ab_cdef,
        }
    }

    /// Host memory standing for the group VM's backing, page aligned.
    struct Ram {
        base: usize,
        layout: Layout,
    }

    impl Ram {
        fn new(size: u64) -> Ram {
            let layout = Layout::from_size_align(size as usize, PAGE as usize).unwrap();
            // SAFETY: a non-zero size
            let p = unsafe { alloc_zeroed(layout) };
            assert!(!p.is_null());
            Ram { base: p as usize, layout }
        }
        fn has(&self, a: usize) -> bool {
            (self.base..self.base + self.layout.size()).contains(&a)
        }
    }

    impl Drop for Ram {
        fn drop(&mut self) {
            // SAFETY: allocated with this layout in new
            unsafe { dealloc(self.base as *mut u8, self.layout) }
        }
    }

    /// Z3's T6 (nanopc_t6_fw's model: the SoC, the GPU's MMU, the MCU
    /// booting from loaded code) with Z4's firmware behaviour over it, as
    /// sched.rs's model has it: on the global doorbell, `lag` reads later,
    /// the GLB `doorbell_req` taken, CSG 0's request fields
    /// (`CSG_REQ_MASK`) and `doorbell_req` acknowledged, and CS 0's request
    /// fields (`CS_REQ_MASK`) when the doorbell names it. On doorbell 1,
    /// `job_lag` reads later, the blocks between the output page's `extract`
    /// and the input page's `insert` run from the ring: `MOV32`/`MOV48` load
    /// registers, `CALL` runs the stream it names the same way, `SYNC_ADD64`
    /// adds to the 64-bit word it names (and writes `sync_status` when that
    /// word is the queue's sync object); then `extract` advances and
    /// `CSG_SYNC_UPDATE` is raised. After a `CALL` it paints the target as
    /// the frame would (red left half, blue right) when `paint` says so. The
    /// group VM is real memory (`ram`, at VA `VA`): the heap writes it
    /// through pointers. Knobs: `never_complete` ignores doorbell 1,
    /// `gpu_id` replaces `GPU_ID`. The identity registers Z3's model leaves
    /// at 0 read as the board's: `AS_PRESENT` 0xff, `TILER_PRESENT` 1,
    /// `L2_FEATURES` 0x07120306.
    struct Z4 {
        t6: T6,
        ram: Ram,
        log: RefCell<Vec<(usize, u32)>>,
        lag: u32,
        job_lag: u32,
        csg_due: Cell<Option<u32>>,
        job_due: Cell<Option<u32>>,
        never_complete: bool,
        sync_status: u32,
        paint: bool,
        gpu_id: Option<u32>,
        as_present: u32,
        calls: Cell<u32>,
    }

    impl Z4 {
        fn new(blob: Vec<u8>) -> Z4 {
            Z4 {
                t6: T6::handed_over(blob),
                ram: Ram::new(VM_BYTES),
                log: RefCell::new(vec![]),
                lag: 3,
                job_lag: 20,
                csg_due: Cell::new(None),
                job_due: Cell::new(None),
                never_complete: false,
                sync_status: 0,
                paint: true,
                gpu_id: None,
                as_present: 0xff,
                calls: Cell::new(0),
            }
        }
        fn cpu(&self, va: u64) -> usize {
            assert!((VA..VA + VM_BYTES).contains(&va), "{va:#x} outside the group VM");
            self.ram.base + (va - VA) as usize
        }
        fn w(&self, a: usize) -> u32 {
            if self.ram.has(a) {
                // SAFETY: inside the allocation, 4-byte aligned (every access is)
                return unsafe { (a as *const u32).read_volatile() };
            }
            self.t6.read32(a)
        }
        fn put(&self, a: usize, v: u32) {
            if self.ram.has(a) {
                // SAFETY: as in w
                return unsafe { (a as *mut u32).write_volatile(v) };
            }
            self.t6.write32(a, v)
        }
        fn w64(&self, a: usize) -> u64 {
            u64::from(self.w(a)) | u64::from(self.w(a + 4)) << 32
        }
        fn put64(&self, a: usize, v: u64) {
            self.put(a, v as u32);
            self.put(a + 4, (v >> 32) as u32);
        }
        fn ack_field(&self, req: usize, ack: usize, mask: u32) {
            self.put(ack, (self.w(ack) & !mask) | (self.w(req) & mask));
        }
        fn answer_csg(&self) {
            let (db_req, db_ack) =
                (GLB_IN + offset_of!(GlbInput, doorbell_req), GLB_OUT + offset_of!(GlbOutput, doorbell_ack));
            let rung = self.w(db_req) ^ self.w(db_ack);
            self.ack_field(db_req, db_ack, rung);
            if rung & 1 == 0 {
                return;
            }
            self.ack_field(CSG_IN + offset_of!(CsgInput, req), CSG_OUT + offset_of!(CsgOutput, ack), CSG_REQ_MASK);
            let (sreq, sack) =
                (CSG_IN + offset_of!(CsgInput, doorbell_req), CSG_OUT + offset_of!(CsgOutput, doorbell_ack));
            let streams = self.w(sreq) ^ self.w(sack);
            self.ack_field(sreq, sack, streams);
            if streams & 1 != 0 {
                self.ack_field(CS_IN + offset_of!(CsInput, req), CS_OUT + offset_of!(CsOutput, ack), CS_REQ_MASK);
            }
        }
        fn exec(&self, w: u64, regs: &mut [u64; 256]) {
            let r = |s: u32| (w >> s & 0xff) as usize;
            match w >> 56 {
                1 => regs[r(48)] = w & 0xffff_ffff_ffff,
                2 => regs[r(48)] = w & 0xffff_ffff,
                32 => {
                    let (at, size) = (regs[r(40)], regs[r(32)]);
                    for k in 0..size / 8 {
                        self.exec(self.w64(self.cpu(at + 8 * k)), regs);
                    }
                    self.calls.set(self.calls.get() + 1);
                    if self.paint {
                        self.paint_target();
                    }
                }
                51 => {
                    let at = regs[r(40)];
                    let c = self.cpu(at);
                    self.put64(c, self.w64(c) + regs[r(32)]);
                    if at == SYNC_VA && self.sync_status != 0 {
                        self.put(c + offset_of!(SyncObj64, status), self.sync_status);
                    }
                }
                _ => {}
            }
        }
        fn paint_target(&self) {
            for y in 0..H as u64 {
                for x in 0..W as u64 {
                    let px = if x < W as u64 / 2 { RED } else { BLUE };
                    self.put(self.cpu(TARGET_VA + 4 * (y * W as u64 + x)), px);
                }
            }
        }
        fn answer_job(&self) {
            let ring = self.w64(CS_IN + offset_of!(CsInput, ringbuf_base));
            let size = u64::from(self.w(CS_IN + offset_of!(CsInput, ringbuf_size)));
            let inp = MEM + self.w64(CS_IN + offset_of!(CsInput, ringbuf_input)) as usize;
            let out = MEM + self.w64(CS_IN + offset_of!(CsInput, ringbuf_output)) as usize;
            let insert = self.w64(inp + offset_of!(RingbufInput, insert));
            let mut extract = self.w64(out + offset_of!(RingbufOutput, extract));
            let mut regs = [0u64; 256];
            while extract < insert {
                for i in 0..job::JOB_WORDS as u64 {
                    self.exec(self.w64(self.cpu(ring + ((extract + 8 * i) & (size - 1)))), &mut regs);
                }
                extract += job::JOB_BYTES;
            }
            self.put64(out + offset_of!(RingbufOutput, extract), extract);
            let ack = CSG_OUT + offset_of!(CsgOutput, ack);
            self.put(ack, self.w(ack) ^ CSG_SYNC_UPDATE);
        }
        fn tick(&self, due: &Cell<Option<u32>>, answer: impl FnOnce()) {
            match due.get() {
                Some(0) => {
                    due.set(None);
                    answer();
                }
                Some(n) => due.set(Some(n - 1)),
                None => {}
            }
        }
        /// The index in the log of the first write of `v` to `a`.
        fn first(&self, a: usize, v: u32) -> Option<usize> {
            self.log.borrow().iter().position(|&w| w == (a, v))
        }
    }

    impl Mmio for Z4 {
        fn read32(&self, a: usize) -> u32 {
            self.tick(&self.csg_due, || self.answer_csg());
            self.tick(&self.job_due, || self.answer_job());
            match a {
                _ if a == GPU + mali::GPU_ID && self.gpu_id.is_some() => self.gpu_id.unwrap(),
                _ if a == GPU + mali::GPU_AS_PRESENT => self.as_present,
                _ if a == GPU + mali::GPU_TILER_PRESENT => 1,
                _ if a == GPU + mali::GPU_L2_FEATURES => 0x0712_0306,
                _ => self.w(a),
            }
        }
        fn write32(&self, a: usize, v: u32) {
            if !self.ram.has(a) && !(MEM..MEM + 0x1000_0000).contains(&a) {
                self.log.borrow_mut().push((a, v));
            }
            if a == GPU + csf::doorbell(0) {
                self.t6.write32(a, v);
                self.csg_due.set(Some(self.lag));
            } else if a == GPU + csf::doorbell(1) {
                if !self.never_complete {
                    self.job_due.set(Some(self.job_lag));
                }
            } else {
                self.put(a, v);
            }
        }
        fn read8(&self, a: usize) -> u8 {
            assert!(self.ram.has(a));
            // SAFETY: inside the allocation
            unsafe { (a as *const u8).read_volatile() }
        }
        fn write8(&self, a: usize, v: u8) {
            if self.ram.has(a) {
                // SAFETY: inside the allocation
                return unsafe { (a as *mut u8).write_volatile(v) };
            }
            self.t6.write8(a, v)
        }
    }

    fn vm(z: &Z4) -> GroupBacking {
        // SAFETY: the model's RAM, VM_BYTES of it, used by this run alone
        unsafe { GroupBacking::new(VM_PA, z.ram.base, VM_BYTES) }
    }

    /// The run over the model with `blob` (the backings the synthetic
    /// file's) and the given frame, the microseconds slept, and Z3's pool
    /// afterwards.
    fn run_frame(z: &Z4, blob: &[u8], f: &Frame) -> (SubmitUp, u64, Vec<Table>) {
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut vm_pool = vec![[0u64; ENTRIES]; 8];
        let mem = Z4Mem { queue: QUEUE, vm: vm(z), vm_pool: &mut vm_pool, vm_tables_pa: VM_TABLES_PA };
        let mut slept = 0u64;
        let o = submit_up(
            z,
            B,
            blob,
            &backings(&synth::small()),
            &mut pool,
            TABLES_PA,
            RK3588_TIMER_HZ,
            mem,
            f,
            &|a, n| z.t6.clean(a, n),
            &mut |us| slept += u64::from(us),
            &mut |_| {},
        );
        (o, slept, pool)
    }

    fn run(z: &Z4) -> SubmitUp {
        run_frame(z, &synth::small(), &frame()).0
    }

    fn ram_u64(z: &Z4, va: u64) -> u64 {
        z.w64(z.cpu(va))
    }

    #[test]
    fn the_frame_runs_on_csg0_cs0_and_the_record_is_as_predicted() {
        let blob = synth::small();
        let z = Z4::new(blob.clone());
        let (o, _, mut pool) = run_frame(&z, &blob, &frame());
        assert_eq!((o.step, o.refusal, o.detail), (STEP_NONE, 0, 0));
        // Z3 as nanopc_t6_fw's own test has it
        assert_eq!((o.fw.step, o.fw.version, o.fw.ping_acked, o.fw.tables_used), (0, 0x0105_0000, 1, 7));
        assert_eq!((o.as_present, o.l2_features), (0xff, 0x0712_0306));
        // the queue's pages at 0x05000000 need one more level-3 table; the
        // group VM: L0, L1, L2 and one L3 for the frame's 2 MiB, one for the
        // ring and sync object after it
        assert_eq!((o.as0_tables_used, o.vm_tables_used), (8, 5));
        assert_eq!((o.suspend_size, o.protm_suspend_size), (0, 0));
        // the heap where the frame names it, a 64-byte slot (L2 line 2^6)
        assert_eq!(
            (o.heap_ctx, o.heap_chunk, o.heap_chunk_size, o.heap_stride),
            (HEAP_CTX_VA, CHUNK_VA, CHUNK_SIZE, 64)
        );
        // the start: START with ENDPOINT_CONFIG; CS 0's START request acked
        assert_eq!((o.csg_req, o.csg_ack, o.cs_req, o.cs_ack), (0x11, 0x11, 0x0511, 0x0511));
        assert_eq!((o.csg_ack_us, o.cs_ack_us), (1, 0));
        // one 128-byte job consumed, the sync object at 1, its event seen
        assert_eq!((o.insert, o.extract, o.seqno, o.sync_status), (128, 128, 1, 0));
        assert_eq!((o.latest_flush, o.sync_updates, o.wait_us), (0, 1, 20));
        assert_eq!((o.status_cmd_ptr, o.info), (0, 0));
        // the end: both acknowledged at once, nothing left pending
        assert_eq!((o.stop_refusal, o.terminate_refusal), (0, 0));
        assert_eq!(o.cleared, [0, 0, 0]);
        // the target as the model painted it
        assert_eq!((o.red, o.blue, o.fill, o.other, o.pixels), (2048, 2048, 0, 0, 4096));
        assert_eq!(o.digest, 0x0123_4567_89ab_cdef);

        // the stream ran (its SYNC_ADD64), once
        assert_eq!((ram_u64(&z, USER_SYNC_VA), z.calls.get()), (1, 1));
        // the blocks at their VAs, the stream's words, the heap's context
        // zeroed and its one chunk's header unlinked
        for (va, bytes) in BLOCKS {
            for (i, &b) in bytes.iter().enumerate().filter(|&(i, _)| va != USER_SYNC_VA || i >= 8) {
                assert_eq!(z.read8(z.cpu(va + i as u64)), b, "{va:#x}+{i}");
            }
        }
        for (i, &w) in CS.iter().enumerate() {
            assert_eq!(ram_u64(&z, CS_VA + 8 * i as u64), w);
        }
        assert_eq!(ram_u64(&z, CHUNK_VA), 0);
        // the ring holds the job's block: CALL of the frame's stream, 24 bytes
        let ring: Vec<u64> = (0..job::JOB_WORDS as u64).map(|i| ram_u64(&z, VA_END + 8 * i)).collect();
        assert!(ring.contains(&mov48(92, CS_VA)) && ring.contains(&job::mov32(94, 24)), "{ring:x?}");
        // CSG 0 terminated, CS 0 stopped
        assert_eq!(z.w(CSG_OUT + offset_of!(CsgOutput, ack)) & CSG_STATE_MASK, CSG_STATE_TERMINATE);
        assert_eq!(z.w(CS_OUT + offset_of!(CsOutput, ack)) & CS_STATE_MASK, CS_STATE_STOP);

        // AS0 maps the user pages uncached at the queue's memory, Z3's
        // sections still there
        let t = Tables { pages: &mut pool, pa: TABLES_PA, used: o.as0_tables_used as usize };
        assert_eq!(t.walk(u64::from(QUEUE_MCU_VA) + 0x1008).pa, Some(QUEUE.pa + 0x1008));
        assert_eq!(t.walk(0x0400_0000).pa, Some(o.fw.shared_pa[0] as u64));
        // the order: AS1's tables, its UPDATE, the group's doorbell, the
        // stream's doorbell
        let as1 = GPU + mmu_as(GROUP_AS);
        let transtab = z.first(as1 + AS_TRANSTAB, VM_TABLES_PA as u32).unwrap();
        let update = z.first(as1 + AS_COMMAND, AS_COMMAND_UPDATE).unwrap();
        let log = z.log.borrow().clone();
        let start = log.iter().rposition(|&(a, _)| a == GPU + csf::doorbell(0)).unwrap();
        let ring1 = z.first(GPU + csf::doorbell(1), 1).unwrap();
        assert!(transtab < update && update < ring1, "{log:x?}");
        assert!(log[update..ring1].iter().any(|&(a, _)| a == GPU + csf::doorbell(0)));
        assert!(ring1 < start, "the group's end rings the group after the job");
    }

    #[test]
    fn a_frame_the_gpu_never_draws_reads_back_as_the_fill() {
        let mut z = Z4::new(synth::small());
        z.paint = false;
        let o = run(&z);
        assert_eq!((o.step, o.seqno), (STEP_NONE, 1));
        assert_eq!((o.red, o.blue, o.fill, o.other, o.pixels), (0, 0, 4096, 0, 4096));
    }

    #[test]
    fn a_z3_stop_is_named_with_z3_s_step_and_nothing_of_z4_runs() {
        let blob = synth::small();
        let mut z = Z4::new(blob.clone());
        z.gpu_id = Some(0xa867_0001);
        let o = run(&z);
        assert_eq!((o.step, o.refusal, o.detail), (STEP_FW, nanopc_t6_fw::STEP_GPU, 0));
        assert_eq!((o.fw.stop, o.fw.detail), (nanopc_t6_gpu::Stop::NotG610 as u32, [0xa867_0001, 0]));
        let as1 = GPU + mmu_as(GROUP_AS);
        let doorbells = [GPU + csf::doorbell(0), GPU + csf::doorbell(1)];
        assert!(z.log.borrow().iter().all(|&(a, _)| !(as1..as1 + 0x40).contains(&a) && !doorbells.contains(&a)));
        assert_eq!((o.heap_ctx, o.pixels, o.digest), (0, 0, 0x0123_4567_89ab_cdef));

        // not a firmware file: Z3's parse, before any write
        let mut bad = blob.clone();
        bad[0] ^= 1;
        let z = Z4::new(bad.clone());
        let (o, slept, _) = run_frame(&z, &bad, &frame());
        assert_eq!((o.step, o.refusal, o.fw.step), (STEP_FW, nanopc_t6_fw::STEP_PARSE, nanopc_t6_fw::STEP_PARSE));
        assert_eq!((z.log.borrow().len(), slept), (0, 0));
    }

    /// A refusal of the frame: nothing written, nothing slept, Z3 not run.
    fn refused(f: &Frame) -> (u32, u64) {
        let blob = synth::small();
        let z = Z4::new(blob.clone());
        let (o, slept, _) = run_frame(&z, &blob, f);
        assert_eq!(o.step, STEP_FRAME);
        assert_eq!((z.log.borrow().len(), slept, o.fw), (0, 0, FwUp::default()));
        (o.refusal, o.detail)
    }

    #[test]
    fn a_frame_outside_its_range_or_with_overlapping_blocks_is_refused_before_anything() {
        let outside: [(u64, &[u8]); 1] = [(VA_END - 4, &DESC[..8])];
        assert_eq!(refused(&Frame { blocks: &outside, ..frame() }), (FrameRefusal::Outside as u32, VA_END - 4));
        let below: [(u64, &[u8]); 1] = [(VA - 0x1000, &DESC)];
        assert_eq!(refused(&Frame { blocks: &below, ..frame() }), (FrameRefusal::Outside as u32, VA - 0x1000));
        // the target past the range's end
        let f = Frame { target_va: VA_END - 0x1000, ..frame() };
        assert_eq!(refused(&f), (FrameRefusal::Outside as u32, VA_END - 0x1000));
        // two blocks over the same bytes; a block over the target, over the
        // heap's context, over the stream
        let twice: [(u64, &[u8]); 2] = [(DESC_VA, &DESC), (DESC_VA + 60, &DATA)];
        assert_eq!(refused(&Frame { blocks: &twice, ..frame() }), (FrameRefusal::Overlap as u32, DESC_VA + 60));
        for va in [TARGET_VA + 16, HEAP_CTX_VA + 32, CS_VA + 16, CHUNK_VA + 0x1_fff0] {
            let b: [(u64, &[u8]); 1] = [(va, &DATA)];
            assert_eq!(refused(&Frame { blocks: &b, ..frame() }), (FrameRefusal::Overlap as u32, va));
        }
        // a range not 4 KiB aligned, an unaligned stream, an empty one, a
        // target too small for its pixels
        assert_eq!(refused(&Frame { va_base: VA + 8, ..frame() }), (FrameRefusal::Range as u32, VA + 8));
        assert_eq!(refused(&Frame { cs_va: CS_VA + 4, ..frame() }), (FrameRefusal::Unaligned as u32, CS_VA + 4));
        assert_eq!(refused(&Frame { cs: &[], ..frame() }), (FrameRefusal::Empty as u32, CS_VA));
        assert_eq!(refused(&Frame { target_bytes: W * H * 4 - 4, ..frame() }), (FrameRefusal::Target as u32, 16380));
        // a range larger than the group VM's memory
        assert_eq!(
            refused(&Frame { va_end: VA_END + 0x1000, ..frame() }),
            (FrameRefusal::Memory as u32, VM_BYTES + 0x1000)
        );
    }

    #[test]
    fn a_job_that_never_completes_times_out_after_5_s_and_the_group_is_ended() {
        let mut z = Z4::new(synth::small());
        z.never_complete = true;
        let blob = synth::small();
        let (o, slept, _) = run_frame(&z, &blob, &frame());
        assert_eq!((o.step, o.refusal, o.detail), (STEP_WAIT, SchedRefusal::JobTimeout as u32, 0));
        assert_eq!((o.wait_us, o.seqno, o.sync_status), (sched::JOB_TIMEOUT_US, 0, 0));
        // the ring was written, nothing consumed it
        assert_eq!((o.insert, o.extract), (128, 0));
        // wait's own stop and terminate, both acknowledged
        assert_eq!((o.stop_refusal, o.terminate_refusal), (0, 0));
        assert!(slept >= u64::from(sched::JOB_TIMEOUT_US));
        assert_eq!(z.w(CSG_OUT + offset_of!(CsgOutput, ack)) & CSG_STATE_MASK, CSG_STATE_TERMINATE);
        // the stream never ran: the target is the fill
        assert_eq!((o.fill, o.pixels, z.calls.get()), (4096, 4096, 0));
    }

    #[test]
    fn an_error_on_the_sync_object_stops_the_wait_and_the_group_is_still_ended() {
        let mut z = Z4::new(synth::small());
        z.sync_status = 0x41;
        let o = run(&z);
        assert_eq!((o.step, o.refusal, o.detail), (STEP_WAIT, SchedRefusal::SyncObjError as u32, 0x41));
        assert_eq!((o.seqno, o.sync_status, o.insert, o.extract), (1, 0x41, 128, 128));
        assert_eq!((o.stop_refusal, o.terminate_refusal), (0, 0));
        // the SYNC_UPDATE event was acknowledged on the poll that refused:
        // nothing left for the terminate to clear
        assert_eq!(o.cleared, [0, 0, 0]);
        assert_eq!(z.w(CSG_OUT + offset_of!(CsgOutput, ack)) & CSG_STATE_MASK, CSG_STATE_TERMINATE);
        assert_eq!((o.red, o.blue), (2048, 2048));
    }

    #[test]
    fn a_heap_the_range_cannot_hold_and_a_missing_address_space_are_named() {
        // the 8 KiB pool from 4 KiB before the range's end
        let blob = synth::small();
        let z = Z4::new(blob.clone());
        let blocks: [(u64, &[u8]); 0] = [];
        let f = Frame { blocks: &blocks, heap_ctx_va: VA_END - 0x1000, ..frame() };
        let (o, _, _) = run_frame(&z, &blob, &f);
        assert_eq!((o.step, o.refusal, o.detail), (STEP_HEAP, 8, 1));
        assert_eq!(o.vm_tables_used, 5);
        // nothing of the group started
        assert!(z.first(GPU + csf::doorbell(1), 1).is_none());
        assert_eq!(z.w(CSG_IN + offset_of!(CsgInput, req)) & CSG_STATE_MASK, CSG_STATE_TERMINATE);

        let mut z = Z4::new(blob.clone());
        z.as_present = 1;
        let o = run(&z);
        assert_eq!((o.step, o.refusal, o.detail), (STEP_IDENT, 1, 1));
        assert_eq!(o.fw.step, nanopc_t6_fw::STEP_NONE);
    }

    #[test]
    fn queue_memory_too_small_for_the_user_pages_is_named() {
        let blob = synth::small();
        let z = Z4::new(blob.clone());
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut vm_pool = vec![[0u64; ENTRIES]; 8];
        let queue = McuBacking { size: 0x1000, ..QUEUE };
        let mem = Z4Mem { queue, vm: vm(&z), vm_pool: &mut vm_pool, vm_tables_pa: VM_TABLES_PA };
        let o = submit_up(
            &z,
            B,
            &blob,
            &backings(&blob),
            &mut pool,
            TABLES_PA,
            RK3588_TIMER_HZ,
            mem,
            &frame(),
            &|a, n| z.t6.clean(a, n),
            &mut |_| {},
            &mut |_| {},
        );
        assert_eq!((o.step, o.refusal, o.detail), (STEP_QUEUE_MEM, 1, 0x2000));
        // a queue page over a firmware section: AS0 refuses the map
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut vm_pool = vec![[0u64; ENTRIES]; 8];
        let z = Z4::new(blob.clone());
        let queue = McuBacking { va: 0x0400_0000, ..QUEUE };
        let mem = Z4Mem { queue, vm: vm(&z), vm_pool: &mut vm_pool, vm_tables_pa: VM_TABLES_PA };
        let o = submit_up(
            &z,
            B,
            &blob,
            &backings(&blob),
            &mut pool,
            TABLES_PA,
            RK3588_TIMER_HZ,
            mem,
            &frame(),
            &|a, n| z.t6.clean(a, n),
            &mut |_| {},
            &mut |_| {},
        );
        assert_eq!((o.step, o.refusal, o.detail), (STEP_AS0_MAP, PtRefusal::AlreadyMapped as u32, 0x0400_0000));
    }

    #[test]
    fn the_record_is_49_words_of_8_at_fixed_offsets_and_the_steps_are_numbered() {
        assert_eq!((size_of::<SubmitUp>(), align_of::<SubmitUp>()), (392, 8));
        assert_eq!(size_of::<FwUp>(), 192);
        assert_eq!(
            [
                STEP_NONE,
                STEP_FRAME,
                STEP_FW,
                STEP_IFACE,
                STEP_IDENT,
                STEP_QUEUE_MEM,
                STEP_AS0_MAP,
                STEP_CLEAN,
                STEP_AS0_FLUSH,
                STEP_VM_MAP,
                STEP_VM_ENABLE,
                STEP_HEAP,
                STEP_START,
                STEP_SUBMIT,
                STEP_WAIT,
                STEP_STOP,
                STEP_TERMINATE
            ],
            core::array::from_fn::<u32, 17, _>(|i| i as u32)
        );
        assert_eq!(heap_error_code(&HeapError::ZeroInitialChunks), (1, 0));
        assert_eq!(heap_error_code(&HeapError::UnknownChunk { chunk_gpu_va: 7 }), (17, 7));
        assert_eq!((FILL, RED, BLUE), (0xc33c_a55a, 0xff00_00ff, 0xffff_0000));
        assert_eq!((RING_SIZE, QUEUE_VM_BYTES, SEQNO), (0x1000, 0x2000, 1));
    }
}
