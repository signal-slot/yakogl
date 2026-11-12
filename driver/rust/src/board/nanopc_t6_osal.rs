// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Zephyr Mali OSAL's logic on the NanoPC-T6 (K1): what
//! src/osal/zephyr_mali.c's `osal_zephyr_ops` forwards to, through the C
//! entries `yd_t6_osal_*` (ffi::rk3588). The ops are src/osal/mali.rs's
//! contract: GPU memory at a GPU VA, the two Mali queries, a submit of an
//! `osal_mali_kick` and its fence, the waits.
//!
//! One group with one stream, made once by [`Osal::up`]: Z1 to Z3 and the
//! group's start are Z4's own ([`nanopc_t6_submit::group_up`]), over a
//! group VM of four regions:
//!
//! | region | VA | mapping |
//! | --- | --- | --- |
//! | the ring and the sync object | [`QUEUE_VA`] | [`Mapping::Queue`]: no-exec, uncached |
//! | the heap's pool (heap 0's context) | [`HEAP_POOL_VA`] | [`Mapping::Heap`] |
//! | the heap's chunks, `heap_initial_chunks` made at the start, the rest on `CS_TILER_OOM` | [`HEAP_CHUNK_VA`] | [`Mapping::Heap`] |
//! | the carve-out every `gpu_alloc` comes from | [`CARVE_VA`] | [`Mapping::User`]: executable, GPU-cached |
//!
//! The carve-out is mapped once, whole, so an allocation is a range of its
//! pages and nothing else: `gpu` is [`CARVE_VA`] plus the offset, `cpu` and
//! `phys` the backing's at the same offset. Pages are handed out first fit
//! ([`Osal::alloc`]); a free is held until the kick being recorded
//! completes, the fence `submitted + 1`, as src/osal/zephyr.c and the
//! panthor OSAL hold it ([`Osal::free`]). `OSAL_MEM_GPU_EXEC` needs nothing:
//! the carve-out is executable, as panthor maps a buffer without
//! `MAP_NOEXEC`. `map_phys` of a range inside the carve-out answers its own
//! pages ([`Osal::window`]); the CPU's view is uncached, so the cache
//! operations have nothing to do.
//!
//! A kick is one job on the ring ([`sched::submit`]) whose `CALL` runs the
//! kick's stream; its fence is the job's sequence number. A wait is the
//! sync object reaching it ([`Osal::wait`]); `completed` reads the sync
//! object without waiting.
//!
//! The tiler heap is the OSAL's: its pool in the pool region, its chunks
//! in the chunk region's 2 MiB slots, lowest free first ([`HeapMem`]). It
//! starts with `heap_initial_chunks` ([`OsalMem`]) and grows to every slot
//! of the region: a `CS_TILER_OOM` seen by a wait, a submit or `completed`
//! is handled as panthor's `group_process_tiler_oom` does
//! ([`sched::service_tiler_oom`] with [`heap::grow`]); with no slot left,
//! or more render passes in flight than the heap's target, it is answered
//! with no chunk (panthor's `-ENOMEM`: the firmware waits).
//!
//! # Recovery
//!
//! A fault, a fatal, a protected-mode or progress-timer event, an OOM the
//! heap refuses, a sync object with an error, a 64-bit read that does not
//! settle, or panthor's 5 s job timeout, seen by a wait, a submit or
//! `completed`, restarts the group ([`Osal::recover`]): every kick not yet
//! complete is lost (its fence completes, so the backend frees its slot
//! and the held frees go back, and a wait for it is `-EIO`), CS 0 is
//! stopped and CSG 0 terminated, the heap made again, CSG 0 / CS 0 started
//! again over the same memory with an empty ring ([`sched::restart`]), and
//! the queue-init stream (`HEAP_SET` of the heap context, Z4's
//! [`z4::QUEUE_INIT`], at [`INIT_VA`]) run and waited for, since a new
//! stream has no heap context. The next kick goes on with the next fence.
//! panthor goes on after a `CS_FAULT` (the stream reaches its
//! `ERROR_BARRIER`) and never starts a failed group again (user space makes
//! a new one, panthor_sched.c:1925-1931); here every error takes the one
//! path, and the module's one group is the one made again.
//!
//! Fences are the ring's sequence numbers plus a base: 0 until the first
//! restart, then the fence before the queue-init job's, so that job (ring
//! sequence number 1) is the last fence already complete and the next
//! kick is `submitted + 1`.
//!
//! [`Osal::shutdown`] waits for the last kick, then stops CS 0 and
//! terminates CSG 0, as panthor ends a destroyed group; later kicks are
//! `-EIO` and frees give their pages back at once. A full pending-free
//! queue waits for its oldest kick (src/osal/pending_frees.h's rule).
//!
//! What needs a GPU reset is not done (panthor resets the device): a group
//! state none of the four, a `CS_FATAL` of type `CS_UNRECOVERABLE`, a start
//! or terminate not acknowledged, a restart or a queue-init job refused.
//! The OSAL is then dead ([`Osal::dead`]): every later submit and wait is
//! `-EIO`. `map_phys` outside the carve-out is `-ENOTSUP`.

use super::nanopc_t6_gpu::{Bases, GpuUp};
use super::nanopc_t6_sections::Backing;
use super::nanopc_t6_submit::{
    self as z4, GroupBacking, GroupUp, HeapPlace, Mapping, McuBacking, Region, SubmitUp, Z4Mem, HEAP_CTX_VA,
    HEAP_POOL_SIZE, LATEST_FLUSH_ID, MAX_HEAP_CHUNKS, QUEUE_VM_BYTES, RING_SIZE,
};
use crate::errno::{EBUSY, EINVAL, EIO, ENOMEM, ENOSPC, ENOTSUP};
use crate::gpu::mali::heap::{
    self, Allocation, HeapError, HeapMemory, HeapObject, HeapParams, HeapParts, HeapPool, OsalMaliHeap,
};
use crate::gpu::mali::pgtable::{Table, PAGE};
use crate::gpu::mali::sched::job::JOB_BYTES;
use crate::gpu::mali::sched::{self, Ended, Job, SchedRefusal, TilerOom, JOB_TIMEOUT_US};
use crate::gpu::mali::{csf, fw};
use crate::mmio::Mmio;

/// The group VM's regions (the module's table). The queue's is Z4's; the
/// heap's pool is where Z4 puts it (`HEAP_SET`'s operand of the board's
/// panthor trace); the chunks start on a 2 MiB boundary after it; the
/// carve-out at 256 GiB, far from all three.
pub const QUEUE_VA: u64 = z4::QUEUE_VA;
pub const HEAP_POOL_VA: u64 = HEAP_CTX_VA;
pub const HEAP_CHUNK_VA: u64 = 0x8000_0020_0000;
pub const CARVE_VA: u64 = 0x40_0000_0000;
/// Mesa's chunk size ([`crate::gpu::mali::heap::HeapParams::MESA`]).
pub const HEAP_CHUNK_SIZE: u32 = 2 * 1024 * 1024;
/// The largest carve-out: 256 MiB, one bit a page in [`Osal`].
pub const MAX_CARVE_PAGES: usize = 64 * 1024;
/// Frees held at once; a free into a full queue is refused and its pages
/// kept (`-ENOSPC`), as src/osal/pending_frees.h does without its wait.
pub const PENDING_FREES: usize = 256;

/// Lost fence ranges a wait still answers `-EIO` for, one a restart; a
/// fence at or below an older one's end is `-EIO` too (the safe side: the
/// backend waits for each frame's fence once, two frames in flight).
pub const LOST_RANGES: usize = 16;
/// Where a restart's queue-init stream lies: the queue region's page after
/// the 16-byte sync object, from the next 64-byte boundary (the ring and
/// the sync object take [`RING_SIZE`] + 16 bytes of the region's
/// [`QUEUE_VM_BYTES`]). The ring itself is no-exec there, as panthor maps
/// it, and the command stream frontend runs it.
pub const INIT_VA: u64 = QUEUE_VA + RING_SIZE as u64 + 64;

const _: () = assert!(INIT_VA + 8 * z4::QUEUE_INIT.len() as u64 <= QUEUE_VA + QUEUE_VM_BYTES);
const _: () = assert!(HEAP_POOL_VA + HEAP_POOL_SIZE <= HEAP_CHUNK_VA);
const _: () = assert!(QUEUE_VA + QUEUE_VM_BYTES <= HEAP_POOL_VA);

/// `OsalMaliGpu` of src/osal/mali.rs (`struct osal_mali_gpu`), which this
/// crate cannot import: the same fields at the same offsets, 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OsalMaliGpu {
    pub gpu_id: u32,
    pub tiler_features: u32,
    pub cs_reg_count: u32,
    pub unpreserved_cs_reg_count: u32,
}

const _: () = assert!(core::mem::size_of::<OsalMaliGpu>() == 16);

/// The memory the OSAL runs on, each part mapped uncached for the CPU: the
/// queue's AS0 memory (user pages, suspend buffers; at
/// [`z4::QUEUE_MCU_VA`]), the four group-VM regions' backings (the
/// module's table), and the group VM's table pages at `vm_tables_pa`.
/// `heap_initial_chunks` of the chunk region's 2 MiB slots are the heap's
/// at the start; the others are its growth (1 to all of them, at most
/// [`MAX_HEAP_CHUNKS`]).
pub struct OsalMem<'a> {
    pub queue: McuBacking,
    pub ring: GroupBacking,
    pub heap_pool: GroupBacking,
    pub heap_chunks: GroupBacking,
    pub carve: GroupBacking,
    pub vm_pool: &'a mut [Table],
    pub vm_tables_pa: u64,
    pub heap_initial_chunks: u32,
}

/// The tiler heap's memory ([`HeapMemory`]): the pool region, handed out
/// once, and the chunk region's 2 MiB slots, the lowest free first, each
/// at its own VA ([`HEAP_CHUNK_VA`] plus its offset). The errors: 1 the
/// pool asked twice or larger than its region, 2 a chunk of another size,
/// 3 no slot free.
struct HeapMem {
    pool: GroupBacking,
    chunks: GroupBacking,
    pool_taken: bool,
    slots: u32,
    used: u64,
}

const _: () = assert!(MAX_HEAP_CHUNKS <= u64::BITS as usize);

impl HeapMem {
    const EMPTY: HeapMem = HeapMem {
        // SAFETY: empty backings; nothing is read or written through them.
        pool: unsafe { GroupBacking::new(0, 0, 0) },
        chunks: unsafe { GroupBacking::new(0, 0, 0) },
        pool_taken: false,
        slots: 0,
        used: 0,
    };
}

// SAFETY: the pool and the chunk region are the OSAL's backings, mapped in
// the group VM at HEAP_POOL_VA and HEAP_CHUNK_VA (`regions`) for the GPU
// and uncached for the CPU, used by nothing else (OsalMem's contract, which
// GroupBacking::new's carries); the pool is handed out once and each chunk
// slot to one allocation at a time, so no two allocations overlap.
unsafe impl HeapMemory for HeapMem {
    type Error = u32;
    fn alloc(&mut self, size: u64, what: HeapObject) -> Result<Allocation, u32> {
        match what {
            HeapObject::Pool => {
                if self.pool_taken || size > self.pool.size() {
                    return Err(1);
                }
                self.pool_taken = true;
                Ok(Allocation { cpu: self.pool.cpu(), gpu: HEAP_POOL_VA, size })
            }
            HeapObject::Chunk => {
                if size != u64::from(HEAP_CHUNK_SIZE) {
                    return Err(2);
                }
                let i = (!self.used).trailing_zeros();
                if i >= self.slots {
                    return Err(3);
                }
                self.used |= 1 << i;
                let off = u64::from(i) * size;
                Ok(Allocation { cpu: self.chunks.cpu() + off as usize, gpu: HEAP_CHUNK_VA + off, size })
            }
        }
    }
    fn free(&mut self, a: Allocation) {
        if a.gpu >= HEAP_CHUNK_VA {
            let i = (a.gpu - HEAP_CHUNK_VA) / u64::from(HEAP_CHUNK_SIZE);
            self.used &= !(1u64 << i);
        }
    }
}

/// The heap's `grow` for [`sched::service_tiler_oom`]: [`heap::grow`] of
/// the heap at `parts` and `list` for the event's heap context and counts.
/// Its value; 0 for panthor's `-ENOMEM` (more render passes in flight than
/// the target, every chunk made, no slot: the firmware waits); any other
/// refusal is [`SchedRefusal::HeapGrow`] with [`z4::heap_error_code`]'s
/// number, as panthor makes the queue fatal (panthor_sched.c:1496-1499).
fn grow_heap(
    pool: Option<&HeapPool>,
    mem: &mut HeapMem,
    parts: &mut HeapParts,
    list: &mut [Allocation],
    e: &TilerOom,
) -> Result<u64, (SchedRefusal, u32)> {
    let refused = |code: u32| (SchedRefusal::HeapGrow, code);
    let (Some(pool), Some(h)) = (pool, heap::Heap::from_parts(*parts, list)) else {
        return Err(refused(z4::heap_error_code(&HeapError::<u32>::UnknownHeap { heap_gpu_va: e.heap_address }).0));
    };
    let mut heaps = [h];
    match heap::grow(pool, mem, &mut heaps, e.heap_address, e.renderpasses_in_flight(), e.pending_frag_count()) {
        Ok(v) => {
            *parts = heaps[0].parts();
            Ok(v)
        }
        Err(
            HeapError::InFlightAboveTarget { .. } | HeapError::MaxChunksReached { .. } | HeapError::ChunkAlloc { .. },
        ) => Ok(0),
        Err(err) => Err(refused(z4::heap_error_code(&err).0)),
    }
}

/// The group VM's regions over `mem`'s backings: each backing cut to whole
/// pages (the chunks' to whole chunks).
pub fn regions(mem: &OsalMem) -> [Region; 4] {
    let chunk = u64::from(HEAP_CHUNK_SIZE);
    [
        Region { va: QUEUE_VA, size: QUEUE_VM_BYTES, mapping: Mapping::Queue },
        Region { va: HEAP_POOL_VA, size: HEAP_POOL_SIZE, mapping: Mapping::Heap },
        Region { va: HEAP_CHUNK_VA, size: mem.heap_chunks.size() / chunk * chunk, mapping: Mapping::Heap },
        Region { va: CARVE_VA, size: mem.carve.size() / PAGE * PAGE, mapping: Mapping::User },
    ]
}

/// The firmware's sections placed one after the other in `arena`, each
/// from a 4 KiB boundary, in [`fw::Image::sections`]'s order: the backings
/// [`Osal::up`] takes. `None` for an image that does not parse or an arena
/// too small. The arena is uncached memory the MCU may run from.
pub fn place_sections(blob: &[u8], arena: Backing) -> Option<([Backing; fw::MAX_SECTIONS], usize)> {
    let img = fw::parse(blob).ok()?;
    let mut out = [Backing::default(); fw::MAX_SECTIONS];
    let mut at = 0u64;
    for (o, s) in out.iter_mut().zip(img.sections()) {
        let size = u64::from(s.va_size());
        if at + size > arena.size {
            return None;
        }
        *o = Backing { pa: arena.pa + at, cpu: arena.cpu + at as usize, size };
        at += size.next_multiple_of(PAGE);
    }
    Some((out, img.sections().len()))
}

/// One allocation: what `gpu_alloc` answers. `handle` goes back in
/// `gpu_free` (the descriptor's `impl`): the allocation's pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alloc {
    pub cpu: usize,
    pub gpu: u64,
    pub phys: u64,
    pub size: usize,
    pub handle: usize,
}

/// A free held until `fence` completes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Held {
    first: u32,
    pages: u32,
    fence: u64,
}

/// Why the OSAL stopped: the sched refusal and its value.
pub type Dead = (SchedRefusal, u32);

/// The OSAL (the module doc). [`Osal::EMPTY`] until [`Osal::up`].
pub struct Osal {
    up: Option<GroupUp>,
    gpu: usize,
    carve: GroupBacking,
    pages: usize,
    /// One bit a page: in use, and the first page of an allocation.
    used: [u64; MAX_CARVE_PAGES / 64],
    head: [u64; MAX_CARVE_PAGES / 64],
    /// Every page below it is in use.
    hint: usize,
    held: [Held; PENDING_FREES],
    n_held: usize,
    submitted: u64,
    completed: u64,
    /// Fence = base + the ring's sequence number (the module doc).
    base: u64,
    /// Lost fences, inclusive ranges, oldest first; at or below `lost_floor`
    /// lost too ([`LOST_RANGES`]).
    lost: [(u64, u64); LOST_RANGES],
    n_lost: usize,
    lost_floor: u64,
    dead: Option<Dead>,
    /// [`Osal::shutdown`] ended the group.
    down: bool,
    /// Restarts so far, and what caused the last.
    pub restarts: u32,
    last_error: Option<Dead>,
    /// The tiler heap: its pool, the heap less its chunk list, the list,
    /// their memory, the chunks it starts with.
    heap_pool: Option<HeapPool>,
    heap_initial: u32,
    heap: HeapParts,
    heap_list: [Allocation; MAX_HEAP_CHUNKS],
    heap_mem: HeapMem,
    /// Pages in use now and at most; frees refused (their pages kept).
    pub live_pages: usize,
    pub live_pages_high: usize,
    pub refused_frees: u32,
    /// `CS_TILER_OOM`s answered with a chunk, and with none.
    pub oom_chunks: u32,
    pub oom_no_chunk: u32,
}

fn bit(map: &[u64], i: usize) -> bool {
    map[i / 64] & 1 << (i % 64) != 0
}

fn set(map: &mut [u64], i: usize, on: bool) {
    if on {
        map[i / 64] |= 1 << (i % 64);
    } else {
        map[i / 64] &= !(1 << (i % 64));
    }
}

impl Osal {
    pub const EMPTY: Osal = Osal {
        up: None,
        gpu: 0,
        // SAFETY: an empty backing; nothing is read or written through it.
        carve: unsafe { GroupBacking::new(0, 0, 0) },
        pages: 0,
        used: [0; MAX_CARVE_PAGES / 64],
        head: [0; MAX_CARVE_PAGES / 64],
        hint: 0,
        held: [Held { first: 0, pages: 0, fence: 0 }; PENDING_FREES],
        n_held: 0,
        submitted: 0,
        completed: 0,
        base: 0,
        lost: [(0, 0); LOST_RANGES],
        n_lost: 0,
        lost_floor: 0,
        dead: None,
        down: false,
        restarts: 0,
        last_error: None,
        heap_pool: None,
        heap_initial: 0,
        heap: HeapParts { id: 0, ctx_gpu: 0, chunk_size: 0, max_chunks: 0, target_in_flight: 0, chunk_count: 0 },
        heap_list: [Allocation { cpu: 0, gpu: 0, size: 0 }; MAX_HEAP_CHUNKS],
        heap_mem: HeapMem::EMPTY,
        live_pages: 0,
        live_pages_high: 0,
        refused_frees: 0,
        oom_chunks: 0,
        oom_no_chunk: 0,
    };

    /// Bring the GPU up and start the group (the module doc): Z1 to Z3 over
    /// `blob`, `backings` (the firmware's sections, [`place_sections`]),
    /// `pool` and `tables_pa` as [`z4::submit_up`] takes them, then the
    /// group over `mem`. `o` gets Z4's record of every step, the stopping
    /// one named in `o.step`; `Err` with it. `z1` gets Z1's whole record
    /// ([`z4::submit_up`]'s). Called again while up: `Ok`, and nothing runs.
    #[allow(clippy::too_many_arguments)]
    pub fn up(
        &mut self,
        m: &impl Mmio,
        b: Bases,
        blob: &[u8],
        backings: &[Backing],
        pool: &mut [Table],
        tables_pa: u64,
        timer_hz: u32,
        mem: OsalMem<'_>,
        clean: &impl Fn(usize, usize) -> i32,
        delay_us: &mut impl FnMut(u32),
        on_handover: &mut impl FnMut(&GpuUp),
        z1: &mut GpuUp,
        o: &mut SubmitUp,
    ) -> Result<(), u32> {
        if self.up.is_some() {
            return Ok(());
        }
        let pages = (mem.carve.size() / PAGE) as usize;
        if pages == 0 || pages > MAX_CARVE_PAGES || mem.ring.size() < QUEUE_VM_BYTES {
            o.step = z4::STEP_VM_MAP;
            o.refusal = 0;
            o.detail = if mem.ring.size() < QUEUE_VM_BYTES { mem.ring.size() } else { mem.carve.size() };
            return Err(o.step);
        }
        let regions = regions(&mem);
        let slots = (regions[2].size / u64::from(HEAP_CHUNK_SIZE)).min(MAX_HEAP_CHUNKS as u64) as u32;
        let initial = mem.heap_initial_chunks;
        if initial == 0 || initial > slots {
            let e = HeapError::<u32>::InitialAboveMax { initial_chunk_count: initial, max_chunks: slots };
            let (code, v) = z4::heap_error_code(if initial == 0 { &HeapError::ZeroInitialChunks } else { &e });
            (o.step, o.refusal, o.detail) = (z4::STEP_HEAP, code, v);
            return Err(o.step);
        }
        let backs = [mem.ring, mem.heap_pool, mem.heap_chunks, mem.carve];
        let heap =
            HeapPlace { pool_va: HEAP_POOL_VA, chunk_va: HEAP_CHUNK_VA, chunk_size: HEAP_CHUNK_SIZE, chunks: initial };
        let z4mem = Z4Mem { queue: mem.queue, vm: &backs, vm_pool: mem.vm_pool, vm_tables_pa: mem.vm_tables_pa };
        let up = z4::group_up(
            m,
            b,
            blob,
            backings,
            pool,
            tables_pa,
            timer_hz,
            z4mem,
            &regions,
            QUEUE_VA,
            &heap,
            clean,
            delay_us,
            on_handover,
            z1,
            o,
        )
        .ok_or(o.step)?;
        *self = Osal::EMPTY;
        self.heap_mem = HeapMem { pool: mem.heap_pool, chunks: mem.heap_chunks, pool_taken: false, slots, used: 0 };
        // the heap group_up made, made again by the OSAL's own memory so
        // that it can grow it: the same bytes at the same addresses
        let made = self.make_heap(up.id.l2_features, initial);
        if let Err((code, v)) = made.and_then(|h| if h == up.heap { Ok(()) } else { Err((18, h.first_chunk_gpu)) }) {
            (o.step, o.refusal, o.detail) = (z4::STEP_HEAP, code, v);
            o.ended(&sched::stop_and_terminate(m, b.gpu, &up.iface, &up.csg, &up.cs, delay_us));
            *self = Osal::EMPTY;
            return Err(o.step);
        }
        self.up = Some(up);
        self.gpu = b.gpu;
        self.carve = mem.carve;
        self.pages = pages;
        self.heap_initial = initial;
        Ok(())
    }

    /// The tiler heap over [`HeapMem`]: its pool, then `initial` chunks of
    /// Mesa's size, [`HeapMem`]'s slots its most; what the query answers.
    /// [`Osal::up`] refuses one that is not group_up's (`STEP_HEAP`,
    /// refusal 18 with its first chunk: the backend's tiler context names
    /// that chunk). The error: [`z4::heap_error_code`]'s.
    fn make_heap(&mut self, l2_features: u32, initial: u32) -> Result<OsalMaliHeap, (u32, u64)> {
        let mut pool = HeapPool::create(&mut self.heap_mem, l2_features).map_err(|e| z4::heap_error_code(&e))?;
        let params = self.heap_params(initial);
        let h = heap::create(&mut pool, &mut self.heap_mem, &params, &mut self.heap_list)
            .map_err(|e| z4::heap_error_code(&e))?;
        let (parts, made) = (h.parts(), h.osal());
        self.heap = parts;
        self.heap_pool = Some(pool);
        Ok(made)
    }

    fn heap_params(&self, initial: u32) -> HeapParams {
        HeapParams {
            initial_chunk_count: initial,
            chunk_size: HEAP_CHUNK_SIZE,
            max_chunks: self.heap_mem.slots,
            target_in_flight: HeapParams::MESA.target_in_flight,
        }
    }

    /// The heap given back and made again as [`Osal::up`] made it: its
    /// grown chunks back in their slots, its context zeroed, its initial
    /// chunks linked again in the same slots, so its first chunk, which the
    /// backend's tiler context names, is where it was. Refused
    /// ([`SchedRefusal::HeapRemake`]) with [`z4::heap_error_code`]'s number,
    /// or 18 for a heap that is not `u`'s.
    fn remake_heap(&mut self, u: &GroupUp) -> Result<(), Dead> {
        let refused = |e: &HeapError<u32>| (SchedRefusal::HeapRemake, z4::heap_error_code(e).0);
        let params = self.heap_params(self.heap_initial);
        let unknown = HeapError::UnknownHeap { heap_gpu_va: self.heap.ctx_gpu };
        let (Some(pool), Some(h)) = (self.heap_pool.as_mut(), heap::Heap::from_parts(self.heap, &mut self.heap_list))
        else {
            return Err(refused(&unknown));
        };
        let list = heap::destroy(pool, &mut self.heap_mem, h).map_err(|(e, _)| refused(&e))?;
        let h = heap::create(pool, &mut self.heap_mem, &params, list).map_err(|e| refused(&e))?;
        let (parts, made) = (h.parts(), h.osal());
        self.heap = parts;
        if made != u.heap {
            return Err((SchedRefusal::HeapRemake, 18));
        }
        Ok(())
    }

    /// The `grow` [`sched::service_tiler_oom`] calls ([`grow_heap`]), each
    /// answer counted.
    fn grower(&mut self) -> impl FnMut(&TilerOom) -> Result<u64, (SchedRefusal, u32)> + '_ {
        let (pool, mem, parts, list) =
            (self.heap_pool.as_ref(), &mut self.heap_mem, &mut self.heap, &mut self.heap_list);
        let (chunks, none) = (&mut self.oom_chunks, &mut self.oom_no_chunk);
        move |e| {
            let v = grow_heap(pool, mem, parts, list, e)?;
            if v != 0 {
                *chunks += 1;
            } else {
                *none += 1;
            }
            Ok(v)
        }
    }

    /// The heap's chunks now, oldest first.
    pub fn heap_chunks(&self) -> &[Allocation] {
        &self.heap_list[..self.heap.chunk_count as usize]
    }

    /// A pending `CS_TILER_OOM` handled ([`sched::service_tiler_oom`]).
    fn service(&mut self, m: &impl Mmio, u: &GroupUp) -> Result<(), Dead> {
        let gpu = self.gpu;
        sched::service_tiler_oom(m, gpu, &u.iface, &u.csg, &u.cs, &mut self.grower()).map(|_| ())
    }

    /// The group is up.
    pub fn is_up(&self) -> bool {
        self.up.is_some()
    }

    /// Why the OSAL stopped taking kicks, if it did.
    pub fn dead(&self) -> Option<Dead> {
        self.dead
    }

    /// [`Osal::shutdown`] ended the group.
    pub fn is_down(&self) -> bool {
        self.down
    }

    /// What caused the last restart ([`Osal::recover`]) or the stop.
    pub fn last_error(&self) -> Option<Dead> {
        self.last_error
    }

    /// Kicks submitted, and the highest completed.
    pub fn fences(&self) -> (u64, u64) {
        (self.submitted, self.completed)
    }

    /// Frees held now.
    pub fn held(&self) -> usize {
        self.n_held
    }

    /// `OSAL_QUERY_MALI_GPU`: `GPU_ID` and `TILER_FEATURES` read after Z3,
    /// CS 0's work registers and the ones the firmware does not keep
    /// (csf.rs: `cs_features_work_regs`, `UNPRESERVED_REG_COUNT`).
    pub fn gpu_info(&self) -> Option<OsalMaliGpu> {
        let u = self.up.as_ref()?;
        Some(OsalMaliGpu {
            gpu_id: u.id.gpu_id,
            tiler_features: u.id.tiler_features,
            cs_reg_count: csf::cs_features_work_regs(u.cs.features),
            unpreserved_cs_reg_count: csf::UNPRESERVED_REG_COUNT,
        })
    }

    /// `OSAL_QUERY_MALI_TILER_HEAP`: the heap [`Osal::up`] made.
    pub fn heap(&self) -> Option<OsalMaliHeap> {
        Some(self.up.as_ref()?.heap)
    }

    fn at(&self, first: usize, pages: usize) -> Alloc {
        let off = (first as u64) * PAGE;
        Alloc {
            cpu: self.carve.cpu() + off as usize,
            gpu: CARVE_VA + off,
            phys: self.carve.pa() + off,
            size: pages * PAGE as usize,
            handle: pages,
        }
    }

    /// `gpu_alloc`: `size` bytes, rounded up to whole pages, first fit in
    /// the carve-out. `-EINVAL` for no bytes or before [`Osal::up`],
    /// `-ENOMEM` when no run of free pages is long enough.
    pub fn alloc(&mut self, size: usize) -> Result<Alloc, i32> {
        if self.up.is_none() || size == 0 {
            return Err(-EINVAL);
        }
        let n = size.div_ceil(PAGE as usize);
        let mut run = 0;
        let mut i = self.hint;
        while i < self.pages {
            if bit(&self.used, i) {
                run = 0;
            } else {
                run += 1;
                if run == n {
                    let first = i + 1 - n;
                    for p in first..=i {
                        set(&mut self.used, p, true);
                    }
                    set(&mut self.head, first, true);
                    while self.hint < self.pages && bit(&self.used, self.hint) {
                        self.hint += 1;
                    }
                    self.live_pages += n;
                    self.live_pages_high = self.live_pages_high.max(self.live_pages);
                    return Ok(self.at(first, n));
                }
            }
            i += 1;
        }
        Err(-ENOMEM)
    }

    fn release(&mut self, first: usize, pages: usize) {
        for p in first..first + pages {
            set(&mut self.used, p, false);
        }
        set(&mut self.head, first, false);
        self.hint = self.hint.min(first);
        self.live_pages -= pages;
    }

    /// Give back every held free whose fence has completed.
    fn drain(&mut self) {
        let mut k = 0;
        while k < self.n_held {
            let h = self.held[k];
            if h.fence <= self.completed {
                self.release(h.first as usize, h.pages as usize);
                self.n_held -= 1;
                self.held[k] = self.held[self.n_held];
            } else {
                k += 1;
            }
        }
    }

    /// `gpu_free` of the allocation at `gpu` with `handle` ([`Alloc`]):
    /// held until the kick being recorded completes (fence `submitted +
    /// 1`); after [`Osal::shutdown`] given back at once. `-EINVAL` for one
    /// this OSAL did not hand out or already freed. With [`PENDING_FREES`]
    /// held, src/osal/pending_frees.h's rule: the oldest held fence is
    /// waited for (panthor's 5 s; an error restarts the group, which
    /// completes every fence), and then the free is held; when that fence
    /// is the kick being recorded, which no wait can complete, `-EBUSY`;
    /// when the OSAL is dead, `-ENOSPC`. A refused free's pages are kept
    /// for good and counted.
    pub fn free(&mut self, m: &impl Mmio, gpu: u64, handle: usize, delay_us: &mut impl FnMut(u32)) -> i32 {
        let off = gpu.wrapping_sub(CARVE_VA);
        let first = (off / PAGE) as usize;
        if self.up.is_none()
            || gpu < CARVE_VA
            || !off.is_multiple_of(PAGE)
            || first >= self.pages
            || handle == 0
            || handle > self.pages - first
            || !bit(&self.head, first)
            || (first..first + handle).any(|p| !bit(&self.used, p))
        {
            return -EINVAL;
        }
        // a second free of the same pages finds the head already gone
        set(&mut self.head, first, false);
        self.completed(m, delay_us);
        if self.n_held == PENDING_FREES {
            // src/osal/pending_frees.h's rule: wait for the oldest fence,
            // unless it is the kick being recorded
            let oldest = self.held[..self.n_held].iter().map(|h| h.fence).min().unwrap_or(0);
            if oldest > self.submitted {
                self.refused_frees += 1;
                return -EBUSY;
            }
            if self.runs() {
                self.wait(m, oldest as i64, -1, delay_us);
            }
            if self.n_held == PENDING_FREES {
                self.refused_frees += 1;
                return -ENOSPC;
            }
        }
        // after the shutdown no kick comes: the pages go back at once
        let fence = if self.down { self.submitted } else { self.submitted + 1 };
        self.held[self.n_held] = Held { first: first as u32, pages: handle as u32, fence };
        self.n_held += 1;
        self.drain();
        0
    }

    /// `map_phys`: `size` bytes at physical `phys` inside the carve-out, at
    /// their own GPU VA and CPU address (no new mapping, and nothing to
    /// free). `-ENOTSUP` for a range the carve-out does not hold.
    pub fn window(&self, phys: u64, size: usize) -> Result<Alloc, i32> {
        let off = phys.wrapping_sub(self.carve.pa());
        let end = (self.pages as u64) * PAGE;
        if self.up.is_none() || size == 0 {
            return Err(-EINVAL);
        }
        if phys < self.carve.pa() || off >= end || size as u64 > end - off {
            return Err(-ENOTSUP);
        }
        Ok(Alloc { cpu: self.carve.cpu() + off as usize, gpu: CARVE_VA + off, phys, size, handle: 0 })
    }

    fn stop(&mut self, why: Dead) -> i32 {
        self.dead.get_or_insert(why);
        -EIO
    }

    /// The group takes kicks: neither dead nor shut down.
    fn runs(&self) -> bool {
        self.dead.is_none() && !self.down
    }

    /// `shutdown`: the last kick waited for (panthor's 5 s, an error
    /// restarting the group as any wait's does), then the group ended as
    /// panthor ends a destroyed one: `panthor_group_destroy`
    /// (panthor_sched.c:3582-3611) marks it so that it cannot run
    /// (`group_can_run`, :1925-1931) and the next tick terminates it. Here
    /// CS 0 is stopped and CSG 0 terminated ([`sched::stop_and_terminate`])
    /// at once, and a kick still not complete is lost. Later submits and
    /// waits are `-EIO`; frees go on (nothing runs: every page held comes
    /// back). The memory stays the image's. Both ends' results; `None` when
    /// the group is not up, already shut down, or dead (a dead OSAL's group
    /// needs the GPU reset, not a terminate).
    pub fn shutdown(&mut self, m: &impl Mmio, delay_us: &mut impl FnMut(u32)) -> Option<Ended> {
        if !self.runs() {
            return None;
        }
        if self.completed < self.submitted {
            self.wait(m, self.submitted as i64, -1, delay_us);
        }
        let u = self.up.filter(|_| self.runs())?;
        let ended = sched::stop_and_terminate(m, self.gpu, &u.iface, &u.csg, &u.cs, delay_us);
        if self.completed < self.submitted {
            self.lose(self.completed + 1, self.submitted);
            self.completed = self.submitted;
        }
        self.down = true;
        self.drain();
        Some(ended)
    }

    /// Fences `from..=to` lost; the oldest range dropped into
    /// `lost_floor` when [`LOST_RANGES`] are kept.
    fn lose(&mut self, from: u64, to: u64) {
        if self.n_lost == LOST_RANGES {
            self.lost_floor = self.lost[0].1;
            self.lost.copy_within(1.., 0);
            self.n_lost -= 1;
        }
        self.lost[self.n_lost] = (from, to);
        self.n_lost += 1;
    }

    /// `fence` was lost to a restart ([`Osal::lose`]).
    fn is_lost(&self, fence: u64) -> bool {
        fence <= self.lost_floor || self.lost[..self.n_lost].iter().any(|&(a, b)| (a..=b).contains(&fence))
    }

    /// After `why`, which needs panthor's GPU reset: a group state none of
    /// the four (panthor_sched.c:1269-1270), a `CS_FATAL` of type
    /// `CS_UNRECOVERABLE` (:1373-1377), a start or terminate not
    /// acknowledged (:2239, :2298), and ours: CS 0's start not
    /// acknowledged, the slot not free for the restart.
    fn needs_reset(why: Dead) -> bool {
        use SchedRefusal::*;
        matches!(why.0, CsgStateUnknown | CsgStartAckTimeout | CsStartAckTimeout | CsgTerminateAckTimeout | CsgSlotBusy)
            || (why.0 == CsFatal && csf::cs_exception_type(why.1) == sched::EXCEPTION_CS_UNRECOVERABLE)
    }

    /// The group restarted after `why` (the module doc, "Recovery"):
    /// every kick not complete lost and its fence completed, CS 0 stopped
    /// and CSG 0 terminated (`ended` when the timeout did both already),
    /// the heap made again ([`Osal::remake_heap`]), CSG 0 / CS 0 started
    /// again ([`sched::restart`]), the queue-init job run and waited for
    /// ([`Osal::run_init`]). `-EIO` either way: the kick that saw `why` is
    /// lost. A step refused, or `why` one that needs a GPU reset
    /// ([`Osal::needs_reset`]), leaves the OSAL dead.
    fn recover(&mut self, m: &impl Mmio, why: Dead, ended: Option<Ended>, delay_us: &mut impl FnMut(u32)) -> i32 {
        let Some(u) = self.up else { return -EINVAL };
        self.last_error = Some(why);
        if self.completed < self.submitted {
            self.lose(self.completed + 1, self.submitted);
            self.completed = self.submitted;
            self.drain();
        }
        if Self::needs_reset(why) {
            return self.stop(why);
        }
        let ended = ended.unwrap_or_else(|| sched::stop_and_terminate(m, self.gpu, &u.iface, &u.csg, &u.cs, delay_us));
        if let Err(r) = ended.terminate {
            return self.stop(r);
        }
        if let Err(r) = self.remake_heap(&u) {
            return self.stop(r);
        }
        let started =
            match sched::restart(m, self.gpu, &u.iface, &u.csg, &u.cs, &u.group, &u.queue, &u.started, delay_us) {
                Ok(s) => s,
                Err(r) => return self.stop(r),
            };
        self.up = Some(GroupUp { started, ..u });
        if let Err(r) = self.run_init(m, delay_us) {
            return self.stop(r);
        }
        self.restarts += 1;
        -EIO
    }

    /// The queue-init stream ([`z4::QUEUE_INIT`]: `HEAP_SET` of the heap
    /// context) written at [`INIT_VA`] and run as the restarted ring's
    /// first job, waited for with panthor's 5 s. Its fence is `submitted`,
    /// already complete: `base` is set so.
    fn run_init(&mut self, m: &impl Mmio, delay_us: &mut impl FnMut(u32)) -> Result<(), Dead> {
        let Some(u) = self.up else { return Err((SchedRefusal::JobParams, 0)) };
        let s = &u.started;
        let cpu = s.syncobj_cpu + (INIT_VA - u.queue.syncobj_va) as usize;
        for (i, w) in z4::QUEUE_INIT.iter().enumerate() {
            m.write32(cpu + 8 * i, *w as u32);
            m.write32(cpu + 8 * i + 4, (*w >> 32) as u32);
        }
        self.base = self.submitted.wrapping_sub(1);
        let job = Job {
            cs_start: INIT_VA,
            cs_size: 8 * z4::QUEUE_INIT.len() as u32,
            latest_flush: m.read32(self.gpu + LATEST_FLUSH_ID),
        };
        sched::submit(m, self.gpu, &u.cs, &u.queue, s, &job, 1, 0)?;
        let gpu = self.gpu;
        let r = sched::wait_seqno(m, gpu, &u.iface, &u.csg, &u.cs, s, 1, delay_us, &mut self.grower());
        r.map(|_| ()).map_err(|r| r.refused)
    }

    /// `submit` of a kick: one job whose `CALL` runs `stream_bytes` at
    /// `stream_gpu` ([`sched::submit`]), `latest_flush` the GPU's
    /// `GPU_LATEST_FLUSH_ID` now; its fence, `submitted + 1`. The ring's
    /// room is the jobs not yet seen complete ([`Osal::in_flight`]), as
    /// panthor's scheduler credits; a full ring waits for the oldest job
    /// first, as panthor's scheduler holds a job back until its credits
    /// fit, and then has room: a full ring is never a refusal and never
    /// restarts the group. A pending tiler OOM is handled first; an error
    /// seen here restarts the group ([`Osal::recover`]) and the kick goes
    /// on the new ring. `-EINVAL` for a stream outside the carve-out, not
    /// 64-byte aligned or not a non-zero multiple of 8 bytes; `-EIO` when
    /// the OSAL is dead or the kick was refused twice.
    pub fn submit(&mut self, m: &impl Mmio, stream_gpu: u64, stream_bytes: u32, delay_us: &mut impl FnMut(u32)) -> i64 {
        if self.up.is_none() {
            return -i64::from(EINVAL);
        }
        let end = CARVE_VA + (self.pages as u64) * PAGE;
        if stream_gpu < CARVE_VA
            || !stream_gpu.is_multiple_of(64)
            || stream_bytes == 0
            || !stream_bytes.is_multiple_of(8)
            || u64::from(stream_bytes) > end - stream_gpu.min(end)
        {
            return -i64::from(EINVAL);
        }
        let job =
            Job { cs_start: stream_gpu, cs_size: stream_bytes, latest_flush: m.read32(self.gpu + LATEST_FLUSH_ID) };
        for _ in 0..2 {
            let Some(u) = self.up.filter(|_| self.runs()) else { break };
            if self.in_flight() + JOB_BYTES > u64::from(u.queue.ring_size) {
                // the oldest job's completion gives its JOB_BYTES back; an
                // error the wait meets restarts the group with an empty
                // ring, or leaves the OSAL dead
                self.wait(m, (self.completed + 1) as i64, -1, delay_us);
            }
            let Some(u) = self.up.filter(|_| self.runs()) else { break };
            if let Err(why) = self.service(m, &u) {
                self.recover(m, why, None, delay_us);
                continue;
            }
            let ring = (self.submitted + 1).wrapping_sub(self.base);
            match sched::submit(m, self.gpu, &u.cs, &u.queue, &u.started, &job, ring, self.in_flight()) {
                Ok(_) => {
                    self.submitted += 1;
                    return self.submitted as i64;
                }
                Err(why) => {
                    self.recover(m, why, None, delay_us);
                }
            }
        }
        -i64::from(EIO)
    }

    /// The ring bytes of the kicks not yet seen complete: panthor's
    /// scheduler credits ([`sched::submit`]'s `in_flight`).
    fn in_flight(&self) -> u64 {
        (self.submitted - self.completed) * JOB_BYTES
    }

    /// The sync object reached ring sequence number `seqno`.
    fn reached(&mut self, seqno: u64) {
        let fence = self.base.wrapping_add(seqno);
        self.completed = self.completed.max(fence.min(self.submitted));
        self.drain();
    }

    /// `wait` for `fence`: 0 once the sync object reached it, `-EIO` for a
    /// fence lost to a restart. A timeout below [`JOB_TIMEOUT_US`] polls
    /// for that long and returns 1 when it runs out, the job left running
    /// ([`sched::poll_seqno`]); a negative or longer one waits panthor's 5 s
    /// ([`sched::wait_seqno`]). Either handles a tiler OOM; an error, the
    /// 5 s timeout among them, restarts the group ([`Osal::recover`]) and is
    /// `-EIO`: the fence is lost. `-EINVAL` for a fence never submitted.
    pub fn wait(&mut self, m: &impl Mmio, fence: i64, timeout_us: i64, delay_us: &mut impl FnMut(u32)) -> i32 {
        let Some(u) = self.up else { return -EINVAL };
        if fence <= 0 || fence as u64 > self.submitted {
            return -EINVAL;
        }
        let target = fence as u64;
        if target <= self.completed {
            return if self.is_lost(target) { -EIO } else { 0 };
        }
        if !self.runs() {
            return -EIO;
        }
        let ring = target.wrapping_sub(self.base);
        let (gpu, iface, csg, cs, s) = (self.gpu, &u.iface, &u.csg, &u.cs, &u.started);
        if (0..i64::from(JOB_TIMEOUT_US)).contains(&timeout_us) {
            let budget = timeout_us as u32;
            let r = sched::poll_seqno(m, gpu, iface, csg, cs, s, ring, budget, delay_us, &mut self.grower());
            return match r {
                Ok(Some(w)) => {
                    self.reached(w.seqno);
                    0
                }
                Ok(None) => 1,
                Err(r) => self.recover(m, r.refused, r.ended, delay_us),
            };
        }
        let r = sched::wait_seqno(m, gpu, iface, csg, cs, s, ring, delay_us, &mut self.grower());
        match r {
            Ok(w) => {
                self.reached(w.seqno);
                0
            }
            Err(r) => self.recover(m, r.refused, r.ended, delay_us),
        }
    }

    /// `completed`: the highest fence the sync object has reached, read
    /// without waiting (a pending tiler OOM handled first, the group's
    /// errors looked at, [`sched::group_errors`]); held frees up to it given
    /// back. An error restarts the group ([`Osal::recover`]), which
    /// completes every fence. Once dead, the last one known.
    pub fn completed(&mut self, m: &impl Mmio, delay_us: &mut impl FnMut(u32)) -> i64 {
        if let Some(u) = self.up.filter(|_| self.runs()) {
            let r = self
                .service(m, &u)
                .and_then(|()| sched::group_errors(m, &u.csg, &u.cs))
                .and_then(|()| sched::completed_seqno(m, &u.started));
            match r {
                Ok(v) => self.reached(v),
                Err(why) => {
                    self.recover(m, why, None, delay_us);
                }
            }
        }
        self.completed as i64
    }
}

#[cfg(test)]
mod tests;
