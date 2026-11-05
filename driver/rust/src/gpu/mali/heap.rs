// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Collabora ltd.
//! Z4's tiler heap (host side): the heap context the L1 frame's first
//! instructions name, its chunks and their links, made by our kernel side as
//! panthor's `DRM_IOCTL_PANTHOR_TILER_HEAP_CREATE` makes them.
//!
//! Why: the L1 frame starts its queue with `MOVE d72` / `HEAP_SET d72`
//! (src/mali/frame.rs, `frame::queue_init`). Their operand is the heap
//! context TILER_HEAP_CREATE returns, not the heap descriptor (measured on
//! the board as 0x800000015000; driver/docs/nanopc-t6-mali.md, "What L1
//! must provide", the bullets "A queue-init stream" and "Inputs only the
//! OSAL has"). The backend reads the context, the first chunk and the chunk
//! size through `OSAL_QUERY_MALI_TILER_HEAP` (src/osal/mali.rs,
//! `OsalMaliHeap`; mirrored here as [`OsalMaliHeap`]). Under Zephyr nobody
//! else makes that heap.
//!
//! Derived from Linux v6.18's drivers/gpu/drm/panthor (Copyright 2023
//! Collabora ltd.), used under the MIT option of its `GPL-2.0 or MIT`
//! licence (the design page's decision 1): panthor_heap.c, with one line
//! each of panthor_regs.h, panthor_gem.c, panthor_drv.c, panthor_mmu.c,
//! panthor_mmu.h and the uAPI's include/uapi/drm/panthor_drm.h. Lines below
//! are of v6.18 (`VERSION = 6`, `PATCHLEVEL = 18`, `SUBLEVEL = 0`); a bare
//! `:N` is panthor_heap.c's line N. Where this differs from panthor it says
//! so.
//!
//! Growth on `CS_TILER_OOM` (`panthor_heap_grow`, :416-489: [`grow`]), the
//! return of a chunk the firmware could not take
//! (`panthor_heap_return_chunk`, :359-414: [`return_chunk`]) and a heap's
//! teardown (`panthor_heap_destroy`, :226-255: [`destroy`]) are transcribed
//! too. Nothing calls them in Z4: they are for a later `CS_TILER_OOM`
//! handler in sched.rs, which refuses `CS_TILER_OOM` today.
//!
//! Two points where panthor differs from the plan's wording for growth,
//! both followed as panthor has them: the in-flight refusal is for strictly
//! more render passes than `target_in_flight`, and it is `-ENOMEM`, not
//! `-EBUSY` (:458-461; panthor's handler takes `-ENOMEM` as "let the
//! firmware wait", panthor_sched.c:1491-1501); and a grown chunk's header is
//! zeroed, not linked to the old head (`initial_chunk` false, :168, 475):
//! the firmware links it after the handler writes it to `heap_start` and
//! `heap_end` (panthor_sched.c:1512-1513).
//!
//! # What panthor builds
//!
//! - **One pool per VM.** `panthor_vm_get_heap_pool` (panthor_mmu.c:
//!   1940-1961) makes a VM's pool on its first TILER_HEAP_CREATE and keeps
//!   it in the VM; the ioctl takes the pool of the VM user space names
//!   (panthor_drv.c:1137-1141). A pool is one kernel buffer of heap
//!   contexts, `ALIGN(MAX_HEAPS_PER_POOL * stride, 4096)` bytes
//!   (`panthor_heap_pool_create`, :537-539, 556-560): [`HeapPool`],
//!   [`pool_size`].
//! - **The heap context** is "an opaque structure used by the GPU to track
//!   the heap allocations. The driver should only touch it to initialize it
//!   (zero all fields)" (:15-20): `HEAP_CONTEXT_SIZE` 32 bytes (:21),
//!   rounded up to the GPU's L2 line, `ALIGN(32, 1 << (L2_FEATURES &
//!   0xff))` (`panthor_heap_ctx_stride`, :105-111; panthor_regs.h:22-23):
//!   [`ctx_stride`]. Heap `id` has the slot at `stride * id`
//!   (`panthor_get_heap_ctx_offset`, :113-116; `panthor_get_heap_ctx`,
//!   :118-122): [`ctx_offset`]. The board's `L2_FEATURES` 0x7120306
//!   (super's module doc) gives a 64-byte line: a 64-byte stride, slots at
//!   0, 64, ..., 8128, an 8 KiB pool. Create zeroes the whole stride
//!   (:336), and the context's GPU address is the pool's plus the offset
//!   (:337-338).
//! - **The chunks** are kernel buffers of `chunk_size` bytes in the same VM,
//!   mapped no-exec (:151-155). Each starts with `struct
//!   panthor_heap_chunk_header` (:23-36): a 64-bit `next` and fourteen
//!   32-bit words that must be zero, 64 bytes, zeroed at allocation
//!   (:165-166). `next` links a chunk to the one allocated before it:
//!   `(prev_gpuva & GENMASK_ULL(63, 12)) | (chunk_size >> 12)` (:177-178),
//!   the address and the size in 4 KiB units in one word ([`chunk_link`],
//!   [`chunk_header`]); the oldest chunk's `next` stays 0. panthor adds
//!   each new chunk at the head of its list (`list_add`, :184), so the
//!   list head, which create returns as the first chunk (:321-324), is the
//!   newest, and the chain runs from it back to the oldest. [`Heap`]'s
//!   list keeps them oldest first, the head last.
//! - **Create's checks**, in panthor's order (`panthor_heap_create`,
//!   :287-295): an initial chunk count of 0, more initial chunks than
//!   `max_chunks`, a chunk size not a multiple of `PAGE_SIZE`, below
//!   `SZ_128K`, above `SZ_8M`; each `-EINVAL`. `target_in_flight` is
//!   stored (:315), not checked: only `panthor_heap_grow` reads it (:458).
//!   panthor checks nothing else of the four. It allocates the chunks
//!   (:317-319), takes the list head as the first chunk (:321-324), then
//!   the lowest free slot of the pool (`xa_alloc` over
//!   `[0, MAX_HEAPS_PER_POOL - 1]`, :331-332), zeroes it and computes the
//!   context's address (:334-338). On any failure after the first
//!   allocation it frees every chunk, head first
//!   (`panthor_free_heap_chunks`, :201-208; :343-350): nothing is left
//!   half-built. The ioctl's handle is `vm_id << 16 | id`
//!   (panthor_drv.c:1157-1160).
//!
//! # The interface: memory the caller maps
//!
//! The module maps nothing. [`HeapMemory`] hands out [`Allocation`]s: a CPU
//! address, the GPU address the same bytes have in the group's VM, and the
//! size; it takes them back. It stands for `panthor_kernel_bo_create(vm,
//! size, DRM_PANTHOR_BO_NO_MMAP, DRM_PANTHOR_VM_BIND_OP_MAP_NOEXEC,
//! PANTHOR_VM_KERNEL_AUTO_VA)` with its `vmap` (:151-163, 556-568) and for
//! `panthor_kernel_bo_destroy` (:135, 193). The caller maps the buffer
//! no-exec and, on the RK3588 (no `dma-coherent`), write-combined or
//! uncached for the CPU, as panthor's shmem objects are on a non-coherent
//! GPU (panthor_gem.c:247, `map_wc = !ptdev->coherent`): no cache
//! maintenance follows a write here, as with sched.rs's `GroupVm`.
//!
//! # Where the heap sits, and how this differs from the goal's "AS0"
//!
//! panthor puts the pool and the chunks in the group's VM, never in the
//! MCU's: the pool is the VM's (panthor_mmu.c:1940-1961), its buffers are
//! created on `pool->vm` (:151, 556), at an address the kernel picks from
//! the VM's kernel range (`PANTHOR_VM_KERNEL_AUTO_VA`, panthor_mmu.h:72-75;
//! `panthor_vm_alloc_va`, panthor_mmu.c:1042-1070, 4 KiB aligned, 2 MiB for
//! buffers of 2 MiB or more, :1061-1065). That range is the top power of
//! two of the VA space above user space (panthor_mmu.c:1493-1495): with
//! the G610's 48 VA bits and Mesa's 2^47 bytes of user VA it starts at
//! 0x8000_0000_0000, where the measured context 0x8000_0001_5000 lies.
//! The tiler reads the context and the chunks through the group's address
//! space, the one `HEAP_SET`'s stream runs in.
//!
//! sched.rs (plan #194) splits the queue memory as panthor does: the ring
//! buffer and the sync object in the group's VM ([`super::sched::GROUP_AS`]
//! 1), the user input/output pages and the suspend buffers in the MCU's AS0.
//! The goal of this plan asked for the heap "in AS0" next to that queue
//! memory. It goes in the group's VM instead, beside the ring and the sync
//! object, because that is where panthor puts it and where the GPU looks:
//! a context in AS0 would be an address the stream cannot reach. In the
//! group's VM, after the queue memory and with room left after it, the plan
//! for Z4 is:
//!
//! - the queue memory (ring, sync object) where sched.rs places it;
//! - the pool at 0x8000_0001_5000, the address panthor's run gave it, so
//!   that heap 0's context, and with it `HEAP_SET`'s operand, is the
//!   trace's 0x800000015000 and t6_submit's frame compares byte for byte;
//! - the chunks above the pool in the same kernel range, each at its own
//!   2 MiB-aligned address for 2 MiB chunks (what `panthor_vm_alloc_va`
//!   would pick, panthor_mmu.c:1062); Mesa's heap is 5 such chunks, 64 at
//!   most (tools/mali/panthor/src/osal.rs), so 128 MiB is reserved for them
//!   and nothing else is placed there.
//!
//! The addresses are the caller's ([`HeapMemory::alloc`]): this module
//! only checks that a chunk is 4 KiB aligned (its low 12 bits carry the
//! size in `next`, :177-178) and the pool too (panthor's buffers are page
//! aligned, panthor_mmu.c:1061-1065, and a context must be cache-line
//! aligned, :15-20).
//!
//! # What we refuse instead of masking
//!
//! Each refusal is a [`HeapError`] variant with the value that decided it:
//! panthor's five parameter checks, one variant each (its single `-EINVAL`
//! for the chunk size, :293-295, is three); a pool with no free slot
//! (`xa_alloc`'s `-EBUSY`, :331-332); an allocator failure, with the
//! allocator's error and the chunk's index; and our own checks of what the
//! allocator handed out (short, not 4 KiB aligned, no CPU address), of the
//! L2 line size (`1 << x` with `x` of 32 or more is not a size,
//! panthor_regs.h:23), and of the caller's chunk list (it must hold
//! `max_chunks` entries, the bound growth stops at, :459). Growth, return
//! and teardown add panthor's own: an unknown heap or chunk (`-EINVAL`,
//! :379-387, 407-408, 443-451, 231-233) and the two `-ENOMEM` bounds of
//! growth (:458-459).
//!
//! Two differences from the plan's wording, both following panthor: the
//! chunk size must be a multiple of 4 KiB, not a power of two (132 KiB is
//! accepted, as panthor accepts it; panthor_drm.h:951-956 says
//! "page-aligned and lie in the [128k:8M] range"), and `target_in_flight`
//! is not checked (:315). The pool cannot be destroyed while a create runs
//! (panthor's `pool->vm` tests, :301-303, 327-330): Rust's borrow of the
//! [`HeapPool`] rules that out. `PAGE_SIZE` is the GPU's 4 KiB granule
//! here (super::pgtable::PAGE), the size every arm64 kernel panthor runs on
//! with 4 KiB pages checks.
//!
//! # Follow-ups (written down only; not changed here)
//!
//! - sched.rs: `CS_TILER_OOM` is refused (`SchedRefusal::TilerOom`). A
//!   handler would read the stream's `heap_address`, `heap_vt_start`,
//!   `heap_vt_end`, `heap_frag_end`, refuse `frag_end > vt_end` or
//!   `vt_end >= vt_start` (panthor_sched.c:1480), call [`grow`] with
//!   `vt_start - frag_end` and `vt_end - frag_end`, write its value (0 on
//!   `InFlightAboveTarget`, `MaxChunksReached` or an allocator's out of
//!   memory, panthor's `-ENOMEM`; any other refusal makes the queue fatal)
//!   to `heap_start`/`heap_end` and acknowledge, then [`return_chunk`] the
//!   chunk if the group was scheduled out meanwhile
//!   (`group_process_tiler_oom`, panthor_sched.c:1448-1532). It also has to
//!   be given the heap's pool and heaps to reach them from the group.
//!   csf.rs already has those `CsInput`/`CsOutput` fields; it needs nothing.
//! - Pool destruction (`panthor_heap_pool_destroy`, :594-616) is not
//!   transcribed: Z4's group VM lives until the reset. panthor's pool
//!   `size` counter (:101-102, 133, 188, 397), read only by its fdinfo, is
//!   not kept.
//! - pgtable.rs: the pool and the chunks are mapped no-exec in the group's
//!   tables at 0x8000_0000_0000 and above; 48-bit VAs are what it builds
//!   already. It maps 4 KiB pages only, so each 2 MiB chunk costs one level
//!   3 table (64 chunks: 64 tables); a 2 MiB block descriptor would save
//!   them.
//! - The Zephyr side: one [`HeapPool`] and one [`Heap`] per group VM, made
//!   before the queue-init stream runs, and `OSAL_QUERY_MALI_TILER_HEAP`
//!   answered from [`Heap::osal`].
//!
//! # For driver/docs/nanopc-t6-mali.md: "The tiler heap (Z4)"
//!
//! The paragraph a later commit moves into the design page, under "Submit
//! (Z4)", once plan #187's edits of that page have merged:
//!
//! **The tiler heap (Z4)** (`gpu::mali::heap`): what panthor's
//! TILER_HEAP_CREATE makes (linux:`panthor_heap.c:273-357`), made by our
//! kernel side in the group's VM, not AS0, because the tiler reads it
//! through the group's address space (panthor's pool is per VM,
//! `panthor_mmu.c:1940-1961`). The pool is one buffer of 128 heap contexts,
//! each `ALIGN(32, L2 line)` bytes, 64 on the G610 (105-122, 537-539);
//! create zeroes heap 0's slot and returns its address, which is
//! `HEAP_SET`'s operand. The chunks (2 MiB, 5 initial, 64 at most, Mesa's
//! values) each start with a 64-byte header whose `next` is the previous
//! chunk's VA with its size in 4 KiB units in the low 12 bits (23-36,
//! 177-178); the newest chunk is the first. Checks in panthor's order
//! (287-295): initial count 0, initial above max, size not 4 KiB aligned,
//! below 128 KiB, above 8 MiB. We place the pool at 0x8000_0001_5000, the
//! measured context, so the frame matches the trace; chunks above it in
//! the kernel VA range. No growth in Z4: `CS_TILER_OOM` is refused.

use core::fmt::Debug;

/// `HEAP_CONTEXT_SIZE` (panthor_heap.c:21): the heap context's size before
/// the L2-line alignment.
pub const HEAP_CONTEXT_SIZE: u32 = 32;

/// `MAX_HEAPS_PER_POOL` (panthor_heap.c:75): the slots of one pool.
pub const MAX_HEAPS_PER_POOL: u32 = 128;

/// The pool buffer's size alignment: the 4096 of `panthor_heap_pool_create`
/// (panthor_heap.c:537-539).
pub const POOL_ALIGN: u64 = 4096;

/// `sizeof(struct panthor_heap_chunk_header)` (panthor_heap.c:26-36): `u64
/// next` and `u32 unknown[14]`, the bytes `memset` zeroes (:166).
pub const CHUNK_HEADER_SIZE: usize = 8 + 14 * 4;

/// The chunk size bounds of `panthor_heap_create` (panthor_heap.c:293-294;
/// panthor_drm.h:951-956): a multiple of `PAGE_SIZE` within
/// `[SZ_128K, SZ_8M]`. `PAGE_SIZE` is the 4 KiB granule (module doc).
pub const CHUNK_ALIGN: u32 = 4096;
pub const CHUNK_MIN_SIZE: u32 = 128 * 1024;
pub const CHUNK_MAX_SIZE: u32 = 8 * 1024 * 1024;

/// `GENMASK_ULL(63, 12)` and the `>> 12` of `next` (panthor_heap.c:177-178).
pub const CHUNK_VA_MASK: u64 = !0xfff;
pub const CHUNK_SIZE_SHIFT: u32 = 12;

/// `GPU_L2_FEATURES_LINE_SIZE(x)` = `1 << (x & GENMASK(7, 0))`
/// (panthor_regs.h:23); `None` for an exponent of 32 or more, which is no
/// 32-bit size (the C shift would be undefined).
pub const fn l2_line_size(l2_features: u32) -> Option<u32> {
    let log2 = l2_features & 0xff;
    if log2 >= 32 {
        return None;
    }
    Some(1 << log2)
}

/// `panthor_heap_ctx_stride` (panthor_heap.c:105-111): `ALIGN(HEAP_CONTEXT_SIZE,
/// line)`, the line a power of two. `None` as [`l2_line_size`].
pub const fn ctx_stride(l2_features: u32) -> Option<u32> {
    match l2_line_size(l2_features) {
        // ALIGN(32, line) for a power-of-two line: 32 below a 32-byte line,
        // the line from there on (computed in u64: a 2^31 line does not
        // overflow)
        Some(line) => {
            let l = line as u64;
            Some(((HEAP_CONTEXT_SIZE as u64 + l - 1) & !(l - 1)) as u32)
        }
        None => None,
    }
}

/// `panthor_get_heap_ctx_offset` (panthor_heap.c:113-116): heap `id`'s slot
/// in the pool buffer.
pub const fn ctx_offset(stride: u32, id: u32) -> u64 {
    stride as u64 * id as u64
}

/// The pool buffer's size (panthor_heap.c:537-539):
/// `ALIGN(MAX_HEAPS_PER_POOL * stride, 4096)`.
pub const fn pool_size(stride: u32) -> u64 {
    (MAX_HEAPS_PER_POOL as u64 * stride as u64 + POOL_ALIGN - 1) & !(POOL_ALIGN - 1)
}

/// A chunk header's `next` pointing at the chunk at `next_va` of
/// `chunk_size` bytes (panthor_heap.c:177-178): `(next_va &
/// GENMASK_ULL(63, 12)) | (chunk_size >> 12)`. panthor's expression as it
/// is: the low 12 bits of `next_va` are dropped, so [`create`] refuses a
/// chunk that has any before it gets here.
pub const fn chunk_link(next_va: u64, chunk_size: u32) -> u64 {
    (next_va & CHUNK_VA_MASK) | (chunk_size >> CHUNK_SIZE_SHIFT) as u64
}

/// The bytes of a `struct panthor_heap_chunk_header` (panthor_heap.c:26-36)
/// whose `next` is `next`: the word little-endian at 0, the fourteen
/// `unknown` words zero ("MBZ", :34). A chunk with no successor has `next`
/// 0 (the `memset`, :166, and no link, :168).
pub const fn chunk_header(next: u64) -> [u8; CHUNK_HEADER_SIZE] {
    let mut h = [0u8; CHUNK_HEADER_SIZE];
    let b = next.to_le_bytes();
    let mut i = 0;
    while i < 8 {
        h[i] = b[i];
        i += 1;
    }
    h
}

/// The creation parameters: `struct drm_panthor_tiler_heap_create`'s inputs
/// other than the VM (panthor_drm.h:944-971), as `panthor_heap_create`
/// takes them (panthor_heap.c:273-279).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeapParams {
    /// Chunks allocated at creation; at least 1 (panthor_heap.c:287).
    pub initial_chunk_count: u32,
    /// Each chunk's size (panthor_heap.c:293-294).
    pub chunk_size: u32,
    /// The most chunks the heap may have; at least the initial count
    /// (panthor_heap.c:290).
    pub max_chunks: u32,
    /// Render passes in flight past which growth leaves the firmware to
    /// wait (panthor_heap.c:65-69); stored, not checked (:315).
    pub target_in_flight: u32,
}

impl HeapParams {
    /// Mesa's heap: 2 MiB chunks, 5 initial, 64 at most, 65535 in flight
    /// (tools/mali/panthor/src/osal.rs: driinfo_panfrost.h:16-21, Mesa
    /// 25.0.7 pan_csf.c:1420-1426). The heap panthor made for the trace.
    pub const MESA: HeapParams =
        HeapParams { initial_chunk_count: 5, chunk_size: 2 * 1024 * 1024, max_chunks: 64, target_in_flight: 65535 };
}

/// What a step refused, with the value that decided it. `E` is the
/// allocator's error ([`HeapMemory::Error`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeapError<E> {
    /// `initial_chunk_count == 0` (panthor_heap.c:287-288).
    ZeroInitialChunks,
    /// `initial_chunk_count > max_chunks` (panthor_heap.c:290-291).
    InitialAboveMax { initial_chunk_count: u32, max_chunks: u32 },
    /// `!IS_ALIGNED(chunk_size, PAGE_SIZE)` (panthor_heap.c:293).
    ChunkSizeNotPageAligned { chunk_size: u32 },
    /// `chunk_size < SZ_128K` (panthor_heap.c:294).
    ChunkSizeBelowMin { chunk_size: u32 },
    /// `chunk_size > SZ_8M` (panthor_heap.c:294).
    ChunkSizeAboveMax { chunk_size: u32 },
    /// Ours: the caller's chunk list holds fewer than `max_chunks` entries,
    /// the count growth may reach (panthor_heap.c:459).
    ChunkListTooShort { len: usize, max_chunks: u32 },
    /// Ours: `L2_FEATURES`' line size exponent is 32 or more
    /// (panthor_regs.h:23).
    L2LineSize { l2_features: u32 },
    /// The pool buffer's allocation failed (panthor_heap.c:556-564).
    PoolAlloc(E),
    /// Chunk `index` (0 the first) failed to allocate
    /// (panthor_heap.c:151-159, 217-220); the earlier ones were given back.
    ChunkAlloc { index: u32, err: E },
    /// Ours: the allocator handed out fewer bytes than asked
    /// (panthor_heap.c:151, 556 ask for exactly `size`).
    ShortAllocation { size: u64, wanted: u64 },
    /// Ours: the allocation's GPU address is not 4 KiB aligned
    /// (panthor's buffers are, panthor_mmu.c:1061-1065; a chunk's low 12 bits
    /// carry its size in `next`, panthor_heap.c:177-178).
    MisalignedAllocation { gpu: u64 },
    /// Ours: the allocation has no CPU address (panthor's `vmap`,
    /// panthor_heap.c:161-163, 566-568).
    NoCpuAddress { gpu: u64 },
    /// Every slot of the pool is taken (`xa_alloc` over
    /// `[0, MAX_HEAPS_PER_POOL - 1]`, panthor_heap.c:331-332); the chunks
    /// were given back (:343-350).
    PoolFull { heaps: u32 },
    /// No heap of the pool has its context at `heap_gpu_va`
    /// (panthor_heap.c:443-451 for grow, 379-387 for return_chunk, 231-233
    /// for destroy; each `-EINVAL`).
    UnknownHeap { heap_gpu_va: u64 },
    /// More render passes in flight than the heap's `target_in_flight`
    /// (panthor_heap.c:458, `-ENOMEM`: the firmware waits instead).
    InFlightAboveTarget { renderpasses_in_flight: u32, target_in_flight: u32 },
    /// The heap has `max_chunks` chunks (panthor_heap.c:459, `-ENOMEM`).
    MaxChunksReached { chunk_count: u32, max_chunks: u32 },
    /// No chunk of the heap at `chunk_gpu_va`, its low 12 bits dropped
    /// (panthor_heap.c:389, 407-408, `-EINVAL`).
    UnknownChunk { chunk_gpu_va: u64 },
}

/// What the module asks of an allocation: the pool buffer or a chunk (the
/// labels panthor gives them, panthor_heap.c:155, 560).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeapObject {
    /// "Heap pool": [`pool_size`] bytes of heap contexts.
    Pool,
    /// "Tiler heap chunk": `chunk_size` bytes.
    Chunk,
}

/// A buffer mapped in the group's VM and for the CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Allocation {
    /// The CPU address of its first byte.
    pub cpu: usize,
    /// The GPU address of the same byte in the group's VM.
    pub gpu: u64,
    /// Its size in bytes.
    pub size: u64,
}

/// The memory of the heap (module doc, "The interface"): kernel buffers in
/// the group's VM, mapped no-exec, and for the CPU uncached or
/// write-combined (panthor_heap.c:151-163, 556-568; panthor_gem.c:247).
///
/// # Safety
///
/// For every [`Allocation`] `alloc` returns, `size` bytes at `cpu` are
/// writable by this module and are the bytes the GPU sees at `gpu` in the
/// group's VM, with no cache maintenance needed, until the allocation is
/// given back through `free`. Allocations do not overlap.
pub unsafe trait HeapMemory {
    type Error: Copy + Debug + PartialEq + Eq;
    /// A buffer of at least `size` bytes for `what`
    /// (`panthor_kernel_bo_create` and `panthor_kernel_bo_vmap`).
    fn alloc(&mut self, size: u64, what: HeapObject) -> Result<Allocation, Self::Error>;
    /// Give `a` back (`panthor_kernel_bo_destroy`, panthor_heap.c:135, 193).
    fn free(&mut self, a: Allocation);
}

/// Check what `alloc` handed out for `wanted` bytes; give it back when it
/// does not do.
fn take<M: HeapMemory>(mem: &mut M, a: Allocation, wanted: u64) -> Result<Allocation, HeapError<M::Error>> {
    let refusal = if a.size < wanted {
        Some(HeapError::ShortAllocation { size: a.size, wanted })
    } else if a.gpu & (POOL_ALIGN - 1) != 0 {
        Some(HeapError::MisalignedAllocation { gpu: a.gpu })
    } else if a.cpu == 0 {
        Some(HeapError::NoCpuAddress { gpu: a.gpu })
    } else {
        None
    };
    match refusal {
        Some(e) => {
            mem.free(a);
            Err(e)
        }
        None => Ok(a),
    }
}

/// A pool of heap contexts (`struct panthor_heap_pool`, panthor_heap.c:
/// 77-103): one per group VM (panthor_mmu.c:1940-1961).
#[derive(Debug, PartialEq, Eq)]
pub struct HeapPool {
    /// `gpu_contexts` (panthor_heap.c:98-99).
    ctx: Allocation,
    stride: u32,
    /// The slots taken, bit `id` for heap `id` (the `xa` of
    /// panthor_heap.c:95-96, which holds the heaps themselves).
    used: u128,
}

const _: () = assert!(MAX_HEAPS_PER_POOL == u128::BITS);

impl HeapPool {
    /// `panthor_heap_pool_create` (panthor_heap.c:534-577): one buffer of
    /// [`pool_size`] bytes for the GPU whose `GPU_L2_FEATURES` reads
    /// `l2_features`. panthor zeroes nothing here; create zeroes each slot.
    pub fn create<M: HeapMemory>(mem: &mut M, l2_features: u32) -> Result<HeapPool, HeapError<M::Error>> {
        let stride = ctx_stride(l2_features).ok_or(HeapError::L2LineSize { l2_features })?;
        let size = pool_size(stride);
        let a = mem.alloc(size, HeapObject::Pool).map_err(HeapError::PoolAlloc)?;
        let ctx = take(mem, a, size)?;
        Ok(HeapPool { ctx, stride, used: 0 })
    }

    /// The stride of a slot (panthor_heap.c:105-111).
    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// The pool buffer.
    pub fn buffer(&self) -> Allocation {
        self.ctx
    }

    /// Heap `id`'s context address (panthor_heap.c:337-338).
    pub fn ctx_gpu(&self, id: u32) -> u64 {
        self.ctx.gpu + ctx_offset(self.stride, id)
    }

    /// The lowest free slot, as `xa_alloc` gives it (panthor_heap.c:331-332).
    fn free_slot(&self) -> Option<u32> {
        let id = (!self.used).trailing_zeros();
        (id < MAX_HEAPS_PER_POOL).then_some(id)
    }
}

/// A heap (`struct panthor_heap`, panthor_heap.c:49-73), its chunks in the
/// caller's list.
#[derive(Debug, PartialEq, Eq)]
pub struct Heap<'l> {
    /// Its slot in the pool (panthor_heap.c:331, 347).
    id: u32,
    ctx_gpu: u64,
    chunk_size: u32,
    max_chunks: u32,
    target_in_flight: u32,
    /// panthor's `chunks` list (:53-54), oldest first: `chunks[0]` is the
    /// tail of panthor's list, `chunks[chunk_count - 1]` its head.
    chunks: &'l mut [Allocation],
    chunk_count: u32,
}

impl Heap<'_> {
    /// The slot, the `id` `panthor_heap_create` returns (panthor_heap.c:347).
    pub fn id(&self) -> u32 {
        self.id
    }
    /// The heap context's GPU address (`heap_ctx_gpu_va`, panthor_heap.c:
    /// 337-338): `HEAP_SET`'s operand.
    pub fn ctx_gpu(&self) -> u64 {
        self.ctx_gpu
    }
    /// The first chunk's GPU address, the list head (`first_chunk_gpu_va`,
    /// panthor_heap.c:321-324); after [`grow`] the newest chunk. 0 once
    /// [`return_chunk`] has given back every chunk.
    pub fn first_chunk_gpu(&self) -> u64 {
        self.chunks().last().map_or(0, |c| c.gpu)
    }
    pub fn chunk_size(&self) -> u32 {
        self.chunk_size
    }
    pub fn max_chunks(&self) -> u32 {
        self.max_chunks
    }
    pub fn target_in_flight(&self) -> u32 {
        self.target_in_flight
    }
    /// The chunks, oldest first.
    pub fn chunks(&self) -> &[Allocation] {
        &self.chunks[..self.chunk_count as usize]
    }
    /// What `OSAL_QUERY_MALI_TILER_HEAP` answers.
    pub fn osal(&self) -> OsalMaliHeap {
        OsalMaliHeap {
            ctx_gpu: self.ctx_gpu,
            first_chunk_gpu: self.first_chunk_gpu(),
            chunk_size: self.chunk_size,
            reserved: 0,
        }
    }
}

/// `OsalMaliHeap` of src/osal/mali.rs:46-56 (`struct osal_mali_heap`), which
/// this crate cannot import: the same fields at the same offsets, 24 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OsalMaliHeap {
    pub ctx_gpu: u64,
    pub first_chunk_gpu: u64,
    pub chunk_size: u32,
    pub reserved: u32,
}

const _: () = assert!(core::mem::size_of::<OsalMaliHeap>() == 24);

/// `panthor_heap_create`'s checks (panthor_heap.c:287-295), in its order,
/// each its own refusal.
pub fn check_params<E>(p: &HeapParams) -> Result<(), HeapError<E>> {
    if p.initial_chunk_count == 0 {
        return Err(HeapError::ZeroInitialChunks);
    }
    if p.initial_chunk_count > p.max_chunks {
        return Err(HeapError::InitialAboveMax {
            initial_chunk_count: p.initial_chunk_count,
            max_chunks: p.max_chunks,
        });
    }
    if !p.chunk_size.is_multiple_of(CHUNK_ALIGN) {
        return Err(HeapError::ChunkSizeNotPageAligned { chunk_size: p.chunk_size });
    }
    if p.chunk_size < CHUNK_MIN_SIZE {
        return Err(HeapError::ChunkSizeBelowMin { chunk_size: p.chunk_size });
    }
    if p.chunk_size > CHUNK_MAX_SIZE {
        return Err(HeapError::ChunkSizeAboveMax { chunk_size: p.chunk_size });
    }
    Ok(())
}

/// Give back `chunks`, head first (`panthor_free_heap_chunks`,
/// panthor_heap.c:201-208, walks the list from its head).
fn free_chunks<M: HeapMemory>(mem: &mut M, chunks: &[Allocation]) {
    for &c in chunks.iter().rev() {
        mem.free(c);
    }
}

/// `panthor_heap_create` (panthor_heap.c:273-357): check `p`, allocate the
/// initial chunks and link each to the one before it (`panthor_alloc_heap_chunk`,
/// :139-199), take the pool's lowest free slot and zero it (:331-338).
/// `list` holds the chunks, at least `p.max_chunks` entries. On a refusal
/// after the first allocation every chunk is given back (:343-350).
pub fn create<'l, M: HeapMemory>(
    pool: &mut HeapPool,
    mem: &mut M,
    p: &HeapParams,
    list: &'l mut [Allocation],
) -> Result<Heap<'l>, HeapError<M::Error>> {
    check_params(p)?;
    if list.len() < p.max_chunks as usize {
        return Err(HeapError::ChunkListTooShort { len: list.len(), max_chunks: p.max_chunks });
    }
    let size = u64::from(p.chunk_size);
    // panthor_alloc_heap_chunks (:210-224): one at a time, each linked to
    // the list head before it becomes the head (:168-179, 184)
    for i in 0..p.initial_chunk_count as usize {
        let a = match mem.alloc(size, HeapObject::Chunk) {
            Ok(a) => a,
            Err(err) => {
                free_chunks(mem, &list[..i]);
                return Err(HeapError::ChunkAlloc { index: i as u32, err });
            }
        };
        let a = match take(mem, a, size) {
            Ok(a) => a,
            Err(e) => {
                free_chunks(mem, &list[..i]);
                return Err(e);
            }
        };
        let next = if i == 0 { 0 } else { chunk_link(list[i - 1].gpu, p.chunk_size) };
        let hdr = chunk_header(next);
        // SAFETY: `a` came from `mem.alloc`, checked to hold at least
        // `chunk_size` (>= CHUNK_MIN_SIZE > CHUNK_HEADER_SIZE) bytes at a
        // non-null `cpu` this module may write (HeapMemory's contract).
        unsafe { core::ptr::copy_nonoverlapping(hdr.as_ptr(), a.cpu as *mut u8, CHUNK_HEADER_SIZE) };
        list[i] = a;
    }
    let count = p.initial_chunk_count;
    let Some(id) = pool.free_slot() else {
        free_chunks(mem, &list[..count as usize]);
        return Err(HeapError::PoolFull { heaps: MAX_HEAPS_PER_POOL });
    };
    let off = ctx_offset(pool.stride, id);
    // SAFETY: the pool buffer holds pool_size(stride) bytes (checked in
    // HeapPool::create), and slot `id` < MAX_HEAPS_PER_POOL ends within it.
    unsafe { core::ptr::write_bytes((pool.ctx.cpu + off as usize) as *mut u8, 0, pool.stride as usize) };
    pool.used |= 1 << id;
    Ok(Heap {
        id,
        ctx_gpu: pool.ctx_gpu(id),
        chunk_size: p.chunk_size,
        max_chunks: p.max_chunks,
        target_in_flight: p.target_in_flight,
        chunks: list,
        chunk_count: count,
    })
}

/// The heap whose context is at `heap_gpu_va`, as `panthor_heap_grow` and
/// `panthor_heap_return_chunk` find it (panthor_heap.c:437-451, 373-387):
/// the offset from the pool buffer, `(u32)offset / stride` the id, `-EINVAL`
/// for an offset above `U32_MAX` (a VA below the pool wraps to one), an id
/// of `MAX_HEAPS_PER_POOL` or more, or a slot `xa_load` finds empty. An
/// address inside a slot names that slot, as panthor's division does.
/// panthor's `xa` holds the heaps; here the caller does, in `heaps`, and the
/// one found is the heap of that slot made on this pool.
fn find_heap<'h, 'l, E>(
    pool: &HeapPool,
    heaps: &'h mut [Heap<'l>],
    heap_gpu_va: u64,
) -> Result<&'h mut Heap<'l>, HeapError<E>> {
    let unknown = HeapError::UnknownHeap { heap_gpu_va };
    let offset = heap_gpu_va.wrapping_sub(pool.ctx.gpu);
    let id = offset as u32 / pool.stride;
    if offset > u64::from(u32::MAX) || id >= MAX_HEAPS_PER_POOL || pool.used & (1 << id) == 0 {
        return Err(unknown);
    }
    let ctx_gpu = pool.ctx_gpu(id);
    heaps.iter_mut().find(|h| h.id == id && h.ctx_gpu == ctx_gpu).ok_or(unknown)
}

/// `panthor_heap_grow` (panthor_heap.c:416-489): one more chunk for the heap
/// whose context is at `heap_gpu_va`, the `heap_address` a stream's
/// `CS_TILER_OOM` reports. For a later `CS_TILER_OOM` handler in sched.rs
/// (`group_process_tiler_oom`, panthor_sched.c:1448-1532); nothing calls it
/// yet: Z4's sched.rs refuses `CS_TILER_OOM`.
///
/// `renderpasses_in_flight` is the stream's `heap_vt_start - heap_frag_end`
/// and `pending_frag_count` its `heap_vt_end - heap_frag_end`
/// (panthor_sched.c:1469-1470); panthor takes the second and reads it
/// nowhere, nor does this. The refusals, in panthor's order: the heap is
/// unknown (`-EINVAL`, :443-451); more render passes in flight than
/// `target_in_flight` (strictly more, :458) or `max_chunks` chunks already
/// (:459), both `-ENOMEM`, which panthor's handler answers by acknowledging
/// with no chunk so the firmware waits for fragment jobs or runs the
/// exception handler (panthor_sched.c:1491-1501, 1512-1514); the chunk's
/// allocation failing (:475-477). After a refusal the heap is as it was.
///
/// Otherwise the new chunk is the list head (`list_add`, :184) and the
/// value is the one panthor writes to `*new_chunk_gpu_va`:
/// `(va & GENMASK_ULL(63, 12)) | (chunk_size >> 12)` (:482-483), which the
/// handler writes to the stream's `heap_start` and `heap_end`
/// (panthor_sched.c:1512-1513). The chunk's header is zeroed and not linked
/// to the old head (`initial_chunk` is false, :168, 475): the firmware
/// links a grown chunk itself; only create's chunks are chained by the
/// kernel side.
pub fn grow<M: HeapMemory>(
    pool: &HeapPool,
    mem: &mut M,
    heaps: &mut [Heap<'_>],
    heap_gpu_va: u64,
    renderpasses_in_flight: u32,
    pending_frag_count: u32,
) -> Result<u64, HeapError<M::Error>> {
    let _ = pending_frag_count;
    let heap = find_heap(pool, heaps, heap_gpu_va)?;
    if renderpasses_in_flight > heap.target_in_flight {
        return Err(HeapError::InFlightAboveTarget { renderpasses_in_flight, target_in_flight: heap.target_in_flight });
    }
    if heap.chunk_count >= heap.max_chunks {
        return Err(HeapError::MaxChunksReached { chunk_count: heap.chunk_count, max_chunks: heap.max_chunks });
    }
    // panthor_alloc_heap_chunk(pool, heap, false) (:139-199, 475)
    let size = u64::from(heap.chunk_size);
    let a = mem.alloc(size, HeapObject::Chunk).map_err(|err| HeapError::ChunkAlloc { index: heap.chunk_count, err })?;
    let a = take(mem, a, size)?;
    let hdr = chunk_header(0);
    // SAFETY: as in create: `a` holds at least chunk_size (> CHUNK_HEADER_SIZE)
    // bytes at a non-null `cpu` this module may write.
    unsafe { core::ptr::copy_nonoverlapping(hdr.as_ptr(), a.cpu as *mut u8, CHUNK_HEADER_SIZE) };
    // chunk_count < max_chunks <= chunks.len() (create's ChunkListTooShort)
    heap.chunks[heap.chunk_count as usize] = a;
    heap.chunk_count += 1;
    Ok(chunk_link(a.gpu, heap.chunk_size))
}

/// `panthor_heap_return_chunk` (panthor_heap.c:359-414): give back the
/// chunk at `chunk_gpu_va` of the heap whose context is at `heap_gpu_va`,
/// the chunk [`grow`] made that the handler could not hand to the stream
/// because the group was scheduled out meanwhile (:365-367;
/// panthor_sched.c:1520-1525). For the same later `CS_TILER_OOM` handler;
/// nothing calls it yet.
///
/// The heap is found as [`grow`] finds it (`-EINVAL`, :373-387), the low 12
/// bits of `chunk_gpu_va` are dropped (:389), so [`grow`]'s value names its
/// chunk, and the first chunk from the head at that address is taken out of
/// the list, `chunk_count` one less (:392-400), and given back (:403-405).
/// No chunk there: `-EINVAL` (:407-408). As in panthor, nothing else is
/// checked or rewritten: a chunk of the chain create linked can be given
/// back too, and the header of the chunk that pointed at it is left as it
/// is; the last chunk can go, leaving the list empty.
pub fn return_chunk<M: HeapMemory>(
    pool: &HeapPool,
    mem: &mut M,
    heaps: &mut [Heap<'_>],
    heap_gpu_va: u64,
    chunk_gpu_va: u64,
) -> Result<(), HeapError<M::Error>> {
    let heap = find_heap(pool, heaps, heap_gpu_va)?;
    let chunk_gpu_va = chunk_gpu_va & CHUNK_VA_MASK;
    let n = heap.chunk_count as usize;
    // panthor walks from the head, the newest: here from the end
    let Some(i) = heap.chunks[..n].iter().rposition(|c| c.gpu == chunk_gpu_va) else {
        return Err(HeapError::UnknownChunk { chunk_gpu_va });
    };
    let removed = heap.chunks[i];
    // list_del (:395): the newer chunks move down one
    heap.chunks.copy_within(i + 1..n, i);
    heap.chunks[n - 1] = Allocation::default();
    heap.chunk_count -= 1;
    mem.free(removed);
    Ok(())
}

/// `panthor_heap_destroy` (panthor_heap.c:226-255): free the heap's slot
/// (`xa_erase`, :231) and give back its chunks, head first
/// (`panthor_free_heap_chunks`, :235; :201-208). The slot is not zeroed
/// (panthor does not; create zeroes it again, :336). A heap that is not
/// this pool's, panthor's `-EINVAL` for a handle `xa_erase` does not find
/// (:232-233), is refused and handed back untouched with the error. On
/// success the caller's chunk list is handed back, every entry given back
/// through `mem`.
pub fn destroy<'l, M: HeapMemory>(
    pool: &mut HeapPool,
    mem: &mut M,
    heap: Heap<'l>,
) -> Result<&'l mut [Allocation], (HeapError<M::Error>, Heap<'l>)> {
    if heap.id >= MAX_HEAPS_PER_POOL || pool.used & (1 << heap.id) == 0 || pool.ctx_gpu(heap.id) != heap.ctx_gpu {
        return Err((HeapError::UnknownHeap { heap_gpu_va: heap.ctx_gpu }, heap));
    }
    pool.used &= !(1 << heap.id);
    let n = heap.chunk_count as usize;
    free_chunks(mem, &heap.chunks[..n]);
    heap.chunks[..n].fill(Allocation::default());
    Ok(heap.chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    /// The board's `L2_FEATURES` (super's module doc): line exponent 6.
    const T6_L2: u32 = 0x0712_0306;

    /// Where the trace's heap context lay: the pool's address (heap 0).
    const POOL_VA: u64 = 0x8000_0001_5000;
    /// The fake's chunks start at the next 2 MiB boundary above the pool.
    const CHUNK_VA: u64 = 0x8000_0020_0000;

    /// Memory for the tests: one buffer filled with 0xaa, handed out in
    /// order, the pool at [`POOL_VA`], chunks from [`CHUNK_VA`] on, each at
    /// a multiple of its size (a 2 MiB chunk 2 MiB aligned, as
    /// panthor_mmu.c:1062 picks).
    struct Fake {
        base: *mut u8,
        len: usize,
        used: usize,
        next_chunk_va: u64,
        /// The CPU offset of each GPU address handed out.
        map: Vec<(u64, usize, u64)>,
        allocs: Vec<Allocation>,
        freed: Vec<Allocation>,
        /// Fail the n-th chunk allocation (0 the first).
        fail_chunk: Option<usize>,
        chunks_asked: usize,
        /// Hand out chunks 16 bytes past a page / 4 KiB short / with no CPU address.
        misalign: bool,
        short: bool,
        no_cpu: bool,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct OutOfMemory;

    impl Fake {
        fn new(len: usize) -> Fake {
            let b = vec![0xaau8; len].into_boxed_slice();
            Fake {
                base: Box::into_raw(b) as *mut u8,
                len,
                used: 0,
                next_chunk_va: CHUNK_VA,
                map: vec![],
                allocs: vec![],
                freed: vec![],
                fail_chunk: None,
                chunks_asked: 0,
                misalign: false,
                short: false,
                no_cpu: false,
            }
        }
        /// `n` bytes at GPU address `va`, read through the CPU side.
        fn read(&self, va: u64, n: usize) -> Vec<u8> {
            let &(g, off, size) = self.map.iter().find(|m| m.0 <= va && va < m.0 + m.2).expect("mapped");
            assert!(va + n as u64 <= g + size);
            let at = off + (va - g) as usize;
            // SAFETY: within the buffer (checked against the allocation)
            unsafe { std::slice::from_raw_parts(self.base.add(at), n).to_vec() }
        }
        fn word(&self, va: u64) -> u64 {
            u64::from_le_bytes(self.read(va, 8).try_into().unwrap())
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            // SAFETY: the buffer Fake::new leaked, of `len` bytes
            drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.base, self.len)) });
        }
    }

    // SAFETY: allocations are disjoint ranges of the one buffer, which lives
    // as long as the Fake.
    unsafe impl HeapMemory for Fake {
        type Error = OutOfMemory;
        fn alloc(&mut self, size: u64, what: HeapObject) -> Result<Allocation, OutOfMemory> {
            let gpu = match what {
                HeapObject::Pool => POOL_VA,
                HeapObject::Chunk => {
                    let n = self.chunks_asked;
                    self.chunks_asked += 1;
                    if self.fail_chunk == Some(n) {
                        return Err(OutOfMemory);
                    }
                    let va = self.next_chunk_va.next_multiple_of(size);
                    self.next_chunk_va = va + size;
                    va
                }
            };
            if self.used + size as usize > self.len {
                return Err(OutOfMemory);
            }
            let off = self.used;
            self.used += size as usize;
            self.map.push((gpu, off, size));
            let mut a = Allocation { cpu: self.base as usize + off, gpu, size };
            if what == HeapObject::Chunk {
                if self.misalign {
                    a.gpu += 16;
                }
                if self.short {
                    a.size -= 4096;
                }
                if self.no_cpu {
                    a.cpu = 0;
                }
            }
            self.allocs.push(a);
            Ok(a)
        }
        fn free(&mut self, a: Allocation) {
            self.freed.push(a);
        }
    }

    type E = HeapError<OutOfMemory>;

    fn params(initial: u32, chunk_size: u32, max: u32) -> HeapParams {
        HeapParams { initial_chunk_count: initial, chunk_size, max_chunks: max, target_in_flight: 65535 }
    }

    #[test]
    fn the_stride_and_slots_are_panthor_s_for_the_t6_and_other_lines() {
        // panthor_heap.c:105-116, 537-539 by hand:
        // T6: L2_FEATURES & 0xff = 6, line 1 << 6 = 64, ALIGN(32, 64) = 64;
        // slot id at 64 * id; pool ALIGN(128 * 64, 4096) = 8192
        assert_eq!(l2_line_size(T6_L2), Some(64));
        assert_eq!(ctx_stride(T6_L2), Some(64));
        assert_eq!([ctx_offset(64, 0), ctx_offset(64, 1), ctx_offset(64, 127)], [0, 64, 8128]);
        assert_eq!(pool_size(64), 8192);
        // a 128-byte line (exponent 7): stride 128, slot 127 at 16256,
        // pool 128 * 128 = 16384
        assert_eq!(ctx_stride(0x07), Some(128));
        assert_eq!(ctx_offset(128, 127), 16256);
        assert_eq!(pool_size(128), 16384);
        // a 16-byte line (exponent 4): ALIGN(32, 16) = 32; pool 128 * 32 =
        // 4096
        assert_eq!(ctx_stride(0x04), Some(32));
        assert_eq!(ctx_offset(32, 3), 96);
        assert_eq!(pool_size(32), 4096);
        // a 1-byte line (exponent 0): still 32
        assert_eq!(ctx_stride(0), Some(32));
        // 1 << 31 is the largest; 1 << 32 is no size
        assert_eq!(ctx_stride(31), Some(1 << 31));
        assert_eq!(pool_size(1 << 31), 1 << 38);
        assert_eq!(ctx_stride(32), None);
        assert_eq!(ctx_stride(0xff), None);
    }

    #[test]
    fn the_chunk_header_is_panthor_s_next_word_then_zeroes() {
        // chunk A at 0x0000_8000_0040_0000, 2 MiB; the chunk after it in the
        // chain points at A: next = (0x8000_0040_0000 & GENMASK_ULL(63, 12))
        // | (0x20_0000 >> 12) = 0x8000_0040_0000 | 0x200 = 0x8000_0040_0200
        // (panthor_heap.c:177-178), little-endian:
        // 00 02 40 00 00 80 00 00, then fourteen zero words (:34)
        let next = chunk_link(0x8000_0040_0000, 2 << 20);
        assert_eq!(next, 0x0000_8000_0040_0200);
        let mut want = [0u8; 64];
        want[..8].copy_from_slice(&[0x00, 0x02, 0x40, 0x00, 0x00, 0x80, 0x00, 0x00]);
        assert_eq!(chunk_header(next), want);
        // 128 KiB at 0x1_2345_6000: 0x1_2345_6000 | (0x2_0000 >> 12 = 0x20)
        // = 0x1_2345_6020: 20 60 45 23 01 00 00 00
        let next = chunk_link(0x1_2345_6000, 128 << 10);
        assert_eq!(next, 0x1_2345_6020);
        assert_eq!(chunk_header(next)[..8], [0x20, 0x60, 0x45, 0x23, 0x01, 0, 0, 0]);
        // 8 MiB: 0x80_0000 >> 12 = 0x800, still within the 12 low bits
        assert_eq!(chunk_link(0x8000_0080_0000, 8 << 20), 0x8000_0080_0800);
        // panthor's mask drops the low 12 bits of the VA as written
        assert_eq!(chunk_link(0x8000_0040_0abc, 2 << 20), 0x8000_0040_0200);
        assert_eq!(chunk_header(0), [0u8; 64]);
        assert_eq!(CHUNK_HEADER_SIZE, 64);
    }

    #[test]
    fn create_gives_the_trace_s_context_and_a_chain_from_the_newest_chunk() {
        let mut mem = Fake::new(16 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        assert_eq!(pool.stride(), 64);
        assert_eq!(pool.buffer(), Allocation { cpu: mem.base as usize, gpu: POOL_VA, size: 8192 });
        let mut list = [Allocation::default(); 64];
        let p = HeapParams::MESA;
        let heap = create(&mut pool, &mut mem, &p, &mut list).unwrap();
        assert_eq!(heap.id(), 0);
        assert_eq!(heap.ctx_gpu(), 0x8000_0001_5000);
        // five 2 MiB chunks at 0x8000_0020_0000, 0x8000_0040_0000, ...; the
        // newest, the fifth, is the first chunk (panthor_heap.c:184, 321-324)
        let vas: Vec<u64> = heap.chunks().iter().map(|c| c.gpu).collect();
        assert_eq!(vas, [0x8000_0020_0000, 0x8000_0040_0000, 0x8000_0060_0000, 0x8000_0080_0000, 0x8000_00a0_0000]);
        assert_eq!(heap.first_chunk_gpu(), 0x8000_00a0_0000);
        // the chain through the headers: each next is the older chunk with
        // 0x200 (2 MiB >> 12) in its low bits; the oldest's next is 0
        let mut walked = vec![];
        let mut va = heap.first_chunk_gpu();
        loop {
            walked.push(va);
            let hdr = mem.read(va, 64);
            assert!(hdr[8..].iter().all(|&b| b == 0), "unknown[] is MBZ");
            let next = mem.word(va);
            if next == 0 {
                break;
            }
            assert_eq!(next & 0xfff, 0x200);
            va = next & CHUNK_VA_MASK;
        }
        assert_eq!(walked, [0x8000_00a0_0000, 0x8000_0080_0000, 0x8000_0060_0000, 0x8000_0040_0000, 0x8000_0020_0000]);
        assert_eq!(mem.word(0x8000_00a0_0000), 0x8000_0080_0200);
        // only the 64-byte header is written: the chunk's body is as found
        assert_eq!(mem.read(0x8000_00a0_0000 + 64, 8), [0xaa; 8]);
        // the context slot is zeroed over the stride, the next slot is not
        assert_eq!(mem.read(POOL_VA, 64), [0u8; 64]);
        assert_eq!(mem.read(POOL_VA + 64, 8), [0xaa; 8]);
        assert!(mem.freed.is_empty());
        // OSAL_QUERY_MALI_TILER_HEAP's three fields
        assert_eq!(
            heap.osal(),
            OsalMaliHeap {
                ctx_gpu: 0x8000_0001_5000,
                first_chunk_gpu: 0x8000_00a0_0000,
                chunk_size: 2 << 20,
                reserved: 0
            }
        );
        assert_eq!((heap.max_chunks(), heap.target_in_flight(), heap.chunk_size()), (64, 65535, 2 << 20));
    }

    #[test]
    fn the_osal_shape_is_src_osal_s() {
        // src/osal/mali.rs:51-56: u64 @0, u64 @8, u32 @16, u32 @20; 24 bytes
        assert_eq!(core::mem::size_of::<OsalMaliHeap>(), 24);
        assert_eq!(core::mem::align_of::<OsalMaliHeap>(), 8);
        assert_eq!(
            [
                offset_of!(OsalMaliHeap, ctx_gpu),
                offset_of!(OsalMaliHeap, first_chunk_gpu),
                offset_of!(OsalMaliHeap, chunk_size),
                offset_of!(OsalMaliHeap, reserved)
            ],
            [0, 8, 16, 20]
        );
    }

    #[test]
    fn a_second_heap_takes_the_next_slot_and_one_chunk_has_no_link() {
        let mut mem = Fake::new(2 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let (mut l0, mut l1) = ([Allocation::default(); 2], [Allocation::default(); 2]);
        let h0 = create(&mut pool, &mut mem, &params(1, 128 << 10, 2), &mut l0).unwrap();
        let h1 = create(&mut pool, &mut mem, &params(2, 128 << 10, 2), &mut l1).unwrap();
        assert_eq!((h0.id(), h0.ctx_gpu()), (0, POOL_VA));
        assert_eq!((h1.id(), h1.ctx_gpu()), (1, POOL_VA + 64));
        assert_eq!(mem.word(h0.first_chunk_gpu()), 0);
        assert_eq!(mem.read(POOL_VA + 64, 64), [0u8; 64]);
        assert_eq!(mem.read(POOL_VA + 128, 8), [0xaa; 8]);
        // 128 KiB chunks: next = older VA | 0x20
        assert_eq!(mem.word(h1.first_chunk_gpu()), h1.chunks()[0].gpu | 0x20);
    }

    #[test]
    fn panthor_s_checks_refuse_in_its_order_with_the_value() {
        let r = |p: HeapParams| check_params::<OutOfMemory>(&p);
        // 128 KiB + 1: not a multiple of 4 KiB (nor a power of two)
        assert_eq!(r(params(1, (128 << 10) + 1, 1)), Err(E::ChunkSizeNotPageAligned { chunk_size: 0x2_0001 }));
        assert_eq!(r(params(1, (3 << 20) | 0x800, 1)), Err(E::ChunkSizeNotPageAligned { chunk_size: 0x30_0800 }));
        // 64 KiB: aligned, below the minimum; 124 KiB too
        assert_eq!(r(params(1, 64 << 10, 1)), Err(E::ChunkSizeBelowMin { chunk_size: 0x1_0000 }));
        assert_eq!(r(params(1, 124 << 10, 1)), Err(E::ChunkSizeBelowMin { chunk_size: 0x1_f000 }));
        // 16 MiB and 8 MiB + 4 KiB: above
        assert_eq!(r(params(1, 16 << 20, 1)), Err(E::ChunkSizeAboveMax { chunk_size: 0x100_0000 }));
        assert_eq!(r(params(1, (8 << 20) + 4096, 1)), Err(E::ChunkSizeAboveMax { chunk_size: 0x80_1000 }));
        assert_eq!(r(params(0, 2 << 20, 64)), Err(E::ZeroInitialChunks));
        assert_eq!(r(params(6, 2 << 20, 5)), Err(E::InitialAboveMax { initial_chunk_count: 6, max_chunks: 5 }));
        // panthor's order (:287-295): the count before the size
        assert_eq!(r(params(0, 1, 0)), Err(E::ZeroInitialChunks));
        assert_eq!(r(params(2, 1, 1)), Err(E::InitialAboveMax { initial_chunk_count: 2, max_chunks: 1 }));
        // the bounds themselves, and a size panthor takes that is not a
        // power of two (132 KiB): its rule, not the plan's
        assert_eq!(r(params(1, 128 << 10, 1)), Ok(()));
        assert_eq!(r(params(1, 8 << 20, 1)), Ok(()));
        assert_eq!(r(params(1, 132 << 10, 1)), Ok(()));
        // target_in_flight is not checked (:315)
        assert_eq!(r(HeapParams { target_in_flight: 0, ..HeapParams::MESA }), Ok(()));
    }

    #[test]
    fn a_refused_create_allocates_nothing() {
        let mut mem = Fake::new(1 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let mut list = [Allocation::default(); 4];
        for (p, e) in [
            (params(0, 128 << 10, 4), E::ZeroInitialChunks),
            (params(5, 128 << 10, 4), E::InitialAboveMax { initial_chunk_count: 5, max_chunks: 4 }),
            (params(1, 100_000, 4), E::ChunkSizeNotPageAligned { chunk_size: 100_000 }),
            (params(1, 64 << 10, 4), E::ChunkSizeBelowMin { chunk_size: 64 << 10 }),
            (params(1, 16 << 20, 4), E::ChunkSizeAboveMax { chunk_size: 16 << 20 }),
            (params(1, 128 << 10, 5), E::ChunkListTooShort { len: 4, max_chunks: 5 }),
        ] {
            assert_eq!(create(&mut pool, &mut mem, &p, &mut list), Err(e));
        }
        assert_eq!(mem.chunks_asked, 0);
        assert_eq!(pool.used, 0);
    }

    #[test]
    fn an_allocator_failing_on_the_third_chunk_gets_the_first_two_back() {
        let mut mem = Fake::new(4 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        mem.fail_chunk = Some(2);
        let mut list = [Allocation::default(); 5];
        assert_eq!(
            create(&mut pool, &mut mem, &params(5, 128 << 10, 5), &mut list),
            Err(E::ChunkAlloc { index: 2, err: OutOfMemory })
        );
        // given back head first (panthor_heap.c:206-207): the second, then the first
        let chunks = &mem.allocs[1..];
        assert_eq!(chunks.len(), 2);
        assert_eq!(mem.freed, [chunks[1], chunks[0]]);
        // no slot taken: the next heap is heap 0
        assert_eq!(pool.used, 0);
        mem.fail_chunk = None;
        let h = create(&mut pool, &mut mem, &params(1, 128 << 10, 5), &mut list).unwrap();
        assert_eq!(h.id(), 0);
    }

    #[test]
    fn an_allocator_failing_on_the_first_chunk_frees_nothing() {
        let mut mem = Fake::new(1 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        mem.fail_chunk = Some(0);
        let mut list = [Allocation::default(); 1];
        assert_eq!(
            create(&mut pool, &mut mem, &params(1, 128 << 10, 1), &mut list),
            Err(E::ChunkAlloc { index: 0, err: OutOfMemory })
        );
        assert!(mem.freed.is_empty());
    }

    #[test]
    fn a_full_pool_refuses_after_the_chunks_and_gives_them_back() {
        let mut mem = Fake::new(1 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        pool.used = u128::MAX;
        let mut list = [Allocation::default(); 2];
        assert_eq!(create(&mut pool, &mut mem, &params(2, 128 << 10, 2), &mut list), Err(E::PoolFull { heaps: 128 }));
        let chunks = &mem.allocs[1..];
        assert_eq!(mem.freed, [chunks[1], chunks[0]]);
        // slot 127 alone free: it is taken, at 127 * 64
        pool.used = !(1 << 127);
        let h = create(&mut pool, &mut mem, &params(1, 128 << 10, 2), &mut list).unwrap();
        assert_eq!((h.id(), h.ctx_gpu()), (127, POOL_VA + 8128));
    }

    #[test]
    fn what_the_allocator_hands_out_is_checked_and_given_back() {
        for (set, want) in [
            (0, E::MisalignedAllocation { gpu: CHUNK_VA + (128 << 10) + 16 }),
            (1, E::ShortAllocation { size: (128 << 10) - 4096, wanted: 128 << 10 }),
            (2, E::NoCpuAddress { gpu: CHUNK_VA + (128 << 10) }),
        ] {
            let mut mem = Fake::new(1 << 20);
            let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
            let mut list = [Allocation::default(); 2];
            // the first chunk is good, the second is not
            let mut l1 = [Allocation::default(); 1];
            let first = create(&mut pool, &mut mem, &params(1, 128 << 10, 1), &mut l1).unwrap().chunks()[0];
            assert_eq!(first.gpu, CHUNK_VA);
            match set {
                0 => mem.misalign = true,
                1 => mem.short = true,
                _ => mem.no_cpu = true,
            }
            assert_eq!(create(&mut pool, &mut mem, &params(2, 128 << 10, 2), &mut list), Err(want));
            // the bad one itself is given back
            assert_eq!(mem.freed, [*mem.allocs.last().unwrap()]);
            assert_eq!(pool.used, 1);
        }
    }

    #[test]
    fn the_pool_refuses_a_line_size_that_is_no_size_and_a_bad_buffer() {
        let mut mem = Fake::new(1 << 20);
        assert_eq!(HeapPool::create(&mut mem, 0x0712_0320), Err(E::L2LineSize { l2_features: 0x0712_0320 }));
        assert!(mem.allocs.is_empty());
        let mut mem = Fake::new(4096);
        assert_eq!(HeapPool::create(&mut mem, T6_L2), Err(E::PoolAlloc(OutOfMemory)));
        // 128-byte line on this fake: 16 KiB asked
        let mut mem = Fake::new(1 << 20);
        let pool = HeapPool::create(&mut mem, 0x07).unwrap();
        assert_eq!(pool.buffer().size, 16384);
        assert_eq!(pool.ctx_gpu(2), POOL_VA + 256);
    }

    /// The chunk VAs from the list head along the headers' `next`.
    fn walk(mem: &Fake, head: u64) -> Vec<u64> {
        let mut walked = vec![];
        let mut va = head;
        while va != 0 {
            walked.push(va);
            va = mem.word(va) & CHUNK_VA_MASK;
        }
        walked
    }

    #[test]
    fn grow_puts_a_zeroed_chunk_at_the_head_and_returns_panthor_s_word() {
        let mut mem = Fake::new(16 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let mut list = [Allocation::default(); 64];
        let mut heaps = [create(&mut pool, &mut mem, &HeapParams::MESA, &mut list).unwrap()];
        let old: Vec<Allocation> = heaps[0].chunks().to_vec();
        // the sixth 2 MiB chunk, at 0x8000_00c0_0000: panthor_heap.c:482-483
        // gives (0x8000_00c0_0000 & GENMASK_ULL(63, 12)) | (0x20_0000 >> 12)
        // = 0x8000_00c0_0200, little-endian 00 02 c0 00 00 80 00 00
        let v = grow(&pool, &mut mem, &mut heaps, 0x8000_0001_5000, 1, 0).unwrap();
        assert_eq!(v, 0x0000_8000_00c0_0200);
        assert_eq!(v.to_le_bytes(), [0x00, 0x02, 0xc0, 0x00, 0x00, 0x80, 0x00, 0x00]);
        let h = &heaps[0];
        // the new chunk is the list head (:184): last of ours, the first chunk
        assert_eq!(h.chunks().len(), 6);
        assert_eq!(h.chunks()[..5], old[..]);
        assert_eq!(h.chunks()[5].gpu, 0x8000_00c0_0000);
        assert_eq!(h.first_chunk_gpu(), 0x8000_00c0_0000);
        assert_eq!(h.osal().first_chunk_gpu, 0x8000_00c0_0000);
        // its header is zeroed, not linked to the old head (:166, 168): the
        // firmware links it; its body is as found
        assert_eq!(mem.read(0x8000_00c0_0000, 64), [0u8; 64]);
        assert_eq!(mem.read(0x8000_00c0_0000 + 64, 8), [0xaa; 8]);
        // create's chain from the old head is untouched
        assert_eq!(mem.word(0x8000_00a0_0000), 0x8000_0080_0200);
        assert_eq!(
            walk(&mem, 0x8000_00a0_0000),
            [0x8000_00a0_0000, 0x8000_0080_0000, 0x8000_0060_0000, 0x8000_0040_0000, 0x8000_0020_0000]
        );
        // an address inside heap 0's slot names heap 0 ((u32)offset / stride,
        // :437-438): the seventh chunk
        assert_eq!(grow(&pool, &mut mem, &mut heaps, 0x8000_0001_5000 + 8, 1, 0), Ok(0x8000_00e0_0200));
        assert_eq!(heaps[0].chunks().len(), 7);
        assert!(mem.freed.is_empty());
    }

    #[test]
    fn grow_refuses_at_max_chunks_and_allocates_nothing() {
        let mut mem = Fake::new(2 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let mut list = [Allocation::default(); 2];
        let mut heaps = [create(&mut pool, &mut mem, &params(1, 128 << 10, 2), &mut list).unwrap()];
        // 128 KiB: the value carries 0x20
        assert_eq!(grow(&pool, &mut mem, &mut heaps, POOL_VA, 0, 0), Ok((CHUNK_VA + (128 << 10)) | 0x20));
        let asked = mem.chunks_asked;
        // chunk_count >= max_chunks (:459): -ENOMEM
        assert_eq!(
            grow(&pool, &mut mem, &mut heaps, POOL_VA, 0, 0),
            Err(E::MaxChunksReached { chunk_count: 2, max_chunks: 2 })
        );
        assert_eq!(mem.chunks_asked, asked);
        assert_eq!(heaps[0].chunks().len(), 2);
    }

    #[test]
    fn grow_refuses_only_above_target_in_flight() {
        let mut mem = Fake::new(2 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let mut list = [Allocation::default(); 4];
        let p = HeapParams { target_in_flight: 3, ..params(1, 128 << 10, 4) };
        let mut heaps = [create(&mut pool, &mut mem, &p, &mut list).unwrap()];
        // renderpasses_in_flight > target_in_flight (:458): 4 is refused
        // with -ENOMEM, pending_frag_count is not read
        assert_eq!(
            grow(&pool, &mut mem, &mut heaps, POOL_VA, 4, 0),
            Err(E::InFlightAboveTarget { renderpasses_in_flight: 4, target_in_flight: 3 })
        );
        assert_eq!(mem.chunks_asked, 1);
        // at the target, panthor grows
        assert!(grow(&pool, &mut mem, &mut heaps, POOL_VA, 3, u32::MAX).is_ok());
        assert_eq!(heaps[0].chunks().len(), 2);
        // both reasons at once: the in-flight one is panthor's first operand
        let mut l2 = [Allocation::default(); 1];
        let q = HeapParams { target_in_flight: 0, ..params(1, 128 << 10, 1) };
        let mut h2 = [create(&mut pool, &mut mem, &q, &mut l2).unwrap()];
        assert_eq!(
            grow(&pool, &mut mem, &mut h2, POOL_VA + 64, 1, 0),
            Err(E::InFlightAboveTarget { renderpasses_in_flight: 1, target_in_flight: 0 })
        );
    }

    #[test]
    fn grow_with_a_failing_allocator_leaves_the_heap_as_it_was() {
        let mut mem = Fake::new(4 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let mut list = [Allocation::default(); 8];
        let mut heaps = [create(&mut pool, &mut mem, &params(3, 128 << 10, 8), &mut list).unwrap()];
        let before: Vec<Allocation> = heaps[0].chunks().to_vec();
        let osal = heaps[0].osal();
        let headers: Vec<Vec<u8>> = before.iter().map(|c| mem.read(c.gpu, 64)).collect();
        // the allocation fails (:475-477): the index is the chunk it would be
        mem.fail_chunk = Some(3);
        assert_eq!(grow(&pool, &mut mem, &mut heaps, POOL_VA, 0, 0), Err(E::ChunkAlloc { index: 3, err: OutOfMemory }));
        // the allocation is handed out but not usable: given back
        mem.fail_chunk = None;
        mem.misalign = true;
        assert_eq!(
            grow(&pool, &mut mem, &mut heaps, POOL_VA, 0, 0),
            Err(E::MisalignedAllocation { gpu: CHUNK_VA + 3 * (128 << 10) + 16 })
        );
        assert_eq!(mem.freed, [*mem.allocs.last().unwrap()]);
        assert_eq!(heaps[0].chunks(), &before[..]);
        assert_eq!(heaps[0].osal(), osal);
        assert_eq!(before.iter().map(|c| mem.read(c.gpu, 64)).collect::<Vec<_>>(), headers);
        // and it grows once the allocator does
        mem.misalign = false;
        assert!(grow(&pool, &mut mem, &mut heaps, POOL_VA, 0, 0).is_ok());
        assert_eq!(heaps[0].chunks()[..3], before[..]);
    }

    #[test]
    fn grow_and_return_refuse_an_unknown_heap() {
        let mut mem = Fake::new(2 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let mut list = [Allocation::default(); 2];
        let mut heaps = [create(&mut pool, &mut mem, &params(1, 128 << 10, 2), &mut list).unwrap()];
        let chunk = heaps[0].first_chunk_gpu();
        for va in [
            // below the pool: the offset wraps above U32_MAX (:443)
            POOL_VA - 64,
            // slot 1, empty (:447-450)
            POOL_VA + 64,
            // slot 128 and the end of the pool's 4 GiB of offsets (:443)
            POOL_VA + 128 * 64,
            POOL_VA + (1 << 32),
        ] {
            assert_eq!(grow(&pool, &mut mem, &mut heaps, va, 0, 0), Err(E::UnknownHeap { heap_gpu_va: va }));
            assert_eq!(return_chunk(&pool, &mut mem, &mut heaps, va, chunk), Err(E::UnknownHeap { heap_gpu_va: va }));
        }
        // a slot taken, but its heap not among those given
        assert_eq!(grow(&pool, &mut mem, &mut [], POOL_VA, 0, 0), Err(E::UnknownHeap { heap_gpu_va: POOL_VA }));
        assert_eq!(mem.chunks_asked, 1);
        assert!(mem.freed.is_empty());
        assert_eq!(heaps[0].chunks().len(), 1);
    }

    #[test]
    fn return_chunk_takes_out_a_middle_chunk_and_a_grown_head() {
        let mut mem = Fake::new(16 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let mut list = [Allocation::default(); 64];
        let mut heaps = [create(&mut pool, &mut mem, &HeapParams::MESA, &mut list).unwrap()];
        let c: Vec<Allocation> = heaps[0].chunks().to_vec();
        // the middle chunk, named with grow's encoding: the low bits are
        // dropped (:389)
        assert_eq!(return_chunk(&pool, &mut mem, &mut heaps, POOL_VA, c[2].gpu | 0x200), Ok(()));
        assert_eq!(mem.freed, [c[2]]);
        assert_eq!(heaps[0].chunks(), &[c[0], c[1], c[3], c[4]][..]);
        assert_eq!(heaps[0].first_chunk_gpu(), c[4].gpu);
        // panthor rewrites no header (:392-400): the chunk that pointed at
        // the returned one still does
        assert_eq!(mem.word(c[3].gpu), c[2].gpu | 0x200);
        // the head: a chunk grow made, given back as panthor's handler does
        // with grow's value (panthor_sched.c:1524-1525)
        let v = grow(&pool, &mut mem, &mut heaps, POOL_VA, 0, 0).unwrap();
        let grown = heaps[0].chunks()[4];
        assert_eq!(v, grown.gpu | 0x200);
        assert_eq!(return_chunk(&pool, &mut mem, &mut heaps, POOL_VA, v), Ok(()));
        assert_eq!(mem.freed, [c[2], grown]);
        // the list re-walked: back to the four, c[4] the head again, and its
        // chain through the headers as create wrote it
        assert_eq!(heaps[0].chunks(), &[c[0], c[1], c[3], c[4]][..]);
        assert_eq!(heaps[0].osal().first_chunk_gpu, c[4].gpu);
        assert_eq!(walk(&mem, heaps[0].first_chunk_gpu()), [c[4].gpu, c[3].gpu, c[2].gpu, c[1].gpu, c[0].gpu]);
        // the freed entries are cleared, and grow takes the next one again
        assert_eq!(list_tail(&heaps[0]), Allocation::default());
        assert!(grow(&pool, &mut mem, &mut heaps, POOL_VA, 0, 0).is_ok());
        assert_eq!(heaps[0].chunks().len(), 5);
    }

    fn list_tail(h: &Heap<'_>) -> Allocation {
        h.chunks[h.chunk_count as usize]
    }

    #[test]
    fn return_chunk_refuses_an_unknown_chunk_and_can_empty_the_list() {
        let mut mem = Fake::new(2 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let mut list = [Allocation::default(); 2];
        let mut heaps = [create(&mut pool, &mut mem, &params(1, 128 << 10, 2), &mut list).unwrap()];
        let only = heaps[0].chunks()[0];
        // a VA no chunk has (:407-408), reported with the low bits dropped;
        // an address inside a chunk is not the chunk
        for (va, want) in [(only.gpu + 0x1_0000, only.gpu + 0x1_0000), (0xdead_beef, 0xdead_b000), (0, 0)] {
            assert_eq!(
                return_chunk(&pool, &mut mem, &mut heaps, POOL_VA, va),
                Err(E::UnknownChunk { chunk_gpu_va: want })
            );
        }
        assert!(mem.freed.is_empty());
        // the only chunk can go, as in panthor; then it is unknown
        assert_eq!(return_chunk(&pool, &mut mem, &mut heaps, POOL_VA, only.gpu), Ok(()));
        assert_eq!(heaps[0].chunks(), &[][..]);
        assert_eq!(heaps[0].first_chunk_gpu(), 0);
        assert_eq!(
            return_chunk(&pool, &mut mem, &mut heaps, POOL_VA, only.gpu),
            Err(E::UnknownChunk { chunk_gpu_va: only.gpu })
        );
        assert_eq!(mem.freed, [only]);
    }

    #[test]
    fn destroy_gives_the_chunks_back_head_first_and_frees_the_slot() {
        let mut mem = Fake::new(4 << 20);
        let mut pool = HeapPool::create(&mut mem, T6_L2).unwrap();
        let mut list = [Allocation::default(); 4];
        let mut heaps = [create(&mut pool, &mut mem, &params(3, 128 << 10, 4), &mut list).unwrap()];
        grow(&pool, &mut mem, &mut heaps, POOL_VA, 0, 0).unwrap();
        let [heap] = heaps;
        let c: Vec<Allocation> = heap.chunks().to_vec();
        // a heap that is not this pool's (:232-233) is handed back untouched
        let mut other = Fake::new(1 << 20);
        let mut foreign = HeapPool::create(&mut other, T6_L2).unwrap();
        let (e, heap) = destroy(&mut foreign, &mut mem, heap).unwrap_err();
        assert_eq!(e, E::UnknownHeap { heap_gpu_va: POOL_VA });
        assert_eq!(heap.chunks(), &c[..]);
        assert!(mem.freed.is_empty());
        // the list, from the head (:206-207): the grown chunk first
        let list = destroy(&mut pool, &mut mem, heap).unwrap();
        assert_eq!(mem.freed, [c[3], c[2], c[1], c[0]]);
        assert_eq!(list, [Allocation::default(); 4]);
        assert_eq!(pool.used, 0);
        // the slot is free: grow finds no heap there, create takes it again
        assert_eq!(grow(&pool, &mut mem, &mut [], POOL_VA, 0, 0), Err(E::UnknownHeap { heap_gpu_va: POOL_VA }));
        let h = create(&mut pool, &mut mem, &params(1, 128 << 10, 4), list).unwrap();
        assert_eq!((h.id(), h.ctx_gpu()), (0, POOL_VA));
    }
}
