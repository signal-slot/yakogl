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
//! | the heap's chunks, all made at the start | [`HEAP_CHUNK_VA`] | [`Mapping::Heap`] |
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
//! What this does not do (K2): a fault, fatal, tiler OOM, protected-mode
//! or progress event, a refused wait or the 5 s job timeout leaves the OSAL
//! dead ([`Osal::dead`]): every later submit and wait is `-EIO`, nothing is
//! reset or restarted. The heap never grows. `map_phys` outside the
//! carve-out is `-ENOTSUP`.

use super::nanopc_t6_gpu::{Bases, GpuUp};
use super::nanopc_t6_sections::Backing;
use super::nanopc_t6_submit::{
    self as z4, GroupBacking, GroupUp, HeapPlace, Mapping, McuBacking, Region, SubmitUp, Z4Mem, HEAP_CTX_VA,
    HEAP_POOL_SIZE, LATEST_FLUSH_ID, QUEUE_VM_BYTES,
};
use crate::errno::{EINVAL, EIO, ENOMEM, ENOSPC, ENOTSUP};
use crate::gpu::mali::heap::OsalMaliHeap;
use crate::gpu::mali::pgtable::{Table, PAGE};
use crate::gpu::mali::sched::job::JOB_BYTES;
use crate::gpu::mali::sched::{self, Job, SchedRefusal, JOB_TIMEOUT_US};
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
pub struct OsalMem<'a> {
    pub queue: McuBacking,
    pub ring: GroupBacking,
    pub heap_pool: GroupBacking,
    pub heap_chunks: GroupBacking,
    pub carve: GroupBacking,
    pub vm_pool: &'a mut [Table],
    pub vm_tables_pa: u64,
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
    dead: Option<Dead>,
    /// Pages in use now and at most; frees refused (their pages kept).
    pub live_pages: usize,
    pub live_pages_high: usize,
    pub refused_frees: u32,
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
        dead: None,
        live_pages: 0,
        live_pages_high: 0,
        refused_frees: 0,
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
        if pages == 0 || pages > MAX_CARVE_PAGES {
            o.step = z4::STEP_VM_MAP;
            o.refusal = 0;
            o.detail = mem.carve.size();
            return Err(o.step);
        }
        let regions = regions(&mem);
        let backs = [mem.ring, mem.heap_pool, mem.heap_chunks, mem.carve];
        let heap = HeapPlace {
            pool_va: HEAP_POOL_VA,
            chunk_va: HEAP_CHUNK_VA,
            chunk_size: HEAP_CHUNK_SIZE,
            chunks: (regions[2].size / u64::from(HEAP_CHUNK_SIZE)) as u32,
        };
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
        self.up = Some(up);
        self.gpu = b.gpu;
        self.carve = mem.carve;
        self.pages = pages;
        Ok(())
    }

    /// The group is up.
    pub fn is_up(&self) -> bool {
        self.up.is_some()
    }

    /// Why the OSAL stopped taking kicks, if it did.
    pub fn dead(&self) -> Option<Dead> {
        self.dead
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
    /// 1`). `-EINVAL` for one this OSAL did not hand out or already freed;
    /// `-ENOSPC` when [`PENDING_FREES`] are held and none has completed:
    /// its pages are then kept for good and counted.
    pub fn free(&mut self, m: &impl Mmio, gpu: u64, handle: usize) -> i32 {
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
        self.completed(m);
        if self.n_held == PENDING_FREES {
            self.refused_frees += 1;
            return -ENOSPC;
        }
        self.held[self.n_held] = Held { first: first as u32, pages: handle as u32, fence: self.submitted + 1 };
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

    /// `submit` of a kick: one job whose `CALL` runs `stream_bytes` at
    /// `stream_gpu` ([`sched::submit`]), `latest_flush` the GPU's
    /// `GPU_LATEST_FLUSH_ID` now; its fence, the job's sequence number.
    /// The ring's room is the jobs not yet seen complete
    /// ([`Osal::in_flight`]), as panthor's scheduler credits; a full ring
    /// waits for the oldest job first, as panthor's scheduler holds a job
    /// back until its credits fit, and then has room: a full ring is never
    /// a refusal. `-EINVAL` for a stream outside the carve-out, not 64-byte
    /// aligned or not a non-zero multiple of 8 bytes; `-EIO` when sched
    /// refused or the wait for the oldest job did (the OSAL is then dead),
    /// or it already was.
    pub fn submit(&mut self, m: &impl Mmio, stream_gpu: u64, stream_bytes: u32, delay_us: &mut impl FnMut(u32)) -> i64 {
        let Some(u) = self.up else { return -i64::from(EINVAL) };
        let end = CARVE_VA + (self.pages as u64) * PAGE;
        if stream_gpu < CARVE_VA
            || !stream_gpu.is_multiple_of(64)
            || stream_bytes == 0
            || !stream_bytes.is_multiple_of(8)
            || u64::from(stream_bytes) > end - stream_gpu.min(end)
        {
            return -i64::from(EINVAL);
        }
        if self.dead.is_some() {
            return -i64::from(EIO);
        }
        let seqno = self.submitted + 1;
        let job =
            Job { cs_start: stream_gpu, cs_size: stream_bytes, latest_flush: m.read32(self.gpu + LATEST_FLUSH_ID) };
        if self.in_flight() + JOB_BYTES > u64::from(u.queue.ring_size) {
            // the oldest job's completion gives its JOB_BYTES back
            if self.wait(m, (self.completed + 1) as i64, -1, delay_us) != 0 {
                return -i64::from(EIO);
            }
        }
        let r = sched::submit(m, self.gpu, &u.cs, &u.queue, &u.started, &job, seqno, self.in_flight());
        match r {
            Ok(_) => {
                self.submitted = seqno;
                seqno as i64
            }
            Err(why) => i64::from(self.stop(why)),
        }
    }

    /// The ring bytes of the kicks not yet seen complete: panthor's
    /// scheduler credits ([`sched::submit`]'s `in_flight`).
    fn in_flight(&self) -> u64 {
        (self.submitted - self.completed) * JOB_BYTES
    }

    fn reached(&mut self, seqno: u64) {
        self.completed = self.completed.max(seqno.min(self.submitted));
        self.drain();
    }

    /// `wait` for `fence`: 0 once the sync object reached it. A timeout
    /// below [`JOB_TIMEOUT_US`] polls for that long and returns 1 when it
    /// runs out, the job left running ([`sched::poll_seqno`]); a negative
    /// or longer one waits panthor's 5 s and on its timeout stops and
    /// terminates the group ([`sched::wait_seqno`]). `-EINVAL` for a fence
    /// never submitted; `-EIO` for a refusal (the OSAL is then dead).
    pub fn wait(&mut self, m: &impl Mmio, fence: i64, timeout_us: i64, delay_us: &mut impl FnMut(u32)) -> i32 {
        let Some(u) = self.up else { return -EINVAL };
        if fence <= 0 || fence as u64 > self.submitted {
            return -EINVAL;
        }
        let target = fence as u64;
        if target <= self.completed {
            return 0;
        }
        if self.dead.is_some() {
            return -EIO;
        }
        let (iface, csg, cs, s) = (&u.iface, &u.csg, &u.cs, &u.started);
        if (0..i64::from(JOB_TIMEOUT_US)).contains(&timeout_us) {
            return match sched::poll_seqno(m, self.gpu, iface, csg, cs, s, target, timeout_us as u32, delay_us) {
                Ok(Some(w)) => {
                    self.reached(w.seqno);
                    0
                }
                Ok(None) => 1,
                Err(r) => self.stop(r.refused),
            };
        }
        match sched::wait_seqno(m, self.gpu, iface, csg, cs, s, target, delay_us) {
            Ok(w) => {
                self.reached(w.seqno);
                0
            }
            Err(r) => self.stop(r.refused),
        }
    }

    /// `completed`: the highest fence the sync object has reached, read
    /// without waiting; held frees up to it given back. After a refusal
    /// the last one known.
    pub fn completed(&mut self, m: &impl Mmio) -> i64 {
        if let (Some(u), None) = (self.up, self.dead) {
            match sched::completed_seqno(m, &u.started) {
                Ok(v) => self.reached(v),
                Err(why) => {
                    self.stop(why);
                }
            }
        }
        self.completed as i64
    }
}

#[cfg(test)]
mod tests;
