// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Unit-test support (`#[cfg(test)]` only): a mock OSAL, a recording mock
//! backend and a guard that gives a test a fresh GL context.
//!
//! ```ignore
//! let gl = TestGl::new(64, 64);           // serialises tests, installs the mocks
//! unsafe { glClear(GL_COLOR_BUFFER_BIT) };
//! ...
//! let draws = gl.draws();                 // every GlesDraw the core resolved
//! let bytes = gl.gpu_bytes(gpu_va, len);  // what the GPU sees at that address
//! ```
//!
//! The library is single-threaded by contract and keeps global state (the
//! current context, the context pool, per-context caches), so `TestGl` holds a
//! process-wide lock for its lifetime: tests that use it run one at a time
//! whatever `--test-threads` says. Dropping it destroys the context and
//! removes the mocks, also when the test panics.
//!
//! # The memory model
//!
//! The target (AM62P) is not cache-coherent from the core's point of view: a
//! CPU write reaches the GPU only through `cache_flush`, a GPU write reaches
//! the CPU only through `cache_invd`. The mock models that with two copies
//! of every allocation: the CPU view (what the `cpu` pointer maps) and the
//! device memory (what the GPU reads and writes, [`TestGl::gpu_bytes`]).
//! `cache_flush` copies CPU view -> device, `cache_invd` device -> CPU view,
//! `gpu_write` / `gpu_read` work on the device memory. A third copy (the
//! value at the last synchronisation) tells an unflushed CPU write from a
//! stale CPU view.
//!
//! Fresh allocations are filled with [`FILL`] (the platform OSALs do not
//! clear them), unless a test asks for zeroed memory
//! ([`TestGl::set_zero_fill`]). A test can also take the CPU mapping away
//! ([`TestGl::set_cpu_mapping`]) to drive the `cpu == NULL` fallbacks, and
//! make `gpu_write` / `gpu_read` fail.
//!
//! # Faults
//!
//! What the mock detects is recorded as a fault; the next `TestGl` call (and
//! dropping the guard) panics with the list, so the test fails at the first
//! assertion after the offending GL call. A mock callback cannot panic
//! itself: it is an `extern "C"` function (a panic there aborts the whole
//! test binary). Detected:
//! - `gpu_free` of an address freed before (a double free, also through a
//!   copy of the descriptor), of a descriptor the mock never handed out, and
//!   of a descriptor whose `impl` does not match;
//! - `cache_flush` / `cache_invd` / `gpu_write` / `gpu_read` of memory that
//!   is not a live allocation or leaves it, and a `cache_invd` that discards
//!   an unflushed CPU write;
//! - a draw that reads GPU memory holding an unflushed CPU write, memory
//!   outside any allocation or already freed, never-written memory (four
//!   consecutive bytes nobody wrote; [`TestGl::allow_uninit_reads`] turns
//!   this off for a test whose GL use is undefined on purpose), or whose
//!   input changes between the draw and the completion of its kick;
//! - a `cache_invd` of a render target whose kick has not completed (the CPU
//!   would read a frame the GPU is still writing).
//!
//! # The kick model
//!
//! Like the PowerVR backend: `kick_id()` names the frame being recorded
//! (the last submitted one + 1); a draw opens that frame (a draw to another
//! target submits the open frame first); `flush` submits it; a kick is
//! pending from its first draw until it completes. Kicks complete when the
//! core waits for them (`finish`: all submitted, `finish_previous`: all but
//! the last), when a test completes them ([`TestGl::complete_kicks`]) or
//! automatically `lag` submits later ([`TestGl::set_kick_lag`]; 0 behaves
//! like a synchronous GPU). [`TestGl::set_kick_pending`] overrides the model
//! for one id. Freed memory stays readable for the GPU until the kick that
//! was being recorded at the time completes (the Zephyr OSAL defers frees
//! the same way).
//!
//! Run the unit tests with `cargo test --no-default-features --features host-test,gpu-powervr`.

use core::ffi::{c_char, c_void};
use std::collections::{BTreeMap, BTreeSet};
use std::string::String;
use std::sync::{Mutex, MutexGuard};
use std::vec::Vec;
use std::{format, vec};

use crate::backend::{
    gles_surf_bytes_per_pixel, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_HEAP_PROGRAM, GLES_HEAP_SHADER,
    GLES_TEXFMT_RGBA8,
};
use crate::gl::consts::*;
use crate::gl::context::{self, GlContext};
use crate::gl::types::*;
use crate::osal::{OsalGpuMem, OsalOps};

/// Device-VA heap bases the mock backend reports (the PowerVR layout).
pub const HEAP_DATA: u64 = 0x80_0000_0000;
pub const HEAP_PROGRAM: u64 = 0xDA_0000_0000;
pub const HEAP_SHADER: u64 = 0xE0_0000_0000;
const PAGE: u64 = 4096;

/// What a fresh allocation holds unless the test asked for zeroed memory.
pub const FILL: u8 = 0xA5;

/// The faults a test may leave unreported before the guard gives up
/// collecting (one bug tends to repeat per draw).
const MAX_FAULTS: usize = 32;

/// One draw as the backend received it, with the draw-time data it points at
/// copied out (the core reuses that staging for the next draw).
pub struct RecordedDraw {
    pub draw: GlesDraw,
    /// The VS / FS uniform blocks (`*_uniform_cpu`, `*_uniform_dwords` dwords).
    pub vs_uniforms: Vec<u32>,
    pub fs_uniforms: Vec<u32>,
    /// Per attribute: the staged bytes (`cpu`, `cpu_bytes`), empty when the
    /// attribute is fetched from GPU memory.
    pub attrib_data: Vec<Vec<u8>>,
    /// The staged u16 indices (`index_cpu`, `index_bytes`), empty otherwise.
    pub index_data: Vec<u8>,
    /// The kick the draw was recorded into.
    pub kick: u32,
}

// SAFETY: the raw pointers in `draw` are never dereferenced after recording
// (the data they named is copied out); tests are serialised.
unsafe impl Send for RecordedDraw {}

/// One OSAL allocation event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemEvent {
    Alloc { heap: u64, size: usize, gpu: u64 },
    Free { gpu: u64 },
}

/// A cache maintenance call, resolved to the allocation it touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheOp {
    /// `cache_flush`: CPU writes -> device memory.
    Flush { gpu: u64, off: usize, len: usize },
    /// `cache_invd`: device memory -> CPU view.
    Invd { gpu: u64, off: usize, len: usize },
}

impl CacheOp {
    /// Does this op cover `[off, off + len)` of the allocation at `gpu`?
    pub fn covers(&self, flush: bool, gpu: u64, off: usize, len: usize) -> bool {
        let (is_flush, g, o, l) = match *self {
            CacheOp::Flush { gpu, off, len } => (true, gpu, off, len),
            CacheOp::Invd { gpu, off, len } => (false, gpu, off, len),
        };
        is_flush == flush && g == gpu && o <= off && off + len <= o + l
    }
}

struct Allocation {
    heap: u64,
    size: usize,
    /// The descriptor's `impl` as handed out (an address).
    impl_: usize,
    /// Backing of the CPU mapping (also kept when the mapping was not handed out).
    cpu_view: Vec<u8>,
    /// The memory as the device sees it.
    dev: Vec<u8>,
    /// Both views' value at the last synchronisation of each byte.
    sync: Vec<u8>,
    /// Device memory somebody wrote (a flushed CPU write, gpu_write, the GPU).
    init: Vec<bool>,
    /// The `cpu` pointer was handed out.
    mapped: bool,
}

impl Allocation {
    fn cpu_base(&self) -> usize {
        self.cpu_view.as_ptr() as usize
    }

    /// Offsets in `[off, off + len)` holding a CPU write that was not flushed.
    fn unflushed(&self, off: usize, len: usize) -> Option<usize> {
        if !self.mapped {
            return None;
        }
        (off..off + len).find(|&i| self.cpu_view[i] != self.dev[i] && self.dev[i] == self.sync[i])
    }

    /// The first run of four never-written bytes in `[off, off + len)`.
    fn uninit(&self, off: usize, len: usize) -> Option<usize> {
        let mut run = 0;
<<<<<<< HEAD
        for (i, &init) in self.init.iter().enumerate().take(off + len).skip(off) {
            run = if init { 0 } else { run + 1 };
=======
        for (i, &written) in (off..).zip(&self.init[off..off + len]) {
            run = if written { 0 } else { run + 1 };
>>>>>>> feace3b018878aa4f6f5201da2eb14d5e987f2e3
            if run == 4 {
                return Some(i - 3);
            }
        }
        None
    }
}

/// Memory a draw makes the GPU read: its device bytes at draw time.
struct Snapshot {
    base: u64,
    off: usize,
    bytes: Vec<u8>,
    what: &'static str,
}

struct Frame {
    id: u32,
    /// The render target (the backend starts a new frame for another one).
    target: u64,
    snaps: Vec<Snapshot>,
}

struct Mock {
    allocs: BTreeMap<u64, Allocation>,
    /// Freed allocations with the kick that must complete before the GPU
    /// stops reading them (`None` once it has: any access is a use after free).
    zombies: BTreeMap<u64, (Allocation, Option<u32>)>,
    freed: BTreeSet<u64>,
    /// map_phys-style mappings of memory the test owns: gpu -> (size, cpu).
    mappings: BTreeMap<u64, (usize, usize)>,
    bump: [u64; 3],
    events: Vec<MemEvent>,
    cache_ops: Vec<CacheOp>,
    faults: Vec<String>,
    /// Fail the n-th next gpu_alloc / gpu_write / gpu_read (1 = the next one); 0 = never.
    fail_alloc_in: u32,
    fail_write_in: u32,
    fail_read_in: u32,
    /// Every gpu_write / gpu_read fails.
    io_fails: bool,
    zero_fill: bool,
    cpu_map: bool,
    allow_uninit: bool,
    draws: Vec<RecordedDraw>,
    flushes: u32,
    finishes: u32,
    /// Every surface_swapped call: the surface, and the flushes before it.
    swaps: Vec<(u64, u32)>,
    /// attachment_gone calls, in order.
    gones: Vec<u32>,
    /// Every texture_uploaded call.
    uploads: Vec<Upload>,
    /// Every wait_kick call (MOCK_BACKEND_WAIT_KICK), the kick waited for.
    waited_kicks: Vec<u32>,
    /// Every depth_resolve call.
    depth_resolves: Vec<DepthResolve>,
    /// What depth_resolve writes into every texel (`None`: it holds
    /// nothing, and answers 1).
    resolve_depth: Option<f32>,
    /* the kick model */
    last_fence: u32,
    completed: u32,
    frame_open: bool,
    frame_target: u64,
    frames: Vec<Frame>,
    overrides: BTreeMap<u32, bool>,
    lag: Option<u32>,
    /// What the mock backend's draw() / flush() / finish() return.
    draw_result: i32,
    flush_result: i32,
    finish_result: i32,
    /// Lines the library logged through the OSAL.
    logs: Vec<String>,
    /// What gpu_free returns: 0 accepts; a negative errno refuses the way a
    /// full pending-free queue does (src/osal/pending_frees.h): the
    /// descriptor is cleared and the memory kept for good.
    free_result: i32,
    /// The Zephyr OSAL's hold on freed memory (src/osal/zephyr.c
    /// z_gpu_free): every free is tagged with the kick after the last one
    /// submitted, recording or not, and its pages are reusable only once
    /// that kick has completed. gpu -> (page-rounded bytes, that kick). This
    /// is separate from `zombies`, which is the GL core's safety rule (when
    /// a GPU read of freed memory becomes a fault) and must not get more
    /// lenient: the OSAL's hold is what decides reuse, the zombie fence what
    /// decides correctness.
    held: BTreeMap<u64, (usize, u32)>,
    /// A pool of this many bytes (`None`: unbounded, the default): an
    /// allocation fails with -ENOMEM when the live bytes, the held bytes and
    /// the request (all page-rounded) would pass it.
    budget: Option<usize>,
    /// The OSAL's CONFIG_YAKOGL_PVR_POOL_RECLAIM arm (src/osal/zephyr.c): a
    /// free made with no frame open is held only for the last submitted
    /// kick (src/pvr/backend.rs `pvr_backend_free_fence`), and an allocation
    /// past the budget waits for the submitted kicks and is tried once more.
    reclaim: bool,
    /// OSAL_QUERY_CLOCK_NS through [`MOCK_OSAL_CLOCK`]: (the next answer,
    /// the step each answer adds).
    clock: (u64, u64),
    /// Every OSAL_QUERY_DRAW_STATS report through [`MOCK_OSAL_CLOCK`].
    draw_stats: Vec<crate::draw_steps::OsalDrawStats>,
}

impl Default for Mock {
    fn default() -> Self {
        Mock {
            allocs: BTreeMap::new(),
            zombies: BTreeMap::new(),
            freed: BTreeSet::new(),
            mappings: BTreeMap::new(),
            bump: [0; 3],
            events: Vec::new(),
            cache_ops: Vec::new(),
            faults: Vec::new(),
            fail_alloc_in: 0,
            fail_write_in: 0,
            fail_read_in: 0,
            io_fails: false,
            zero_fill: false,
            cpu_map: true,
            allow_uninit: false,
            draws: Vec::new(),
            flushes: 0,
            swaps: Vec::new(),
            gones: Vec::new(),
            uploads: Vec::new(),
            waited_kicks: Vec::new(),
            depth_resolves: Vec::new(),
            resolve_depth: None,
            finishes: 0,
            last_fence: 0,
            completed: 0,
            frame_open: false,
            frame_target: 0,
            frames: Vec::new(),
            overrides: BTreeMap::new(),
            lag: None,
            draw_result: 0,
            flush_result: 0,
            finish_result: 0,
            logs: Vec::new(),
            free_result: 0,
            held: BTreeMap::new(),
            budget: None,
            reclaim: false,
            clock: (0, 0),
            draw_stats: Vec::new(),
        }
    }
}

static MOCK: Mutex<Option<Mock>> = Mutex::new(None);
static SERIAL: Mutex<()> = Mutex::new(());

std::thread_local! {
    /// Whether this thread holds [`SERIAL`] through a live [`TestGl`]. The
    /// library's EGL state is one process-wide record (egl.rs `state()`), so
    /// a test that reaches it without the lock races every other test that
    /// does: four `eglGetProcAddress` tests reset the shared error status
    /// while `create_sync_errors` and `create_image_errors` were reading
    /// theirs, about once in twenty loaded runs. `egl::state()` checks this
    /// in test builds, so the next such test fails every time instead.
    pub static HOLDS_SERIAL: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

fn with_mock<R>(f: impl FnOnce(&mut Mock) -> R) -> R {
    let mut g = MOCK.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Mock::default))
}

/// Where a device address points.
enum Place {
    Live(u64, usize),
    Zombie(u64, usize, bool),
    Mapping(u64, usize, usize),
    Nowhere,
}

impl Mock {
    fn fault(&mut self, msg: String) {
        if self.faults.len() < MAX_FAULTS {
            self.faults.push(msg);
        }
    }

    /// What the pool holds: live allocations and frees the OSAL still holds,
    /// page-rounded.
    fn pool_bytes(&self) -> usize {
        self.allocs.values().map(|a| pages(a.size)).sum::<usize>() + self.held.values().map(|h| h.0).sum::<usize>()
    }

    fn kick_id(&self) -> u32 {
        self.last_fence.wrapping_add(1)
    }

    fn kick_pending(&self, id: u32) -> bool {
        match self.overrides.get(&id) {
            Some(p) => *p,
            None => id != 0 && id > self.completed && id <= self.kick_id(),
        }
    }

    /// Resolve device address `gpu` (inside an allocation, or one past its end).
    fn place(&self, gpu: u64) -> Place {
        if let Some((&b, a)) = self.allocs.range(..=gpu).next_back() {
            if gpu - b < a.size as u64 {
                return Place::Live(b, (gpu - b) as usize);
            }
        }
        if let Some((&b, (a, fence))) = self.zombies.range(..=gpu).next_back() {
            if gpu - b < a.size as u64 {
                return Place::Zombie(b, (gpu - b) as usize, fence.is_none());
            }
        }
        if let Some((&b, &(size, _))) = self.mappings.range(..=gpu).next_back() {
            if gpu - b < size as u64 {
                return Place::Mapping(b, (gpu - b) as usize, size);
            }
        }
        Place::Nowhere
    }

    fn alloc_any(&self, base: u64) -> Option<&Allocation> {
        self.allocs.get(&base).or_else(|| self.zombies.get(&base).map(|z| &z.0))
    }

    /// The live allocation whose CPU view holds `[cpu, cpu + n)`: (gpu, offset).
    fn by_cpu(&mut self, what: &str, cpu: *mut c_void, n: usize) -> Option<(u64, usize)> {
        let p = cpu as usize;
        let hit = |a: &Allocation| p >= a.cpu_base() && p < a.cpu_base() + a.size.max(1);
        if let Some((&g, a)) = self.allocs.iter().find(|(_, a)| hit(a)) {
            let off = p - a.cpu_base();
            if off + n > a.size {
                self.fault(format!("{what}({cpu:p}, {n}) leaves the allocation at 0x{g:x} ({} bytes)", a.size));
                return None;
            }
            return Some((g, off));
        }
        if let Some((&g, _)) = self.zombies.iter().find(|(_, z)| hit(&z.0)) {
            self.fault(format!("{what}({cpu:p}, {n}) of the freed allocation at 0x{g:x}"));
            return None;
        }
        if self.mappings.values().any(|&(size, c)| p >= c && p + n <= c + size) {
            return None; /* the test's own memory: coherent */
        }
        self.fault(format!("{what}({cpu:p}, {n}) of memory that is not a GPU allocation"));
        None
    }

    /// The live allocation a descriptor names, for gpu_write / gpu_read.
    fn io_target(&mut self, what: &str, mem: &OsalGpuMem, off: usize, n: usize) -> Result<u64, i32> {
        match self.place(mem.gpu) {
            Place::Live(b, 0) => {
                let size = self.allocs[&b].size;
                if off.checked_add(n).is_none_or(|e| e > mem.size || e > size) {
                    self.fault(format!("{what}(0x{b:x}, off {off}, {n}) leaves the allocation ({size} bytes)"));
                    return Err(-crate::util::EINVAL);
                }
                Ok(b)
            }
            Place::Zombie(b, ..) => {
                self.fault(format!("{what} on the freed allocation at 0x{b:x}"));
                Err(-crate::util::EINVAL)
            }
            Place::Mapping(..) => Err(-crate::util::EINVAL), /* no bulk access to a mapping */
            _ => {
                self.fault(format!("{what} on a descriptor the mock never handed out: {mem:?}"));
                Err(-crate::util::EINVAL)
            }
        }
    }

    /* ---- kicks ---- */

    fn check_frame(&mut self, i: usize, when: &str) {
        let mut faults = Vec::new();
        for s in &self.frames[i].snaps {
            let now = match self.place(s.base) {
                Place::Live(..) | Place::Zombie(_, _, false) => {
                    let a = self.alloc_any(s.base).unwrap();
                    a.dev[s.off..s.off + s.bytes.len()].to_vec()
                }
                _ => {
                    faults.push(format!(
                        "{} at 0x{:x}+{} was released before kick {} {when}",
                        s.what, s.base, s.off, self.frames[i].id
                    ));
                    continue;
                }
            };
            if now != s.bytes {
                let at = now.iter().zip(&s.bytes).position(|(a, b)| a != b).unwrap();
                faults.push(format!(
                    "{} at 0x{:x}+{} changed (byte {at}) between the draw and kick {} {when}",
                    s.what, s.base, s.off, self.frames[i].id
                ));
            }
        }
        for f in faults {
            self.fault(f);
        }
    }

    fn submit(&mut self) {
        if !self.frame_open {
            return;
        }
        if let Some(i) = self.frames.iter().position(|f| f.id == self.kick_id()) {
            self.check_frame(i, "was submitted");
        }
        self.last_fence += 1;
        self.frame_open = false;
        if let Some(lag) = self.lag {
            self.complete(self.last_fence.saturating_sub(lag));
        }
    }

    /// Kicks up to `through` (submitted ones only) have run.
    fn complete(&mut self, through: u32) {
        let through = through.min(self.last_fence);
        if through <= self.completed {
            return;
        }
        let mut i = 0;
        while i < self.frames.len() {
            if self.frames[i].id <= through {
                self.check_frame(i, "completed");
                self.frames.remove(i);
            } else {
                i += 1;
            }
        }
        self.completed = through;
        /* the OSAL drains its queue when it learns of a completion */
        self.held.retain(|_, h| h.1 > through);
        for z in self.zombies.values_mut() {
            if z.1.is_some_and(|f| f <= through) {
                z.1 = None;
            }
        }
    }

    /* ---- draw checks ---- */

    /// A GPU read of `[gpu, gpu + len)` by the draw being recorded.
    fn gpu_reads(&mut self, snaps: &mut Vec<Snapshot>, what: &'static str, gpu: u64, len: usize, target: bool) {
        if gpu == 0 || len == 0 {
            return;
        }
        let (base, off) = match self.place(gpu) {
            Place::Live(b, o) => (b, o),
            Place::Zombie(b, o, released) => {
                if released {
                    return self.fault(format!("the draw reads {what} at 0x{gpu:x}: freed memory"));
                }
                (b, o)
            }
            Place::Mapping(b, o, size) => {
                if o + len > size {
                    self.fault(format!("the draw reads {what} at 0x{b:x}+{o}, {len} bytes: past the mapping"));
                }
                return;
            }
            Place::Nowhere => {
                return self.fault(format!("the draw reads {what} at 0x{gpu:x}: not GPU memory"));
            }
        };
        let a = self.alloc_any(base).unwrap();
        if off + len > a.size {
            let size = a.size;
            return self.fault(format!(
                "the draw reads {what} at 0x{base:x}+{off}, {len} bytes: past the allocation ({size} bytes)"
            ));
        }
        let unflushed = a.unflushed(off, len);
        let uninit = if target || self.allow_uninit { None } else { a.uninit(off, len) };
        let bytes = a.dev[off..off + len].to_vec();
        if let Some(i) = unflushed {
            self.fault(format!("the draw reads {what} at 0x{base:x}+{i}: a CPU write that was not flushed"));
        }
        if let Some(i) = uninit {
            self.fault(format!("the draw reads {what} at 0x{base:x}+{i}: memory nobody wrote"));
        }
        if !target {
            snaps.push(Snapshot { base, off, bytes, what });
        }
    }

    fn dev_bytes(&self, gpu: u64, len: usize) -> Option<Vec<u8>> {
        match self.place(gpu) {
            Place::Live(b, o) | Place::Zombie(b, o, _) => {
                let a = self.alloc_any(b)?;
                a.dev.get(o..o + len).map(|s| s.to_vec())
            }
            _ => None,
        }
    }

    fn on_draw(&mut self, d: &GlesDraw, rec: &RecordedDraw) {
        if self.frame_open && self.frame_target != d.target_gpu {
            self.submit(); /* the backend starts a new frame for a new target */
        }
        self.frame_open = true;
        self.frame_target = d.target_gpu;
        let id = self.kick_id();
        let mut snaps = Vec::new();

        /* the target: loaded and written by the kick */
        let bpp = gles_surf_bytes_per_pixel(d.target_format) as usize;
        self.gpu_reads(&mut snaps, "the target", d.target_gpu, d.target_w as usize * d.target_h as usize * bpp, true);
        // SAFETY: the core's shaders are valid for the draw.
        unsafe {
            if let Some(vs) = d.vs.as_ref() {
                self.gpu_reads(&mut snaps, "the VS code", d.vs_usc_gpu, vs.usc_dw as usize * 4, false);
            }
            if let Some(fs) = d.fs.as_ref() {
                self.gpu_reads(&mut snaps, "the FS code", d.fs_usc_gpu, fs.usc_dw as usize * 4, false);
            }
        }
        if d.vs_uniform_cpu.is_null() {
            self.gpu_reads(&mut snaps, "the VS uniforms", d.vs_uniform_gpu, d.vs_uniform_dwords as usize * 4, false);
        }
        if d.fs_uniform_cpu.is_null() {
            self.gpu_reads(&mut snaps, "the FS uniforms", d.fs_uniform_gpu, d.fs_uniform_dwords as usize * 4, false);
        }
        /* the vertices fetched */
        let mut indices: Vec<u8> = Vec::new();
        let indexed = d.index_buf_gpu != 0 || !d.index_cpu.is_null();
        if d.index_cpu.is_null() {
            let n = d.index_count as usize * 2;
            self.gpu_reads(&mut snaps, "the index buffer", d.index_buf_gpu, n, false);
            indices = self.dev_bytes(d.index_buf_gpu, n).unwrap_or_default();
        } else {
            indices.extend_from_slice(&rec.index_data);
        }
        if d.index_count2 != 0 {
            let n = d.index_count2 as usize * 2;
            self.gpu_reads(&mut snaps, "the second index buffer", d.index_buf2_gpu, n, false);
            indices.extend(self.dev_bytes(d.index_buf2_gpu, n).unwrap_or_default());
        }
        let verts = if indexed {
            indices.as_chunks::<2>().0.iter().map(|q| u16::from_ne_bytes(*q) as usize + 1).max().unwrap_or(0)
        } else {
            d.index_count as usize
        };
        for a in &d.attribs[..(d.num_attribs as usize).min(16)] {
            if !a.cpu.is_null() || verts == 0 {
                continue;
            }
            /* one element is the attribute's components in its source
             * format: a byte or short array a backend fetches in place is
             * narrower than float32 (GlesAttrib::format) */
            let tsz = match a.format {
                crate::backend::GLES_ATTRIBF_BYTE | crate::backend::GLES_ATTRIBF_UBYTE => 1,
                crate::backend::GLES_ATTRIBF_SHORT | crate::backend::GLES_ATTRIBF_USHORT => 2,
                _ => 4,
            };
            let last = a.components as usize * tsz;
            let len = if a.stride == 0 { last } else { a.stride as usize * (verts - 1) + last };
            self.gpu_reads(&mut snaps, "a vertex attribute", a.buffer_gpu.wrapping_add(a.offset as u64), len, false);
        }
        for t in &d.textures[..(d.num_textures as usize).min(8)] {
            if t.format == GLES_TEXFMT_RGBA8 && t.stride_texels > t.width {
                /* rows padded to the stride: the pad texels are never sampled */
                for y in 0..t.height as u64 {
                    let row = t.base_gpu + y * t.stride_texels as u64 * 4;
                    self.gpu_reads(&mut snaps, "a texture", row, t.width as usize * 4, false);
                }
            } else {
                self.gpu_reads(&mut snaps, "a texture", t.base_gpu, t.bytes as usize, false);
            }
        }
        match self.frames.iter_mut().find(|f| f.id == id) {
            Some(f) => f.snaps.extend(snaps),
            None => self.frames.push(Frame { id, target: d.target_gpu, snaps }),
        }
    }
}

/* ---- mock OSAL ------------------------------------------------------------- */

/// `n` bytes rounded up to whole pages, as the OSAL allocates them.
fn pages(n: usize) -> usize {
    n.div_ceil(PAGE as usize) * PAGE as usize
}

fn heap_index(base: u64) -> Option<usize> {
    match base {
        HEAP_DATA => Some(0),
        HEAP_PROGRAM => Some(1),
        HEAP_SHADER => Some(2),
        _ => None,
    }
}

unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}

unsafe extern "C" fn m_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let Some(hi) = heap_index(heap_base) else {
        return -crate::util::EINVAL;
    };
    if out.is_null() || size == 0 {
        return -crate::util::EINVAL;
    }
    with_mock(|m| {
        if m.fail_alloc_in > 0 {
            m.fail_alloc_in -= 1;
            if m.fail_alloc_in == 0 {
                return -crate::util::ENOMEM;
            }
        }
        if let Some(budget) = m.budget {
            if m.pool_bytes() + pages(size) > budget && m.reclaim {
                /* what has completed is already given back (`complete`);
                 * the wait for the submitted kicks */
                let last = m.last_fence;
                m.complete(last);
            }
            if m.pool_bytes() + pages(size) > budget {
                return -crate::util::ENOMEM;
            }
        }
        let gpu = heap_base + m.bump[hi];
        m.bump[hi] += (size as u64).div_ceil(PAGE) * PAGE;
        let fill = if m.zero_fill { 0 } else { FILL };
        let mut a = Allocation {
            heap: heap_base,
            size,
            impl_: 0,
            cpu_view: vec![fill; size],
            dev: vec![fill; size],
            sync: vec![fill; size],
            init: vec![m.zero_fill; size],
            mapped: m.cpu_map,
        };
        /* a stable, unique, non-null bookkeeping handle */
        a.impl_ = a.dev.as_ptr() as usize;
        let cpu = if m.cpu_map { a.cpu_view.as_mut_ptr() as *mut c_void } else { core::ptr::null_mut() };
        // SAFETY: `out` is the caller's valid descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: a.impl_ as *mut c_void } };
        m.allocs.insert(gpu, a);
        m.events.push(MemEvent::Alloc { heap: heap_base, size, gpu });
        0
    })
}

unsafe extern "C" fn m_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: null or the caller's valid descriptor.
    let Some(mem) = (unsafe { mem.as_mut() }) else {
        return 0;
    };
    let d = *mem;
    *mem = OsalGpuMem::ZERO;
    if d.impl_.is_null() && d.gpu == 0 && d.cpu.is_null() && d.size == 0 {
        return 0; /* an empty descriptor */
    }
    let refused = with_mock(|m| m.free_result);
    if refused < 0 {
        return refused; /* kept for good: the allocation stays live */
    }
    with_mock(|m| {
        if let Some(a) = m.allocs.remove(&d.gpu) {
            if a.impl_ != d.impl_ as usize {
                m.fault(format!("gpu_free of 0x{:x} with a descriptor that is not the allocation's: {d:?}", d.gpu));
            }
            m.events.push(MemEvent::Free { gpu: d.gpu });
            m.freed.insert(d.gpu);
            /* the Zephyr OSAL's tag: the kick after the last submitted one;
             * with the reclaim, that kick only while a frame is open */
            let tag = if m.reclaim && !m.frame_open { m.last_fence } else { m.last_fence + 1 };
            if tag > m.completed {
                m.held.insert(d.gpu, (pages(a.size), tag));
            }
            /* the frame being recorded, or else the last one in flight, may
             * still read it */
            let fence = if m.frame_open { Some(m.kick_id()) } else { Some(m.last_fence).filter(|&f| f > m.completed) };
            m.zombies.insert(d.gpu, (a, fence));
        } else if m.mappings.remove(&d.gpu).is_some() {
            m.freed.insert(d.gpu);
        } else if m.freed.contains(&d.gpu) {
            m.fault(format!("double free: gpu_free of 0x{:x}, which was freed before ({d:?})", d.gpu));
        } else {
            m.fault(format!("gpu_free of a descriptor the mock never handed out: {d:?}"));
        }
    });
    0
}

unsafe extern "C" fn m_mmu_map(_va: u64, _phys: u64, _size: usize, _flags: u32) -> i32 {
    0
}

fn io_fails(n: &mut u32, always: bool) -> bool {
    if always {
        return true;
    }
    if *n > 0 {
        *n -= 1;
        return *n == 0;
    }
    false
}

unsafe extern "C" fn m_gpu_write(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
    // SAFETY: null or the caller's valid descriptor.
    let Some(mem) = (unsafe { mem.as_ref() }) else { return -crate::util::EINVAL };
    with_mock(|m| {
        if src.is_null() {
            // include/gles_osal.h: a NULL buffer is refused, also for n == 0.
            m.fault(format!("gpu_write with a NULL source ({n} bytes)"));
            return -crate::util::EINVAL;
        }
        if io_fails(&mut m.fail_write_in, m.io_fails) {
            return -crate::util::EIO;
        }
        let b = match m.io_target("gpu_write", mem, off, n) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let a = m.allocs.get_mut(&b).unwrap();
        // SAFETY: the core passes `n` readable bytes.
        let s = unsafe { core::slice::from_raw_parts(src as *const u8, n) };
        a.dev[off..off + n].copy_from_slice(s);
        a.sync[off..off + n].copy_from_slice(s);
        a.cpu_view[off..off + n].copy_from_slice(s);
        a.init[off..off + n].fill(true);
        0
    })
}

unsafe extern "C" fn m_gpu_read(mem: *mut OsalGpuMem, off: usize, dst: *mut c_void, n: usize) -> i32 {
    // SAFETY: null or the caller's valid descriptor.
    let Some(mem) = (unsafe { mem.as_ref() }) else { return -crate::util::EINVAL };
    with_mock(|m| {
        if dst.is_null() {
            // include/gles_osal.h: a NULL buffer is refused, also for n == 0.
            m.fault(format!("gpu_read with a NULL destination ({n} bytes)"));
            return -crate::util::EINVAL;
        }
        if io_fails(&mut m.fail_read_in, m.io_fails) {
            return -crate::util::EIO;
        }
        let b = match m.io_target("gpu_read", mem, off, n) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let a = &m.allocs[&b];
        // SAFETY: the core passes `n` writable bytes that do not overlap the mock's copy.
        unsafe { core::ptr::copy_nonoverlapping(a.dev[off..].as_ptr(), dst as *mut u8, n) };
        0
    })
}

unsafe extern "C" fn m_cache_flush(cpu: *mut c_void, n: usize) {
    with_mock(|m| {
        let Some((g, off)) = m.by_cpu("cache_flush", cpu, n) else { return };
        let a = m.allocs.get_mut(&g).unwrap();
        for i in off..off + n {
            if a.cpu_view[i] != a.sync[i] {
                a.init[i] = true;
            }
            a.dev[i] = a.cpu_view[i];
            a.sync[i] = a.cpu_view[i];
        }
        m.cache_ops.push(CacheOp::Flush { gpu: g, off, len: n });
    });
}

unsafe extern "C" fn m_cache_invd(cpu: *mut c_void, n: usize) {
    with_mock(|m| {
        let Some((g, off)) = m.by_cpu("cache_invd", cpu, n) else { return };
        let size = m.allocs[&g].size as u64;
        if let Some(f) = m.frames.iter().find(|f| f.target >= g && f.target < g + size) {
            let id = f.id;
            m.fault(format!("cache_invd of 0x{g:x} while kick {id}, which renders into it, has not completed"));
        }
        let a = m.allocs.get_mut(&g).unwrap();
        let lost = (off..off + n).find(|&i| a.cpu_view[i] != a.sync[i] && a.cpu_view[i] != a.dev[i]);
        for i in off..off + n {
            a.cpu_view[i] = a.dev[i];
            a.sync[i] = a.dev[i];
        }
        m.cache_ops.push(CacheOp::Invd { gpu: g, off, len: n });
        if let Some(i) = lost {
            m.fault(format!("cache_invd discards an unflushed CPU write at 0x{g:x}+{i}"));
        }
    });
}

/// The log hook: a Rust function with the two arguments `osal::log_str`
/// passes (`"%s"`, line).
extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: osal::log_str passes a NUL-terminated line.
    let text = unsafe { std::ffi::CStr::from_ptr(line) }.to_string_lossy().into_owned();
    with_mock(|m| m.logs.push(text));
}

/* The table's `log` is a C variadic function, which stable Rust cannot
 * define, and calling a non-variadic function through a variadic function
 * pointer is undefined behaviour in Rust. So the hook is a two-instruction
 * assembly trampoline declared to Rust as the variadic foreign function it
 * is: Rust only ever calls it through its true type. The trampoline
 * tail-jumps to m_log with the argument registers untouched, which is
 * ABI-correct exactly where the variadic calling convention passes the
 * leading pointer arguments in the same registers as a non-variadic call:
 * x86-64 System V (rdi, rsi; %al, the vector-register count, is ignored by a
 * non-variadic callee) and AArch64 AAPCS64 on Linux (x0, x1; Apple's arm64
 * ABI puts variadic arguments on the stack, so it is excluded). Other hosts
 * get no log hook: the tests that assert on log lines fail there. */
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
core::arch::global_asm!(
    ".pushsection .text.yakogl_test_log,\"ax\",@progbits",
    ".globl yakogl_test_log",
    ".hidden yakogl_test_log",
    ".type yakogl_test_log,@function",
    "yakogl_test_log:",
    "jmp {log}",
    ".size yakogl_test_log, . - yakogl_test_log",
    ".popsection",
    log = sym m_log,
);
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
core::arch::global_asm!(
    ".pushsection .text.yakogl_test_log,\"ax\",%progbits",
    ".globl yakogl_test_log",
    ".hidden yakogl_test_log",
    ".type yakogl_test_log,%function",
    "yakogl_test_log:",
    "b {log}",
    ".size yakogl_test_log, . - yakogl_test_log",
    ".popsection",
    log = sym m_log,
);

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
extern "C" {
    /// The trampoline above.
    fn yakogl_test_log(fmt: *const c_char, ...);
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
const LOG_HOOK: Option<unsafe extern "C" fn(*const c_char, ...)> = Some(yakogl_test_log);
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
const LOG_HOOK: Option<unsafe extern "C" fn(*const c_char, ...)> = None;

/// The mock platform. `submit`/`wait` are absent because the mock backend
/// never submits; `map_phys` is a test's to add ([`mock_map_phys`]).
/// The mock display: how many vertical periods it has been asked to wait
/// out, and whether it has any (a platform without vertical sync answers
/// -ENOTSUP, which is what tells EGL that presentation is immediate).
static VBLANKS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static HAS_VBLANK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

unsafe extern "C" fn mock_wait_vblank(_timeout_us: i64) -> i32 {
    if !HAS_VBLANK.load(core::sync::atomic::Ordering::Relaxed) {
        return -crate::util::ENOTSUP;
    }
    VBLANKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    0
}

/* ---- the shared-state lock -------------------------------------------------
 *
 * The mock is single-threaded, so these do not block; they count. Two things
 * are worth counting. A depth that ever reaches 2 is an entry point taking
 * the lock while already holding it, which a real non-recursive mutex does
 * not survive -- on the board it is a hang, and the test suite is where that
 * should be found instead. A depth that is not 0 when an entry point returns
 * is a lock it never gave back. */
static LOCK_DEPTH: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(0);
static LOCK_MAX_DEPTH: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(0);
static LOCK_TAKEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

extern "C" fn m_lock() {
    let d = LOCK_DEPTH.fetch_add(1, core::sync::atomic::Ordering::SeqCst) + 1;
    LOCK_MAX_DEPTH.fetch_max(d, core::sync::atomic::Ordering::SeqCst);
    LOCK_TAKEN.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
}

extern "C" fn m_unlock() {
    LOCK_DEPTH.fetch_sub(1, core::sync::atomic::Ordering::SeqCst);
}

/* ---- thread ids -------------------------------------------------------------
 *
 * Like the Linux OSALs: every std thread gets its own non-zero id, so the
 * library's owner check (context::foreign_thread) is live in every test that
 * makes a context current through eglMakeCurrent. A test turns the ids off
 * ([`set_thread_ids`]) to model a platform that cannot tell threads apart;
 * each TestGl starts with them on. */
static THREAD_IDS_ON: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);
static NEXT_THREAD_ID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);
std::thread_local! {
    static THREAD_ID: u32 = NEXT_THREAD_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

extern "C" fn m_thread_id() -> u32 {
    if !THREAD_IDS_ON.load(core::sync::atomic::Ordering::Relaxed) {
        return 0;
    }
    THREAD_ID.with(|t| *t)
}

/// Whether the mock OSAL names threads (true) or answers 0 for every thread
/// (false: the platform cannot tell, and the owner check is off).
pub fn set_thread_ids(on: bool) {
    THREAD_IDS_ON.store(on, core::sync::atomic::Ordering::Relaxed);
}

/// (times taken, depth now, deepest it ever went) since [`lock_counts_reset`].
pub fn lock_counts() -> (u32, i32, i32) {
    (
        LOCK_TAKEN.load(core::sync::atomic::Ordering::SeqCst),
        LOCK_DEPTH.load(core::sync::atomic::Ordering::SeqCst),
        LOCK_MAX_DEPTH.load(core::sync::atomic::Ordering::SeqCst),
    )
}

pub fn lock_counts_reset() {
    LOCK_TAKEN.store(0, core::sync::atomic::Ordering::SeqCst);
    LOCK_DEPTH.store(0, core::sync::atomic::Ordering::SeqCst);
    LOCK_MAX_DEPTH.store(0, core::sync::atomic::Ordering::SeqCst);
}

static MOCK_OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: Some(m_mmu_map),
    map_phys: None,
    gpu_write: Some(m_gpu_write),
    gpu_read: Some(m_gpu_read),
    cache_flush: Some(m_cache_flush),
    cache_invd: Some(m_cache_invd),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: LOG_HOOK,
    wait_vblank: Some(mock_wait_vblank),
    lock: Some(m_lock),
    unlock: Some(m_unlock),
    thread_id: Some(m_thread_id),
};

/// [`MOCK_OSAL`] with a clock (OSAL_QUERY_CLOCK_NS) that keeps the draw
/// stats reported to it (OSAL_QUERY_DRAW_STATS) and answers nothing else:
/// [`TestGl::set_clock`] installs it, so every other test keeps an OSAL
/// with no query at all.
static MOCK_OSAL_CLOCK: OsalOps = OsalOps { query: Some(m_query_clock), ..MOCK_OSAL };

unsafe extern "C" fn m_query_clock(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    if what == crate::osal::OSAL_QUERY_DRAW_STATS
        && !out.is_null()
        && out_sz == core::mem::size_of::<crate::draw_steps::OsalDrawStats>()
    {
        // SAFETY: the library's report of the size checked above.
        let d = unsafe { *(out as *const crate::draw_steps::OsalDrawStats) };
        with_mock(|m| m.draw_stats.push(d));
        return 0;
    }
    if what != crate::osal::OSAL_QUERY_CLOCK_NS || out.is_null() || out_sz != 8 {
        return -crate::util::ENOTSUP;
    }
    let ns = with_mock(|m| {
        let now = m.clock.0;
        m.clock.0 += m.clock.1;
        now
    });
    // SAFETY: a u64 out-parameter of the size given.
    unsafe { *(out as *mut u64) = ns };
    0
}

/// Register a map_phys-style mapping of `size` bytes of the test's own
/// memory at `cpu` (null: no CPU view) at device address `gpu`. It is
/// coherent (no cache model), may be freed once, and bounds draws that
/// render into it or sample it.
pub fn mock_map_phys(gpu: u64, size: usize, cpu: *mut c_void) {
    with_mock(|m| {
        m.freed.remove(&gpu);
        m.mappings.insert(gpu, (size, cpu as usize));
    });
}

/// The GPU writes `bytes` at `gpu` (rendering): the device memory changes,
/// the CPU view does not until the core invalidates it.
pub fn gpu_render(gpu: u64, bytes: &[u8]) {
    with_mock(|m| {
        let Place::Live(base, off) = m.place(gpu) else { panic!("gpu_render: no live allocation at 0x{gpu:x}") };
        let a = m.allocs.get_mut(&base).unwrap();
        assert!(off + bytes.len() <= a.size, "gpu_render: range leaves the allocation");
        a.dev[off..off + bytes.len()].copy_from_slice(bytes);
        a.init[off..off + bytes.len()].fill(true);
    });
}

/* ---- mock backend ---------------------------------------------------------- */

unsafe extern "C" fn b_init() -> i32 {
    0
}

/// Copy `n` bytes at `p` (empty for null / 0).
unsafe fn copy_bytes(p: *const c_void, n: usize) -> Vec<u8> {
    if p.is_null() || n == 0 {
        return Vec::new();
    }
    // SAFETY: the core guarantees `n` readable bytes for the draw's duration.
    unsafe { core::slice::from_raw_parts(p as *const u8, n).to_vec() }
}

fn to_dwords(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_ne_bytes(*c)).collect()
}

unsafe extern "C" fn b_draw(draw: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a valid draw.
    let d = unsafe { *draw };
    // SAFETY: the draw's staging pointers are valid for the sizes it names.
    let mut rec = unsafe {
        RecordedDraw {
            draw: d,
            vs_uniforms: to_dwords(&copy_bytes(d.vs_uniform_cpu, d.vs_uniform_dwords as usize * 4)),
            fs_uniforms: to_dwords(&copy_bytes(d.fs_uniform_cpu, d.fs_uniform_dwords as usize * 4)),
            attrib_data: d.attribs[..(d.num_attribs as usize).min(16)]
                .iter()
                .map(|a| copy_bytes(a.cpu, a.cpu_bytes as usize))
                .collect(),
            index_data: copy_bytes(d.index_cpu, d.index_bytes as usize),
            kick: 0,
        }
    };
    with_mock(|m| {
        if m.draw_result != 0 {
            m.draws.push(rec);
            return m.draw_result;
        }
        m.on_draw(&d, &rec);
        rec.kick = m.kick_id();
        m.draws.push(rec);
        0
    })
}

unsafe extern "C" fn b_flush() -> i32 {
    with_mock(|m| {
        m.flushes += 1;
        if m.flush_result != 0 {
            return m.flush_result;
        }
        m.submit();
        0
    })
}

unsafe extern "C" fn b_surface_swapped(surface_gpu: u64) {
    with_mock(|m| {
        let n = m.flushes;
        m.swaps.push((surface_gpu, n))
    });
}

unsafe extern "C" fn b_attachment_gone(id: u32) {
    with_mock(|m| m.gones.push(id));
}

/// One depth_resolve call as the mock backend saw it: the object, the size
/// and pitch, and how many flushes and finishes came before it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthResolve {
    pub id: u32,
    pub w: u32,
    pub h: u32,
    pub pitch: u32,
    pub flushes: u32,
    pub finishes: u32,
}

unsafe extern "C" fn b_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    let fill = with_mock(|m| {
        let (flushes, finishes) = (m.flushes, m.finishes);
        m.depth_resolves.push(DepthResolve { id, w, h, pitch, flushes, finishes });
        m.resolve_depth
    });
    let Some(v) = fill else { return 1 };
    for y in 0..h as usize {
        for x in 0..w as usize {
            // SAFETY: the caller's contract: h rows of `pitch` floats at `dst`.
            unsafe { dst.add(y * pitch as usize + x).write(v) };
        }
    }
    0
}

/// One texture_uploaded call as the mock backend saw it: the binding, the
/// level and box, the source pointer and pitch, and the first row's bytes.
#[derive(Clone)]
pub struct Upload {
    pub b: GlesTextureBind,
    pub level: u32,
    pub rect: [u32; 4],
    pub src: usize,
    pub stride: u32,
    pub first_row: Vec<u8>,
}

// SAFETY: the binding's `cpu` pointer is never dereferenced after recording
// (the first row is copied out); tests are serialised.
unsafe impl Send for Upload {}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn b_texture_uploaded(
    b: *const GlesTextureBind,
    level: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    src: *const c_void,
    stride: u32,
) {
    // SAFETY: the GL core's live binding, and h rows of `stride` bytes at src.
    let (b, first_row) = unsafe { (*b, core::slice::from_raw_parts(src as *const u8, w as usize * 4).to_vec()) };
    with_mock(|m| m.uploads.push(Upload { b, level, rect: [x, y, w, h], src: src as usize, stride, first_row }));
}

unsafe extern "C" fn b_finish() -> i32 {
    with_mock(|m| {
        m.finishes += 1;
        if m.finish_result != 0 {
            return m.finish_result;
        }
        m.complete(m.last_fence);
        0
    })
}

unsafe extern "C" fn b_finish_previous() -> i32 {
    with_mock(|m| {
        m.finishes += 1;
        if m.finish_result != 0 {
            return m.finish_result;
        }
        m.complete(m.last_fence.saturating_sub(1));
        0
    })
}

unsafe extern "C" fn b_kick_id() -> u32 {
    with_mock(|m| m.kick_id())
}

/// The mock's wait for one kick: that kick (and every one before it)
/// completes; nothing recorded is submitted, and no finish is counted.
unsafe extern "C" fn b_wait_kick(id: u32) -> i32 {
    with_mock(|m| {
        m.waited_kicks.push(id);
        m.overrides.remove(&id);
        m.complete(id);
        0
    })
}

unsafe extern "C" fn b_kick_pending(id: u32) -> bool {
    with_mock(|m| m.kick_pending(id))
}

unsafe extern "C" fn b_heap_base(heap: GlesHeap) -> u64 {
    match heap {
        GLES_HEAP_SHADER => HEAP_SHADER,
        GLES_HEAP_PROGRAM => HEAP_PROGRAM,
        _ => HEAP_DATA,
    }
}

static MOCK_BACKEND: YakoglBackend = YakoglBackend {
    /* the mock is a PowerVR-shaped backend: float32 arrays only, R,G,B,A
     * render targets, a packed mip chain. A test that wants the other
     * answers asks for `MOCK_BACKEND_GC7000` (TestGl::backend_like_gc7000) */
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    name: c"mock".as_ptr(),
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_flush),
    finish: Some(b_finish),
    finish_previous: Some(b_finish_previous),
    kick_id: Some(b_kick_id),
    kick_pending: Some(b_kick_pending),
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: Some(b_surface_swapped),
    texture_uploaded: Some(b_texture_uploaded),
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: Some(b_attachment_gone),
    depth_resolve: Some(b_depth_resolve),
};

/// [`MOCK_BACKEND`] with the capabilities of the GC7000: a vertex fetch that
/// reads every source format, an RGBA8888 target stored B,G,R,A, and a mip
/// chain at one row pitch with every level on a 64-byte boundary.
static MOCK_BACKEND_GC7000: YakoglBackend = YakoglBackend {
    native_attrib_formats: ((1u32 << crate::backend::GLES_ATTRIBF_COUNT) - 1),
    rt_rgba8888_is_bgra: true,
    mip_level_align: 64,
    compressed_blocks_in_rows: true,
    native_layered_textures: true,
    front_facing_shader_variant: true,
    etc1_decoded_at_upload: false,
    name: c"mock".as_ptr(),
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_flush),
    finish: Some(b_finish),
    finish_previous: Some(b_finish_previous),
    kick_id: Some(b_kick_id),
    kick_pending: Some(b_kick_pending),
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: Some(b_surface_swapped),
    texture_uploaded: Some(b_texture_uploaded),
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: Some(b_attachment_gone),
    depth_resolve: Some(b_depth_resolve),
};

/// [`MOCK_BACKEND`] whose texture unit has no ETC1, as the V3D's has none:
/// the GL core decodes ETC1 images at upload (`etc1_decoded_at_upload`).
/// Everything else is the PowerVR-shaped mock's, so a test sees the flag
/// alone switch the path.
static MOCK_BACKEND_ETC1_DECODED: YakoglBackend = YakoglBackend {
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: true,
    name: c"mock".as_ptr(),
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_flush),
    finish: Some(b_finish),
    finish_previous: Some(b_finish_previous),
    kick_id: Some(b_kick_id),
    kick_pending: Some(b_kick_pending),
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: Some(b_surface_swapped),
    texture_uploaded: Some(b_texture_uploaded),
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: Some(b_attachment_gone),
    depth_resolve: Some(b_depth_resolve),
};

/// [`MOCK_BACKEND`] that takes an instanced draw's arrays with any divisor up
/// to 0xffff as one draw (`max_instance_divisor`), as the V3D does; the
/// PowerVR-shaped mock takes divisor 1 only.
static MOCK_BACKEND_DIVISORS: YakoglBackend = YakoglBackend { max_instance_divisor: 0xffff, ..MOCK_BACKEND };

/// [`MOCK_BACKEND`] that waits for one kick (`wait_kick`), as the V3D does:
/// the sync waits then submit nothing recorded after their fence.
static MOCK_BACKEND_WAIT_KICK: YakoglBackend = YakoglBackend { wait_kick: Some(b_wait_kick), ..MOCK_BACKEND };

/// [`MOCK_BACKEND`] whose draws sample copies of the texels made when the
/// draw was recorded (`samples_texel_copies`), as the V3D's do: a texture
/// write then waits only for a kick that renders into the texture.
static MOCK_BACKEND_TEXEL_COPIES: YakoglBackend = YakoglBackend { samples_texel_copies: true, ..MOCK_BACKEND };

/* ---- the guard ------------------------------------------------------------- */

/// Put every process-global the library keeps back to its initial value, so
/// that no test sees what an earlier one (possibly a panicking one) left.
///
/// # Safety
/// Tests are serialised (SERIAL is held); no GL call is running.
unsafe fn reset_library_statics() {
    {
        context::reset_for_tests(); /* pool, current context, error log cap */
        crate::gl::draw::reset_for_tests(true); /* staging, program caches, pending clear, latches */
        crate::gl::buffer::reset_for_tests(); /* bind history, out-of-names latch */
        crate::gl::shader::reset_for_tests(); /* info logs, dump counter */
        crate::gl::program::reset_for_tests(); /* location tables, info logs */
        crate::gl::state::reset_for_tests(); /* glPixelStorei latch */
        crate::gl::texture::reset_for_tests(); /* ES 3.0 log budget */
        crate::gl::framebuffer::reset_for_tests(); /* multisample / blit latches */
        crate::gl::es3::reset_for_tests(); /* query, sampler and feedback names, syncs */
        crate::gl::egl::reset_for_tests(); /* the whole EGL state */
        set_thread_ids(true); /* the owner itself is in context::reset_for_tests */
        /* the narrow luminance/alpha switch: PowerVR tests start from the
         * expanded store and the narrow ones turn it on (the product default
         * is on); the GC7000 stores narrow always, and its test knob goes
         * back to that */
        #[cfg(feature = "gpu-powervr")]
        crate::pvr::backend::pvr_narrow_la(0);
        #[cfg(all(feature = "gpu-vivante", feature = "test-knobs"))]
        crate::vivante::backend::gc_debug_set_narrow_la(1);
        /* With no table there is nothing to fall back to and the online
         * back end is already the initialiser, so this is a no-op there. */
        #[cfg(not(feature = "no-shader-table"))]
        // SAFETY: the caller's contract: tests are serialised, no GL call runs.
        unsafe {
            crate::shaderc::g_shaderc = &crate::shaderc::offline::SHC_OFFLINE;
        }
    }
}

/// The NUL-terminated GLSL text of a shader the tests name by its table
/// basename (`"passthrough.vert"`), for [`TestGl::shader`].
///
/// With an offline table this is the table's own source row, so the text
/// hashes to the table's blobs. A `no-shader-table` build (the V3D) has no
/// table and no text in it, so the text comes from the files the table is
/// generated from (`tools/shaders/`, see `tools/glsl2usc.sh`). Thirteen of
/// those are `#version 450` sources written for SPIR-V, which no ES 2.0
/// application can write and the online compiler refuses at line 1; for
/// those the build takes their GLSL ES 1.00 twins (`tools/shaders/gl_test_es100/`,
/// the same names, declarations and order; `src/shaderc/es100_twins.rs`
/// checks that each twin reflects exactly as its original).
pub fn shader_source(name: &str) -> &'static str {
    #[cfg(not(feature = "no-shader-table"))]
    {
        let cname = std::ffi::CString::new(name).unwrap();
        // SAFETY: a NUL-terminated name; the result is null or a static NUL-terminated row.
        let p = unsafe { crate::shaderc::offline::gles_offline_shader_source(cname.as_ptr()) };
        assert!(!p.is_null(), "{name}: not in the offline shader table");
        // SAFETY: the row's text is static, NUL-terminated and UTF-8 (it is a `&'static str`).
        let n = unsafe { crate::util::cstr_bytes(p).len() };
        // SAFETY: as above; `n + 1` keeps the NUL.
        core::str::from_utf8(unsafe { core::slice::from_raw_parts(p as *const u8, n + 1) }).unwrap()
    }
    #[cfg(feature = "no-shader-table")]
    {
        macro_rules! src {
            ($name:literal, $path:literal) => {
                ($name, concat!(include_str!(concat!("../tools/shaders/", $path)), "\0"))
            };
        }
        /* Every name a test passes to TestGl::shader or TestGl::program. */
        const SOURCES: &[(&str, &str)] = &[
            src!("passthrough.vert", "gl_test_es100/passthrough.vert"),
            src!("uniform_color.frag", "gl_test_es100/uniform_color.frag"),
            src!("uv.vert", "gl_test_es100/uv.vert"),
            src!("texture.frag", "gl_test_es100/texture.frag"),
            src!("texture2.frag", "gl_test_es100/texture2.frag"),
            src!("mvp.vert", "gl_test_es100/mvp.vert"),
            src!("interp_color.frag", "gl_test_es100/interp_color.frag"),
            src!("color_uv.vert", "gl_test_es100/color_uv.vert"),
            src!("color_uv.frag", "gl_test_es100/color_uv.frag"),
            src!("fvgtest_discard.frag", "fvgtest_discard.frag"),
            src!("main.vert", "femtovg/main.vert"),
            src!("fill_color.frag", "femtovg/fill_color.frag"),
            src!("sprite.vert", "qt/sprite.vert"),
            src!("sprite.frag", "qt/sprite.frag"),
            src!("visualization.vert", "qt/visualization.vert"),
            src!("visualization.frag", "qt/visualization.frag"),
            src!("0119aeb276db508c.frag", "qt3d/0119aeb276db508c.frag"),
            src!("83a830e25f56fd85.vert", "qt3d/83a830e25f56fd85.vert"),
            src!("fd0c8a22ac0eef86.vert", "qt3d/fd0c8a22ac0eef86.vert"),
            src!("cube.vert", "es300/cube.vert"),
            src!("cube.frag", "es300/cube.frag"),
            src!("array.frag", "es300/array.frag"),
            src!("vid.vert", "es300/vid.vert"),
            src!("vcolor.frag", "es300/vcolor.frag"),
            src!("matattr4.vert", "es100/matattr4.vert"),
            src!("matattr.frag", "es100/matattr.frag"),
        ];
        let Some((_, text)) = SOURCES.iter().find(|(n, _)| *n == name) else {
            panic!("{name}: not in testutil::shader_source's list for the no-shader-table build");
        };
        /* The two Qt Quick 3D sources declare `sampler2DArray`, an ES 3.00
         * type under `#version 100` that the offline table never judged and
         * the online compiler refuses. Neither shader samples it, so the
         * build takes the ES 1.00 text with `sampler2D` in its place; the
         * rest of the reflection is the table's. */
        if !(text.contains("sampler2DArray") && text.starts_with("#version 100")) {
            return text;
        }
        static ES100_TEXT: Mutex<BTreeMap<&'static str, &'static str>> = Mutex::new(BTreeMap::new());
        let mut cache = ES100_TEXT.lock().unwrap_or_else(|e| e.into_inner());
        let es100: &'static str = cache
            .entry(text)
            .or_insert_with(|| Box::leak(text.replace("sampler2DArray", "sampler2D").into_boxed_str()));
        es100
    }
}

/// Make `stage` read as one the OFFLINE table compiled: no record of the
/// build's own compiler behind it (`shaderc::is_online` is then false), so
/// the GL core reads its uniform rows in the table's shape (one flattened
/// row per struct-array member, `program::row_is_flattened`). With a table
/// the stages the tests link already are that; in a `no-shader-table` build
/// every stage is the compiler's, and a test that hands the core the table's
/// reflection shape marks the stage it hand-built. The stage's own record
/// stays in the keep region until the next reset.
pub fn mark_stage_offline(stage: &mut crate::shaderc::ShcShader) {
    stage.priv_ = core::ptr::null();
    assert!(!crate::shaderc::is_online(stage), "a stage without a record is not the compiler's");
}

/// A fresh GL context on the mocks, current for the guard's lifetime.
pub struct TestGl {
    ctx: *mut GlContext,
    _serial: MutexGuard<'static, ()>,
}

impl TestGl {
    /// Install the mocks and create + make current a context with an owned
    /// `w` x `h` RGBA8888 default surface.
    pub fn new(w: u32, h: u32) -> TestGl {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        HOLDS_SERIAL.with(|c| c.set(true));
        /* a line another test logged first through glog_once! logs again */
        crate::osal::new_log_epoch();
        *MOCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Mock::default());
        // SAFETY: tests are serialised by SERIAL; the tables are statics.
        let ctx = unsafe {
            reset_library_statics();
            crate::osal::osal_set_ops(&MOCK_OSAL);
            crate::backend::g_backend = &MOCK_BACKEND;
            context::gl_context_create(w, h)
        };
        assert!(!ctx.is_null(), "context pool exhausted");
        let gl = TestGl { ctx, _serial: serial };
        gl.clear_recordings();
        gl
    }

    /// Panic with the faults the mock recorded (see the module documentation).
    fn check(&self) {
        let faults = with_mock(|m| core::mem::take(&mut m.faults));
        if !faults.is_empty() {
            panic!("mock OSAL/backend faults:\n  {}", faults.join("\n  "));
        }
    }

    /// Take the recorded faults instead of failing on them (for tests that
    /// prove a fault is detected).
    pub fn take_faults(&self) -> Vec<String> {
        with_mock(|m| core::mem::take(&mut m.faults))
    }

    /// The context (for white-box assertions). Do not hold the reference
    /// across GL calls.
    #[allow(clippy::mut_from_ref)]
    pub fn ctx(&self) -> &mut GlContext {
        // SAFETY: the context lives as long as the guard; tests are single-threaded.
        unsafe { &mut *self.ctx }
    }

    /// glGetError().
    pub fn error(&self) -> GLenum {
        self.check();
        let c = self.ctx();
        core::mem::replace(&mut c.error, GL_NO_ERROR)
    }

    /// Forget recorded draws, memory events, cache ops and flush/finish counts.
    pub fn clear_recordings(&self) {
        with_mock(|m| {
            m.draws.clear();
            m.events.clear();
            m.cache_ops.clear();
            m.flushes = 0;
            m.finishes = 0;
            m.waited_kicks.clear();
        });
    }

    /// Take the draws recorded so far.
    pub fn draws(&self) -> Vec<RecordedDraw> {
        self.check();
        with_mock(|m| core::mem::take(&mut m.draws))
    }

    /// Take the backend's surface_swapped calls so far: (surface, flushes
    /// before the call).
    pub fn swaps(&self) -> Vec<(u64, u32)> {
        with_mock(|m| core::mem::take(&mut m.swaps))
    }

    /// Take the backend's attachment_gone calls so far (the ids, in order).
    pub fn gones(&self) -> Vec<u32> {
        with_mock(|m| core::mem::take(&mut m.gones))
    }

    /// Take the backend's texture_uploaded calls so far.
    pub fn uploads(&self) -> Vec<Upload> {
        with_mock(|m| core::mem::take(&mut m.uploads))
    }

    pub fn flushes(&self) -> u32 {
        with_mock(|m| m.flushes)
    }

    pub fn finishes(&self) -> u32 {
        with_mock(|m| m.finishes)
    }

    /// Take the lines logged so far (glog!).
    pub fn logs(&self) -> Vec<String> {
        with_mock(|m| core::mem::take(&mut m.logs))
    }

    /// Take the count of vertical periods waited out since the last call.
    pub fn vblanks(&self) -> u32 {
        self.check();
        VBLANKS.swap(0, core::sync::atomic::Ordering::Relaxed)
    }

    /// Whether the mock platform has a vertical-sync source at all: without
    /// one the op answers -ENOTSUP and presentation is immediate.
    pub fn set_vblank_source(&self, present: bool) {
        self.check();
        HAS_VBLANK.store(present, core::sync::atomic::Ordering::Relaxed);
    }

    /// Take the allocation events recorded so far.
    pub fn mem_events(&self) -> Vec<MemEvent> {
        self.check();
        with_mock(|m| core::mem::take(&mut m.events))
    }

    /// Take the cache_flush / cache_invd calls recorded so far.
    pub fn cache_ops(&self) -> Vec<CacheOp> {
        self.check();
        with_mock(|m| core::mem::take(&mut m.cache_ops))
    }

    /// Number of live GPU allocations (freed ones the GPU may still read
    /// do not count).
    pub fn live_allocs(&self) -> usize {
        self.check();
        with_mock(|m| m.allocs.len())
    }

    /// Bytes held by live GPU allocations (as `live_allocs`, freed ones the
    /// GPU may still read do not count).
    pub fn live_bytes(&self) -> usize {
        self.check();
        with_mock(|m| m.allocs.values().map(|a| a.size).sum())
    }

    /// Bytes the pool cannot hand out now (page-rounded): live allocations
    /// and frees the OSAL still holds (see `Mock::held`).
    pub fn held_bytes(&self) -> usize {
        self.check();
        with_mock(|m| m.pool_bytes())
    }

    /// Give the pool `bytes` (`None`: unbounded): past it, gpu_alloc fails
    /// with -ENOMEM.
    pub fn set_pool_budget(&self, bytes: Option<usize>) {
        with_mock(|m| m.budget = bytes);
    }

    /// The OSAL's CONFIG_YAKOGL_PVR_POOL_RECLAIM arm on or off (see
    /// `Mock::reclaim`).
    pub fn set_osal_reclaim(&self, on: bool) {
        with_mock(|m| m.reclaim = on);
    }

    /// Make the n-th next gpu_alloc fail with -ENOMEM (1 = the next one).
    pub fn fail_alloc_in(&self, n: u32) {
        with_mock(|m| m.fail_alloc_in = n);
    }

    /// Make the n-th next gpu_write fail with -EIO (1 = the next one).
    pub fn fail_gpu_write_in(&self, n: u32) {
        with_mock(|m| m.fail_write_in = n);
    }

    /// Make the n-th next gpu_read fail with -EIO (1 = the next one).
    pub fn fail_gpu_read_in(&self, n: u32) {
        with_mock(|m| m.fail_read_in = n);
    }

    /// Every gpu_write / gpu_read fails with -EIO from now on (or not).
    pub fn set_gpu_io_fails(&self, fail: bool) {
        with_mock(|m| m.io_fails = fail);
    }

    /// gpu_free refuses with `code` (a negative errno) from now on, as an
    /// OSAL with a full pending-free queue does; 0 accepts again.
    pub fn set_free_result(&self, code: i32) {
        with_mock(|m| m.free_result = code);
    }

    /// The OSAL answers OSAL_QUERY_CLOCK_NS from now on, starting at
    /// `start` ns and moving `step` ns a read (`None`: the OSAL has no query
    /// again, the default).
    pub fn set_clock(&self, clock: Option<(u64, u64)>) {
        with_mock(|m| m.clock = clock.unwrap_or((0, 0)));
        // SAFETY: serialised by SERIAL (this TestGl holds it); the tables are statics.
        unsafe { crate::osal::osal_set_ops(if clock.is_some() { &MOCK_OSAL_CLOCK } else { &MOCK_OSAL }) };
    }

    /// The draw stats the OSAL received through [`TestGl::set_clock`]'s
    /// query since the last call.
    pub fn draw_stats_taken(&self) -> Vec<crate::draw_steps::OsalDrawStats> {
        with_mock(|m| core::mem::take(&mut m.draw_stats))
    }

    /// Fresh allocations are zeroed (`true`) or filled with [`FILL`] (the default).
    pub fn set_zero_fill(&self, zero: bool) {
        with_mock(|m| m.zero_fill = zero);
    }

    /// Allocations made from now on come with a CPU mapping (the default) or
    /// without one (`cpu` null: the core must use gpu_write / gpu_read).
    pub fn set_cpu_mapping(&self, mapped: bool) {
        with_mock(|m| m.cpu_map = mapped);
    }

    /// Do not report draws that read memory nobody wrote (a test that draws
    /// with undefined contents on purpose).
    pub fn allow_uninit_reads(&self, allow: bool) {
        with_mock(|m| m.allow_uninit = allow);
    }

    /* ---- kicks ---- */

    /// The kick being recorded (what the backend's kick_id() returns).
    pub fn kick_id(&self) -> u32 {
        with_mock(|m| m.kick_id())
    }

    /// What the backend's kick_pending(id) returns.
    pub fn kick_pending(&self, id: u32) -> bool {
        with_mock(|m| m.kick_pending(id))
    }

    /// The kicks submitted so far.
    pub fn submitted(&self) -> u32 {
        with_mock(|m| m.last_fence)
    }

    /// The GPU completes every submitted kick up to `through`.
    pub fn complete_kicks(&self, through: u32) {
        with_mock(|m| m.complete(through));
        self.check();
    }

    /// Complete every submitted kick automatically `lag` submits after its
    /// own (0: at its submit); `None`: only when waited for or completed by
    /// the test (the default).
    pub fn set_kick_lag(&self, lag: Option<u32>) {
        with_mock(|m| {
            m.lag = lag;
            if let Some(l) = lag {
                m.complete(m.last_fence.saturating_sub(l));
            }
        });
    }

    /// Report kick `id` as still running (`kick_pending`), or not, whatever
    /// the kick model says.
    /// Answer the backend capability questions the way the GC7000 does: it
    /// fetches every attribute source format, stores an RGBA8888 target as
    /// B,G,R,A, and reads a mip chain at one row pitch with every level on a
    /// 64-byte boundary ([`MOCK_BACKEND_GC7000`]).
    pub fn backend_like_gc7000(&self) {
        // SAFETY: tests are serialised by SERIAL; both tables are statics
        // that outlive the test.
        unsafe { crate::backend::g_backend = &MOCK_BACKEND_GC7000 };
    }

    /// Answer as a backend whose texture unit has no ETC1: the GL core
    /// decodes ETC1 at upload ([`MOCK_BACKEND_ETC1_DECODED`]).
    pub fn backend_decodes_etc1(&self) {
        // SAFETY: tests are serialised by SERIAL; the table is a static
        // that outlives the test.
        unsafe { crate::backend::g_backend = &MOCK_BACKEND_ETC1_DECODED };
    }

    /// Install the mock backend variant that takes instanced arrays of any
    /// divisor up to 0xffff as one draw ([`MOCK_BACKEND_DIVISORS`]).
    pub fn backend_takes_divisors(&self) {
        // SAFETY: tests are serialised by SERIAL; the table is a static
        // that outlives the test.
        unsafe { crate::backend::g_backend = &MOCK_BACKEND_DIVISORS };
    }

    /// Install the mock backend variant that waits for one kick
    /// ([`MOCK_BACKEND_WAIT_KICK`]).
    pub fn backend_waits_for_kicks(&self) {
        // SAFETY: tests are serialised by SERIAL; the table is a static
        // that outlives the test.
        unsafe { crate::backend::g_backend = &MOCK_BACKEND_WAIT_KICK };
    }

    /// Install the mock backend variant that samples texel copies
    /// ([`MOCK_BACKEND_TEXEL_COPIES`]).
    pub fn backend_samples_texel_copies(&self) {
        // SAFETY: tests are serialised by SERIAL; the table is a static
        // that outlives the test.
        unsafe { crate::backend::g_backend = &MOCK_BACKEND_TEXEL_COPIES };
    }

    /// Take the backend's depth_resolve calls so far.
    pub fn depth_resolves(&self) -> Vec<DepthResolve> {
        with_mock(|m| core::mem::take(&mut m.depth_resolves))
    }

    /// What the mock's depth_resolve writes into every texel (`None`: it
    /// holds nothing and answers 1).
    pub fn set_resolve_depth(&self, v: Option<f32>) {
        with_mock(|m| m.resolve_depth = v);
    }

    /// The kicks wait_kick was called for, in order.
    pub fn waited_kicks(&self) -> Vec<u32> {
        with_mock(|m| m.waited_kicks.clone())
    }

    pub fn set_kick_pending(&self, id: u32, pending: bool) {
        with_mock(|m| {
            m.overrides.insert(id, pending);
        });
    }

    /// What the mock backend's draw() returns from now on (0 = success; a
    /// failed draw is recorded but opens no frame).
    pub fn set_draw_result(&self, rc: i32) {
        with_mock(|m| m.draw_result = rc);
    }

    /// What the mock backend's flush() returns from now on (0 = success; a
    /// failed flush submits nothing).
    pub fn set_flush_result(&self, rc: i32) {
        with_mock(|m| m.flush_result = rc);
    }

    /// What the mock backend's finish() / finish_previous() return from now on.
    pub fn set_finish_result(&self, rc: i32) {
        with_mock(|m| m.finish_result = rc);
    }

    /* ---- memory ---- */

    /// `len` bytes of device memory at `gpu` (inside one allocation, live or
    /// freed: the mock keeps freed memory): what the GPU sees.
    pub fn gpu_bytes(&self, gpu: u64, len: usize) -> Vec<u8> {
        self.check();
        with_mock(|m| {
            let (base, off) = match m.place(gpu) {
                Place::Live(b, o) | Place::Zombie(b, o, _) => (b, o),
                _ => panic!("gpu_bytes: no allocation at 0x{gpu:x}"),
            };
            let a = m.alloc_any(base).unwrap();
            assert!(off + len <= a.size, "gpu_bytes: range leaves the allocation");
            a.dev[off..off + len].to_vec()
        })
    }

    /// `len` bytes of the CPU view at `gpu` (what a CPU read sees, stale or not).
    pub fn cpu_view(&self, gpu: u64, len: usize) -> Vec<u8> {
        with_mock(|m| {
            let Place::Live(base, off) = m.place(gpu) else { panic!("cpu_view: no live allocation at 0x{gpu:x}") };
            m.allocs[&base].cpu_view[off..off + len].to_vec()
        })
    }

    /// The GPU writes `bytes` at `gpu` (rendering): see [`gpu_render`].
    pub fn gpu_render(&self, gpu: u64, bytes: &[u8]) {
        gpu_render(gpu, bytes);
    }

    /// The first byte of `[gpu, gpu + len)` whose CPU write was not flushed.
    pub fn unflushed(&self, gpu: u64, len: usize) -> Option<usize> {
        with_mock(|m| match m.place(gpu) {
            Place::Live(b, o) => m.allocs[&b].unflushed(o, len).map(|i| i - o),
            _ => panic!("unflushed: no live allocation at 0x{gpu:x}"),
        })
    }

    /// The heap an allocation (live or freed) was made in.
    pub fn heap_of(&self, gpu: u64) -> Option<u64> {
        with_mock(|m| match m.place(gpu) {
            Place::Live(b, _) | Place::Zombie(b, _, _) => m.alloc_any(b).map(|a| a.heap),
            _ => None,
        })
    }

    /// Compile one of the tests' named shaders ([`shader_source`]) through
    /// glShaderSource + glCompileShader, as an application would, with the
    /// build's own shader back end (`g_shaderc`: the offline table, or the
    /// online compiler in a `no-shader-table` build). A compile the back end
    /// refuses fails the test with the back end's info log.
    pub fn shader(&self, type_: GLenum, name: &str) -> GLuint {
        let src = shader_source(name);
        // SAFETY: `src` is NUL-terminated and static; the GL calls follow the API contract.
        unsafe {
            let s = crate::gl::shader::glCreateShader(type_);
            let srcs: [*const c_char; 1] = [src.as_ptr() as *const c_char];
            crate::gl::shader::glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
            crate::gl::shader::glCompileShader(s);
            let mut ok: GLint = 0;
            crate::gl::shader::glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            if ok != 1 {
                let mut log = [0u8; 1024];
                let mut len: GLsizei = 0;
                crate::gl::shader::glGetShaderInfoLog(
                    s,
                    log.len() as GLsizei,
                    &mut len,
                    log.as_mut_ptr() as *mut c_char,
                );
                let log = String::from_utf8_lossy(&log[..len.max(0) as usize]);
                panic!("{name}: compile failed: {log}");
            }
            s
        }
    }

    /// Compile, link and return a program from two named shaders ([`TestGl::shader`]).
    pub fn program(&self, vs: &str, fs: &str) -> GLuint {
        let v = self.shader(GL_VERTEX_SHADER, vs);
        let f = self.shader(GL_FRAGMENT_SHADER, fs);
        // SAFETY: plain GL calls on valid handles.
        unsafe {
            let p = crate::gl::program::glCreateProgram();
            crate::gl::program::glAttachShader(p, v);
            crate::gl::program::glAttachShader(p, f);
            crate::gl::program::glLinkProgram(p);
            let mut ok: GLint = 0;
            crate::gl::program::glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "{vs} + {fs}: link failed");
            p
        }
    }
}

impl Drop for TestGl {
    fn drop(&mut self) {
        // SAFETY: the context is ours; tests are serialised.
        unsafe {
            context::gl_context_destroy(self.ctx);
            /* the draw path's grow-only staging points into the mock's memory */
            crate::gl::draw::reset_for_tests(true);
            context::gl_context_set_current(core::ptr::null_mut());
        }
        /* the GPU runs what was submitted */
        with_mock(|m| m.complete(m.last_fence));
        let faults = with_mock(|m| core::mem::take(&mut m.faults));
        // SAFETY: as above.
        unsafe {
            reset_library_statics();
            crate::backend::g_backend = core::ptr::null();
            crate::osal::osal_set_ops(core::ptr::null());
        }
        *MOCK.lock().unwrap_or_else(|e| e.into_inner()) = None;
        HOLDS_SERIAL.with(|c| c.set(false));
        if !faults.is_empty() && !std::thread::panicking() {
            panic!("mock OSAL/backend faults:\n  {}", faults.join("\n  "));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osal;

    #[test]
    fn context_lifecycle_on_the_mocks() {
        {
            let gl = TestGl::new(64, 32);
            let c = gl.ctx();
            assert_eq!(c.viewport, [0, 0, 64, 32]);
            assert!(c.default_surface_valid && c.default_surface_owned);
            assert_eq!(c.default_surface.size, 64 * 32 * 4);
            assert_eq!(gl.live_allocs(), 1);
            assert_eq!(gl.error(), GL_NO_ERROR);
            c.set_error(GL_INVALID_VALUE, "glTest");
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            assert!(gl.logs().iter().any(|l| l.contains("in glTest")));
        }
        // the guard released the pool slot: the pool holds MAX_CONTEXTS, so
        // more rounds than that prove nothing leaks
        for _ in 0..context::MAX_CONTEXTS + 1 {
            let gl = TestGl::new(8, 8);
            assert_eq!(gl.live_allocs(), 1);
        }
    }

    #[test]
    fn draw_is_recorded_with_its_uniforms() {
        let gl = TestGl::new(64, 64);
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls; the client vertex array outlives the draw.
        unsafe {
            use crate::gl::{draw::*, program::*, state::*, vertex::*};
            let verts: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
            glUseProgram(prog);
            let loc = glGetUniformLocation(prog, c"u_color".as_ptr());
            assert!(loc >= 0);
            glUniform4f(loc, 0.25, 0.5, 0.75, 1.0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        let d = &draws[0];
        assert!(d.draw.do_clear, "the pending glClear is fused into the draw");
        assert_eq!(d.draw.clear_rgba, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(d.draw.index_count, 3);
        assert_eq!((d.draw.target_w, d.draw.target_h), (64, 64));
        let want: Vec<u32> = [0.25f32, 0.5, 0.75, 1.0].iter().map(|f| f.to_bits()).collect();
        assert_eq!(d.fs_uniforms, want);
        assert_eq!(d.kick, 1, "the first frame");
    }

    fn alloc(size: usize) -> OsalGpuMem {
        let mut m = OsalGpuMem::ZERO;
        assert_eq!(osal::gpu_alloc(HEAP_DATA, size, 0, &mut m), 0);
        m
    }

    /* ---- the mock itself ---- */

    /// A free the OSAL refuses (a full pending-free queue, src/osal/pending_frees.h)
    /// is counted by osal::gpu_free and the first one is logged; the
    /// descriptor is cleared either way (the GL object is gone).
    #[test]
    fn a_refused_free_is_counted_and_the_first_is_logged() {
        let gl = TestGl::new(8, 8);
        gl.logs();
        let before = osal::refused_frees();
        let mut a = alloc(8);
        let mut b = alloc(8);
        gl.set_free_result(-crate::util::ETIMEDOUT);
        osal::gpu_free(&mut a);
        osal::gpu_free(&mut b);
        gl.set_free_result(0);
        assert_eq!(osal::refused_frees() - before, 2, "every refusal is counted");
        assert!(a.gpu == 0 && a.cpu.is_null() && a.size == 0 && a.impl_.is_null(), "{a:?}");
        let logs = gl.logs();
        let refused = std::format!("was refused ({})", -crate::util::ETIMEDOUT);
        let logged = logs.iter().filter(|l| l.contains(refused.as_str())).count();
        /* only this test refuses; the line is the process's first refusal */
        assert_eq!(logged, usize::from(before == 0), "the first refusal is logged, once: {logs:?}");
    }

    #[test]
    fn fresh_memory_is_filled_unless_zeroing_is_asked_for() {
        let gl = TestGl::new(8, 8);
        let mut a = alloc(8);
        assert_eq!(gl.gpu_bytes(a.gpu, 8), [FILL; 8]);
        // SAFETY: a live CPU-mapped allocation of 8 bytes.
        assert_eq!(unsafe { a.bytes_mut() }.unwrap(), [FILL; 8]);
        gl.set_zero_fill(true);
        let mut z = alloc(8);
        assert_eq!(gl.gpu_bytes(z.gpu, 8), [0; 8]);
        osal::gpu_free(&mut a);
        osal::gpu_free(&mut z);
    }

    #[test]
    fn the_gpu_sees_cpu_writes_only_after_a_flush() {
        let gl = TestGl::new(8, 8);
        let mut a = alloc(16);
        // SAFETY: a live CPU-mapped allocation of 16 bytes.
        let b = unsafe { a.bytes_mut() }.unwrap();
        b[4..8].copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(gl.gpu_bytes(a.gpu + 4, 4), [FILL; 4]);
        assert_eq!(gl.unflushed(a.gpu, 16), Some(4));
        // SAFETY: inside the mapping.
        unsafe { osal::cache_flush((a.cpu as *mut u8).add(4) as *mut c_void, 2) };
        assert_eq!(gl.gpu_bytes(a.gpu + 4, 4), [1, 2, FILL, FILL]);
        assert_eq!(gl.unflushed(a.gpu, 16), Some(6));
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu: a.gpu, off: 4, len: 2 }]);
        // SAFETY: inside the mapping.
        unsafe { osal::cache_flush(a.cpu, 16) };
        assert_eq!(gl.unflushed(a.gpu, 16), None);
        osal::gpu_free(&mut a);
    }

    #[test]
    fn the_cpu_sees_gpu_writes_only_after_an_invalidate() {
        let gl = TestGl::new(8, 8);
        let mut a = alloc(8);
        gl.gpu_render(a.gpu, &[9; 8]);
        assert_eq!(gl.cpu_view(a.gpu, 8), [FILL; 8]);
        // SAFETY: inside the mapping.
        unsafe { osal::cache_invd(a.cpu, 8) };
        // SAFETY: a live CPU-mapped allocation.
        assert_eq!(unsafe { a.bytes_mut() }.unwrap(), [9; 8]);
        assert_eq!(gl.cache_ops(), [CacheOp::Invd { gpu: a.gpu, off: 0, len: 8 }]);
        osal::gpu_free(&mut a);
    }

    #[test]
    fn an_invalidate_over_an_unflushed_write_is_a_fault() {
        let gl = TestGl::new(8, 8);
        let mut a = alloc(8);
        // SAFETY: a live CPU-mapped allocation.
        unsafe { a.bytes_mut() }.unwrap()[3] = 1;
        // SAFETY: inside the mapping.
        unsafe { osal::cache_invd(a.cpu, 8) };
        let f = gl.take_faults();
        assert!(f[0].contains("discards an unflushed CPU write"), "{f:?}");
        osal::gpu_free(&mut a);
    }

    #[test]
    fn cache_calls_outside_live_allocations_are_faults() {
        let gl = TestGl::new(8, 8);
        let mut a = alloc(8);
        let mut local = [0u8; 4];
        // SAFETY: the mock never dereferences memory outside its allocations.
        unsafe {
            osal::cache_flush((a.cpu as *mut u8).add(4) as *mut c_void, 8);
            osal::cache_flush(local.as_mut_ptr() as *mut c_void, 4);
        }
        let cpu = a.cpu;
        osal::gpu_free(&mut a);
        // SAFETY: the freed allocation's memory is kept by the mock.
        unsafe { osal::cache_invd(cpu, 8) };
        let f = gl.take_faults();
        assert_eq!(f.len(), 3, "{f:?}");
        assert!(f[0].contains("leaves the allocation"));
        assert!(f[1].contains("not a GPU allocation"));
        assert!(f[2].contains("freed allocation"));
    }

    #[test]
    fn double_frees_are_detected_also_through_a_copy() {
        let gl = TestGl::new(8, 8);
        let mut a = alloc(8);
        let mut copy = a;
        osal::gpu_free(&mut a);
        osal::gpu_free(&mut a); /* cleared: a no-op */
        assert!(gl.take_faults().is_empty());
        osal::gpu_free(&mut copy);
        let f = gl.take_faults();
        assert!(f[0].starts_with("double free"), "{f:?}");
    }

    #[test]
    #[should_panic(expected = "double free")]
    fn a_double_free_fails_the_test_at_the_next_check() {
        let gl = TestGl::new(8, 8);
        let mut a = alloc(8);
        let mut copy = a;
        osal::gpu_free(&mut a);
        osal::gpu_free(&mut copy);
        gl.error();
    }

    #[test]
    fn freeing_what_the_mock_never_allocated_is_a_fault() {
        let gl = TestGl::new(8, 8);
        let mut forged = OsalGpuMem { gpu: HEAP_DATA + 0x10_0000, size: 4, ..OsalGpuMem::ZERO };
        osal::gpu_free(&mut forged);
        let mut a = alloc(8);
        let mut wrong = a;
        wrong.impl_ = core::ptr::without_provenance_mut(1);
        osal::gpu_free(&mut wrong);
        let f = gl.take_faults();
        assert!(f[0].contains("never handed out"), "{f:?}");
        assert!(f[1].contains("not the allocation's"), "{f:?}");
        a = OsalGpuMem::ZERO;
        osal::gpu_free(&mut a);
    }

    #[test]
    fn allocations_without_a_cpu_view_use_gpu_write_and_gpu_read() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        let mut a = alloc(8);
        assert!(a.cpu.is_null());
        let ops = osal::ops().unwrap();
        let mut out = [0u8; 3];
        // SAFETY: valid descriptor and buffers.
        unsafe {
            assert_eq!(ops.gpu_write.unwrap()(&mut a, 2, [1u8, 2, 3].as_ptr() as *const c_void, 3), 0);
            assert_eq!(ops.gpu_read.unwrap()(&mut a, 2, out.as_mut_ptr() as *mut c_void, 3), 0);
            assert_eq!(out, [1, 2, 3]);
            assert_eq!(gl.gpu_bytes(a.gpu, 6), [FILL, FILL, 1, 2, 3, FILL]);
            /* out of range: refused and reported */
            assert_ne!(ops.gpu_write.unwrap()(&mut a, 6, out.as_ptr() as *const c_void, 3), 0);
            assert!(gl.take_faults()[0].contains("leaves the allocation"));
            /* injected failures */
            gl.fail_gpu_write_in(2);
            assert_eq!(ops.gpu_write.unwrap()(&mut a, 0, out.as_ptr() as *const c_void, 1), 0);
            assert_eq!(ops.gpu_write.unwrap()(&mut a, 0, out.as_ptr() as *const c_void, 1), -crate::util::EIO);
            assert_eq!(ops.gpu_write.unwrap()(&mut a, 0, out.as_ptr() as *const c_void, 1), 0);
            gl.fail_gpu_read_in(1);
            assert_eq!(ops.gpu_read.unwrap()(&mut a, 0, out.as_mut_ptr() as *mut c_void, 1), -crate::util::EIO);
            gl.set_gpu_io_fails(true);
            assert_eq!(ops.gpu_read.unwrap()(&mut a, 0, out.as_mut_ptr() as *mut c_void, 1), -crate::util::EIO);
        }
        let mut stale = a;
        osal::gpu_free(&mut a);
        gl.set_gpu_io_fails(false);
        // SAFETY: as above.
        unsafe { ops.gpu_write.unwrap()(&mut stale, 0, out.as_ptr() as *const c_void, 1) };
        assert!(gl.take_faults()[0].contains("freed allocation"));
    }

    #[test]
    fn the_kick_model_follows_the_backend() {
        let gl = TestGl::new(8, 8);
        let _prog = gl.program("passthrough.vert", "uniform_color.frag");
        assert_eq!(gl.kick_id(), 1);
        assert!(gl.kick_pending(1), "the frame being recorded is pending");
        assert!(!gl.kick_pending(0) && !gl.kick_pending(2), "0 and future ids are not");
        // SAFETY: the backend entry points take no arguments.
        unsafe {
            assert_eq!(b_flush(), 0);
            assert_eq!(gl.submitted(), 0, "an empty frame is not submitted");
        }
        let f = [0.0f32; 12];
        // SAFETY: plain GL calls on a client array that outlives them.
        unsafe {
            use crate::gl::{draw::*, program::*, vertex::*};
            glUseProgram(_prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, f.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.kick_id(), 1);
            assert_eq!(b_flush(), 0);
            assert_eq!((gl.submitted(), gl.kick_id()), (1, 2));
            assert!(gl.kick_pending(1), "submitted, not completed");
            glDrawArrays(GL_TRIANGLES, 0, 3);
            b_flush();
            assert!(b_finish_previous() == 0 && !gl.kick_pending(1) && gl.kick_pending(2));
            gl.set_kick_pending(1, true);
            assert!(gl.kick_pending(1), "an override wins");
            gl.set_kick_pending(1, false);
            assert_eq!(b_finish(), 0);
            assert!(!gl.kick_pending(2));
            gl.set_kick_lag(Some(0));
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert!(gl.kick_pending(3));
            b_flush();
            assert!(!gl.kick_pending(3), "lag 0: completed at submit");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_draw_on_another_target_submits_the_open_frame() {
        let gl = TestGl::new(8, 8);
        let mut other = alloc(16);
        let mut d = GlesDraw::ZERO;
        d.target_gpu = gl.ctx().default_surface.gpu;
        d.target_w = 2;
        d.target_h = 2;
        gl.gpu_render(d.target_gpu, &[0; 16]);
        gl.gpu_render(other.gpu, &[0; 16]);
        // SAFETY: a valid draw without staging.
        unsafe {
            b_draw(&d);
            assert_eq!(gl.submitted(), 0);
            b_draw(&d);
            assert_eq!(gl.submitted(), 0);
            d.target_gpu = other.gpu;
            b_draw(&d);
        }
        assert_eq!((gl.submitted(), gl.kick_id()), (1, 2));
        assert_eq!(gl.draws().iter().map(|r| r.kick).collect::<Vec<_>>(), [1, 1, 2]);
        osal::gpu_free(&mut other);
    }

    #[test]
    fn draw_inputs_are_checked_against_the_memory_model() {
        let gl = TestGl::new(8, 8);
        let mut vb = alloc(64);
        let mut d = GlesDraw::ZERO;
        d.target_gpu = gl.ctx().default_surface.gpu;
        d.target_w = 8;
        d.target_h = 8;
        d.index_count = 3;
        d.num_attribs = 1;
        d.attribs[0].components = 4;
        d.attribs[0].stride = 16;
        d.attribs[0].buffer_gpu = vb.gpu;
        // SAFETY: a valid draw; the mapping is live.
        unsafe {
            /* nobody wrote the vertices */
            b_draw(&d);
            assert!(gl.take_faults()[0].contains("memory nobody wrote"));
            /* written, not flushed */
            vb.bytes_mut().unwrap()[..48].fill(1);
            b_draw(&d);
            assert!(gl.take_faults()[0].contains("not flushed"));
            osal::cache_flush(vb.cpu, 48);
            b_draw(&d);
            assert!(gl.take_faults().is_empty());
            /* changed before the kick ran */
            vb.bytes_mut().unwrap()[0] = 2;
            osal::cache_flush(vb.cpu, 1);
            b_flush();
            let f = gl.take_faults();
            assert!(!f.is_empty() && f.iter().all(|f| f.contains("changed") && f.contains("submitted")), "{f:?}");
            b_finish();
            let f = gl.take_faults();
            assert!(!f.is_empty() && f.iter().all(|f| f.contains("changed") && f.contains("completed")), "{f:?}");
            /* one vertex too many: past the allocation */
            d.index_count = 5;
            b_draw(&d);
            assert!(gl.take_faults()[0].contains("past the allocation"));
            d.attribs[0].buffer_gpu = 0x1234;
            b_draw(&d);
            assert!(gl.take_faults()[0].contains("not GPU memory"));
        }
        gl.draws();
        osal::gpu_free(&mut vb);
    }

    #[test]
    fn freed_memory_stays_readable_until_the_recording_kick_completes() {
        let gl = TestGl::new(8, 8);
        let mut vb = alloc(16);
        gl.gpu_render(vb.gpu, &[1; 16]);
        let mut d = GlesDraw::ZERO;
        d.target_gpu = gl.ctx().default_surface.gpu;
        d.index_count = 1;
        d.num_attribs = 1;
        d.attribs[0].components = 4;
        d.attribs[0].buffer_gpu = vb.gpu;
        let gpu = vb.gpu;
        // SAFETY: a valid draw.
        unsafe { b_draw(&d) };
        osal::gpu_free(&mut vb);
        assert_eq!(gl.live_allocs(), 1);
        assert_eq!(gl.gpu_bytes(gpu, 16), [1; 16], "the pending kick still reads it");
        // SAFETY: as above.
        unsafe {
            b_flush();
            b_finish();
            /* completed: a draw that reads it now reads freed memory */
            b_draw(&d);
        }
        assert!(gl.take_faults()[0].contains("freed memory"));
    }

    #[test]
    fn map_phys_mappings_are_coherent_and_freed_once() {
        let gl = TestGl::new(8, 8);
        let mut px = [0u8; 16];
        mock_map_phys(0x90_0000_0000, 16, px.as_mut_ptr() as *mut c_void);
        let mut m =
            OsalGpuMem { cpu: px.as_mut_ptr() as *mut c_void, gpu: 0x90_0000_0000, size: 16, ..OsalGpuMem::ZERO };
        let mut copy = m;
        // SAFETY: inside the test's own buffer.
        unsafe { osal::cache_flush(m.cpu, 16) };
        assert!(gl.cache_ops().is_empty());
        osal::gpu_free(&mut m);
        assert!(gl.take_faults().is_empty());
        osal::gpu_free(&mut copy);
        assert!(gl.take_faults()[0].starts_with("double free"));
    }

    #[test]
    fn the_log_hook_receives_formatted_lines() {
        let gl = TestGl::new(8, 8);
        crate::glog!("line {} of {}", 1, "two");
        assert_eq!(gl.logs(), ["line 1 of two"]);
    }

    #[test]
    fn a_new_guard_starts_from_pristine_library_statics() {
        {
            let gl = TestGl::new(8, 8);
            /* leave every observable process-global dirty, as a panicking test would */
            // SAFETY: plain GL / EGL calls and the library's own debug hooks; tests are serialised.
            unsafe {
                crate::gl::draw::gl_debug_set_attrib_dump(1);
                for _ in 0..20 {
                    gl.ctx().set_error(GL_INVALID_ENUM, "glDirty"); /* spends the error-log budget */
                }
                let leaked = context::gl_context_create(4, 4); /* never destroyed */
                assert!(!leaked.is_null());
                context::gl_context_set_current(gl.ctx());
                /* after the switches, which carry a pending clear out */
                let p = crate::gl::draw::pending_clear();
                p.color = true;
                p.depth_value = 0.25;
                let dpy = crate::gl::egl::eglGetDisplay(core::ptr::null_mut());
                crate::gl::egl::eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut());
                crate::gl::buffer::gl_debug_bind_hist(4, 99);
                crate::gl::framebuffer::glBlitFramebuffer(0, 0, 1, 1, 0, 0, 1, 1, GL_COLOR_BUFFER_BIT, GL_NEAREST);
                crate::gl::texture::glTexImage3D(0, 0, 0, 0, 0, 0, 0, 0, 0, core::ptr::null());
            }
            gl.logs();
            gl.error();
        }
        let gl = TestGl::new(8, 8);
        // SAFETY: tests are serialised; plain reads and GL calls.
        unsafe {
            assert_eq!(core::ptr::addr_of!(crate::gl::draw::gl_debug_attrib_dump).read(), 0);
            let p = crate::gl::draw::pending_clear();
            assert!(!p.color && p.depth_value == 1.0);
            /* EGL is terminated */
            let mut n = 0;
            let dpy = crate::gl::egl::eglGetDisplay(core::ptr::null_mut());
            assert_eq!(crate::gl::egl::eglGetConfigs(dpy, core::ptr::null_mut(), 0, &mut n), 0);
            assert_eq!(crate::gl::egl::eglGetError(), crate::gl::egl_consts::EGL_NOT_INITIALIZED);
            /* the pool has room for every context again */
            let others: Vec<_> = (1..context::MAX_CONTEXTS).map(|_| context::gl_context_create(4, 4)).collect();
            assert!(others.iter().all(|c| !c.is_null()), "a leaked context still holds a pool slot");
            for c in others {
                context::gl_context_destroy(c);
            }
            context::gl_context_set_current(gl.ctx());
            /* the one-shot logs fire again */
            crate::gl::framebuffer::glBlitFramebuffer(0, 0, 1, 1, 0, 0, 1, 1, GL_COLOR_BUFFER_BIT, GL_NEAREST);
            crate::gl::texture::glTexImage3D(0, 0, 0, 0, 0, 0, 0, 0, 0, core::ptr::null());
        }
        let logs = gl.logs();
        assert!(logs.iter().any(|l| l.contains("glBlitFramebuffer")), "{logs:?}");
        assert!(logs.iter().any(|l| l.contains("glTexImage3D")), "{logs:?}");
        assert!(logs.iter().any(|l| l.starts_with("GL error")), "the error-log budget is back: {logs:?}");
        gl.error();
    }
}
