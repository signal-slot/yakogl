// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Raspberry Pi 5's V3D 7.1 table, `yakogl_backend_v3d71`: the build's
//! `g_backend` when `gpu-v3d71` is on (src/backend.rs, `v3d_table`), in
//! place of the 4.2 table `yakogl_backend_v3d` (roadmap stage 5, P9;
//! docs/notes/v3d71-table71.md, docs/notes/v3d71-gl-table.md section 4).
//!
//! # What it records (plan item `table71-draw`)
//!
//! The table keeps the **open frame**: one framebuffer (`glue71::Frame71`
//! and the depth/stencil identity `ds_id`) and the GL core's draws and
//! clears on it since the last flush, each as an owned copy (the GlesDraw's
//! value fields, the program's description and uniform lists, the uniform
//! blocks, the arrays staged into the data arena). A GlesDraw on another
//! framebuffer ends the open frame first, as the 4.2 table does.
//!
//! * **draw**: the 4.2 table's shared refusals first (`state::refusal`,
//!   `state::unsupported`, called, not copied), then `glue71` maps the
//!   GlesDraw and `fill71::fill` runs dry over the queue with the new
//!   operations. A refusal is this call's return value, logged by name once
//!   per cause, and leaves the queue as it was. When the dry run gives a job
//!   more than the queue alone (a whole clear after queued draws), the queue
//!   is flushed first, so `kick_id` after the call names the kick holding
//!   the draw (src/backend.rs, `YakoglBackend::kick_id`).
//! * **flush**: `fill71::fill` for real, then per job
//!   `backend71::assemble_filled` with the C builder
//!   `driver/src/v3d/v3d_job71.c` (`backend71::CBuilder71` through `ffi71`),
//!   which checks what the builder wrote against the pure modules, then the
//!   submit of a `V3dKick` through the OSAL. A job that does not assemble is
//!   not submitted.
//! * **memory**: one arena set (binning list, render list, indirect buffer,
//!   staged data) from the OSAL's data heap at init, as the 4.2 table
//!   allocates its own, and the binner's tile allocation and tile state,
//!   allocated for the framebuffer that opens a frame (they grow, never
//!   shrink). With one set, a job waits for the one before it: before the
//!   lists are written again and before a new queue stages its first array.
//! * **readback**: the colour target is stored raster at the GL core's own
//!   address and pitch (glue71's frame: `V3D_MEMORY_FORMAT_RASTER`, stride
//!   `draw_pitch * 4`), RGBA8 as R, G, B, A (`rt_rgba8888_is_bgra` false):
//!   what `glReadPixels` reads once the kick completed (`kick_pending`,
//!   `wait_kick`, `finish`).
//!
//! # What this cut refuses by name
//!
//! * a `glClear` the tile buffer cannot do (masked or scissored: a clear
//!   quad): [`REF_CLEAR_QUAD`]. The clear program belongs to item
//!   `table71-cases`;
//! * what `backend71` does not assemble (indexed, instanced, point size) and
//!   everything `glue71`, `fill71` and their modules refuse (textures among
//!   them);
//! * a depth/stencil the framebuffer keeps between jobs: the table keeps it
//!   in the tile buffer only (`glue71::DsPlace` at address 0), so a draw
//!   that reads what an earlier job left is zskeep71's refusal, the 4.2
//!   string (gl_test case 59).
//!
//! # A product build records nothing yet
//!
//! The pure 7.1 modules are in the library only in a `host-test` build
//! (src/v3d/mod.rs: they build their results in `Vec`s and `String`s, and
//! the product library is `no_std` without an allocator). The recording
//! path is therefore `host-test` only too; in a product `gpu-v3d71` build
//! every draw and clear is still refused by [`REF_NO_RECORDING`]. `backend71`
//! is not in src/v3d/mod.rs either: it is included here with `#[path]`
//! until that file names it.
//!
//! The hooks the design note leaves `None` in the first cut (texture,
//! depth/stencil resolve and write, queries, mipmaps) are `None` here: the
//! GL core then refuses by name what needs them, or does it on the CPU.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::backend::*;

#[cfg(feature = "host-test")]
#[allow(unused_imports)]
use super::{admit71, attr71, clearplan71, ffi71, fill71, ident71, record71, rt71, uniform71, zskeep71};

/// The job assembly (its siblings are this module's imports above).
#[cfg(feature = "host-test")]
#[allow(dead_code)]
#[path = "backend71.rs"]
mod backend71;

/// The table's name (`GL_RENDERER`'s backend half), as section 1 of
/// docs/notes/v3d71-table71.md names it.
pub const NAME: &core::ffi::CStr = c"Broadcom V3D 7.1 (Raspberry Pi 5)";

/// The refusal of every draw and clear in a product build, which has no
/// recording path yet (module docs): the log line is `yakogl/v3d71: draw
/// refused: ` followed by this.
pub const REF_NO_RECORDING: &str =
    "V3D 7.1: the table records no draw or clear in this build (the 7.1 modules are host-test only)";

/// A `glClear` that needs a clear quad (masked or scissored): the table has
/// no clear program yet (item table71-cases).
pub const REF_CLEAR_QUAD: &str =
    "V3D 7.1: a masked or scissored glClear needs the clear quad, which the table does not draw yet";

/// The log prefix of a refusal, as the 4.2 table's `yakogl/v3d: draw
/// refused: ` (src/v3d/backend.rs `count_refusal`).
pub const REFUSAL_PREFIX: &str = "yakogl/v3d71: draw refused: ";

/// The draws one job is sized for when the OSAL names no budget
/// (`OSAL_QUERY_V3D_DRAWS_PER_JOB`). Smaller than the 4.2 table's 1024: every
/// draw runs `fill71::fill` over the queue.
pub const DEFAULT_DRAWS_PER_JOB: u32 = 256;
/// A draw's bytes in the binning list, an upper bound: v3d_job71 writes
/// every state packet for every draw (tests/v3d71_table_draw.rs measures a
/// draw against it).
pub const BCL_BYTES_PER_DRAW: u32 = 256;
/// The binning list's prologue and `v3d_job71_end`'s FLUSH.
pub const BCL_FIXED: u32 = 4096;
/// A draw's share of the indirect buffer the arena is sized by; a draw is
/// counted with what its streams and records take ([`BCL_BYTES_PER_DRAW`]'s
/// twin is computed per draw).
pub const IND_BYTES_PER_DRAW: u32 = 1024;
/// The indirect buffer's generic tile list (`IND_END_RESERVE` of
/// v3d_job71.c is 288 bytes) and slack.
pub const IND_FIXED: u32 = 8192;
/// The render list: about 100 bytes plus 3 per supertile (at most 256).
pub const RCL_BYTES: usize = 4 * 1024;
/// Arrays the GL core staged for one draw only, copied for the job's life
/// (the 4.2 table's `DATA_BYTES`).
pub const DATA_BYTES: usize = 1024 * 1024;

static INITED: AtomicBool = AtomicBool::new(false);
/// The draws refused since init (the 4.2 table's `refused`).
static REFUSED: AtomicU32 = AtomicU32::new(0);

/// The draws and clears refused since init.
pub fn refused_draws() -> u32 {
    REFUSED.load(Ordering::Relaxed)
}

/// Whether [`v3d71_init`] ran and no [`v3d71_shutdown`] since.
pub fn inited() -> bool {
    INITED.load(Ordering::Relaxed)
}

/// The recording path (module docs): `host-test` builds only.
#[cfg(feature = "host-test")]
mod rec {
    use core::ffi::c_void;
    use core::sync::atomic::Ordering;

    use super::backend71::{self, Builder71, CBuilder71, List71, Table71};
    use super::{
        BCL_BYTES_PER_DRAW, BCL_FIXED, DATA_BYTES, DEFAULT_DRAWS_PER_JOB, IND_BYTES_PER_DRAW, IND_FIXED, INITED,
        RCL_BYTES, REFUSAL_PREFIX, REFUSED, REF_CLEAR_QUAD,
    };
    use crate::backend::GlesDraw;
    use crate::osal::{
        self, OsalGpuMem, OSAL_MEM_DATA, OSAL_QUERY_V3D_DRAWS_PER_JOB, OSAL_QUERY_V3D_HUB_IDENT,
        OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_VPM_BYTES,
    };
    use crate::shaderc::{ShcShader, ShcV3dUniform, ShcV3dVs, SHC_V3D_NO_RECORD};
    use crate::util::{Global, EINVAL, EIO, ENODEV, ENOMEM, ENOTSUP};
    use crate::v3d::kick::{V3dKick, V3D_KICK_MAGIC};
    use crate::v3d::zskeep71::Known71;
    use crate::v3d::{attr71, fill71, glue71, record71, state, uniform71};

    /// The data heap's base. The 7.1 OSAL places every allocation in its one
    /// carve-out and ignores it (`heap_base` answers 0 for the same reason).
    const HEAP: u64 = 0;
    /// The GFXH-930 dummy: the data arena's first bytes, kept zero.
    const DUMMY_BYTES: usize = 64;
    /// How long a job may run (the 4.2 table's `JOB_TIMEOUT_US`).
    const JOB_TIMEOUT_US: i64 = 1_000_000;
    /// Colour buffers and depth/stencil identities whose history is kept.
    const HISTORY: usize = 64;
    /// v3d_frame.c `v3d_frame_alloc_sizes`: the binner's initial block a
    /// tile, the fixed part and the cap of the pool.
    const TILE_ALLOC_INITIAL_BLOCK: u64 = 128;
    const TILE_ALLOC_POOL_CAP: u64 = 512 * 1024;

    /// A linked program, owned (the GlesDraw's `fs` lives for the call only).
    struct Prog {
        desc: record71::DrawDesc,
        cs_reads: [u8; attr71::MAX_ATTRS],
        vs_reads: [u8; attr71::MAX_ATTRS],
        cs_vcm_cache_size: u32,
        vs_vcm_cache_size: u32,
        separate_segments: bool,
        varyings: fill71::VaryingFlags,
        discards: bool,
        fs_uniforms: Vec<uniform71::Word>,
        vs_uniforms: Vec<uniform71::Word>,
        cs_uniforms: Vec<uniform71::Word>,
    }

    impl Prog {
        fn view(&self) -> fill71::Program<'_> {
            fill71::Program {
                desc: self.desc,
                cs_reads: self.cs_reads,
                vs_reads: self.vs_reads,
                cs_vcm_cache_size: self.cs_vcm_cache_size,
                vs_vcm_cache_size: self.vs_vcm_cache_size,
                separate_segments: self.separate_segments,
                varyings: self.varyings,
                discards: self.discards,
                fs_uniforms: &self.fs_uniforms,
                vs_uniforms: &self.vs_uniforms,
                cs_uniforms: &self.cs_uniforms,
            }
        }
    }

    /// One GlesDraw of the open frame, owned.
    struct Queued {
        /// The GlesDraw. Its pointers are dead after the call: glue71 reads
        /// the value fields, and of a pointer only whether it is null.
        d: GlesDraw,
        prog: Option<Prog>,
        /// The arrays in attribute record order (`record_of_location`).
        attrs: Vec<attr71::Attr>,
        vs_words: Vec<u32>,
        fs_words: Vec<u32>,
        indices: Option<glue71::IndexPlace>,
    }

    /// The open frame.
    struct Open {
        frame: glue71::Frame71,
        /// `GlesDraw::ds_id` of a frame with depth/stencil, else 0: two
        /// framebuffers sharing a colour texture differ in it.
        ds_id: u32,
        queue: Vec<Queued>,
        /// The jobs the queue fills into (the last dry run's count).
        jobs: usize,
    }

    #[derive(Clone, Copy)]
    struct InFlight {
        kick: u32,
        fence: i64,
        /// The stamp of a draw that ended the frame it was stamped for and
        /// is in this job (the 4.2 table's `InFlight::alias`).
        alias: u32,
    }

    struct State {
        inited: bool,
        table: Option<Table71>,
        draws_per_job: usize,
        bcl: OsalGpuMem,
        rcl: OsalGpuMem,
        ind: OsalGpuMem,
        data: OsalGpuMem,
        data_used: usize,
        tile_alloc: OsalGpuMem,
        tile_state: OsalGpuMem,
        open: Option<Open>,
        open_alias: u32,
        in_flight: Option<InFlight>,
        kicks: u32,
        waits: u32,
        /// The colour buffers a job stored, by address.
        stored: Vec<u32>,
        /// What each depth/stencil identity holds.
        ds: Vec<(u32, Known71, Known71)>,
        /// The refusals logged in `log_epoch`.
        logged: Vec<String>,
        log_epoch: u32,
    }

    impl State {
        const ZERO: State = State {
            inited: false,
            table: None,
            draws_per_job: 0,
            bcl: OsalGpuMem::ZERO,
            rcl: OsalGpuMem::ZERO,
            ind: OsalGpuMem::ZERO,
            data: OsalGpuMem::ZERO,
            data_used: 0,
            tile_alloc: OsalGpuMem::ZERO,
            tile_state: OsalGpuMem::ZERO,
            open: None,
            open_alias: 0,
            in_flight: None,
            kicks: 0,
            waits: 0,
            stored: Vec::new(),
            ds: Vec::new(),
            logged: Vec::new(),
            log_epoch: 0,
        };
    }

    static G: Global<State> = Global::new(State::ZERO);

    fn g() -> &'static mut State {
        // SAFETY: single-threaded by contract; each entry point takes it once.
        unsafe { G.get() }
    }

    /// Why a draw is not queued.
    enum Fail {
        /// A named refusal: logged, counted, this errno.
        Refused(String, i32),
        /// The data arena cannot take the draw's arrays.
        Full,
        /// A submit or a wait failed (already logged).
        Lost(i32),
    }

    impl From<fill71::Refusal> for Fail {
        /// A 7.1 module's refusal: legal GL state this table cannot render
        /// yet (-ENOTSUP, `GL_INVALID_OPERATION` with the reason in the log).
        fn from(r: fill71::Refusal) -> Fail {
            Fail::Refused(r.to_string(), -ENOTSUP)
        }
    }

    /// Count a refused draw and log its cause once per log epoch.
    fn count_refusal(g: &mut State, why: &str) {
        REFUSED.fetch_add(1, Ordering::Relaxed);
        let epoch = osal::log_epoch();
        if g.log_epoch != epoch {
            g.log_epoch = epoch;
            g.logged.clear();
        }
        if g.logged.iter().any(|l| l == why) {
            osal::note_log_repeat();
            return;
        }
        g.logged.push(why.to_string());
        glog!("{}{}", REFUSAL_PREFIX, why);
    }

    fn gpu32(a: u64) -> Option<u32> {
        u32::try_from(a).ok().filter(|&a| a != 0)
    }

    fn query_u32(what: u32) -> Option<u32> {
        let q = osal::ops().and_then(|o| o.query)?;
        let mut v = 0u32;
        // SAFETY: a u32 out-parameter of the size passed.
        (unsafe { q(what, &mut v as *mut u32 as *mut c_void, 4) } == 0).then_some(v)
    }

    /// Tell the OSAL a job is being recorded, so that it holds the frees of
    /// what the job may name (the 4.2 table's `job_open_to_osal`).
    fn job_open_to_osal(open: bool) {
        if let Some(q) = osal::ops().and_then(|o| o.query) {
            let mut v = open as u32;
            // SAFETY: a u32 in-parameter of the size passed.
            let _ = unsafe { q(OSAL_QUERY_V3D_JOB_OPEN, &mut v as *mut u32 as *mut c_void, 4) };
        }
    }

    fn alloc(mem: &mut OsalGpuMem, bytes: usize) -> bool {
        osal::gpu_alloc(HEAP, bytes, OSAL_MEM_DATA, mem) == 0 && !mem.cpu.is_null() && gpu32(mem.gpu).is_some()
    }

    fn free_all(g: &mut State) {
        for mem in [&mut g.bcl, &mut g.rcl, &mut g.ind, &mut g.data, &mut g.tile_alloc, &mut g.tile_state] {
            osal::gpu_free(mem);
        }
    }

    pub fn init() -> i32 {
        let g = g();
        if g.inited {
            return 0;
        }
        *g = State::ZERO;
        REFUSED.store(0, Ordering::Relaxed);

        /* the online compiler links every program for V3D_VPM_BYTES (the
         * 4.2 table's check) */
        let want = crate::shaderc::online::codegen::V3D_VPM_BYTES;
        let vpm = query_u32(OSAL_QUERY_V3D_VPM_BYTES);
        if vpm != Some(want) {
            glog!("yakogl/v3d71: the VPM is {:?} bytes, the programs are compiled for {}: not starting", vpm, want);
            return -ENODEV;
        }
        /* the hub's identity: only 7.1 with IPREV >= 10 (BCM2712 D0) */
        let mut ident = [0u32; 2];
        let rc = match osal::ops().and_then(|o| o.query) {
            // SAFETY: two u32 out-parameters of the size passed.
            Some(q) => unsafe { q(OSAL_QUERY_V3D_HUB_IDENT, ident.as_mut_ptr() as *mut c_void, 8) },
            None => -ENODEV,
        };
        if rc != 0 {
            glog!("yakogl/v3d71: the OSAL does not give the hub identity (query {}): not starting", rc);
            return -ENODEV;
        }
        let table = match backend71::init(ident[0], ident[1]) {
            Ok(t) => t,
            Err(r) => {
                glog!("yakogl/v3d71: {}: not starting", r);
                return -ENODEV;
            }
        };
        let (draws, from) = match query_u32(OSAL_QUERY_V3D_DRAWS_PER_JOB) {
            Some(n) if n != 0 => (n, "the platform's budget"),
            _ => (DEFAULT_DRAWS_PER_JOB, "the default"),
        };
        let size = |fixed: u32, per: u32| draws.checked_mul(per)?.checked_add(fixed)?.checked_next_multiple_of(4096);
        let (Some(bcl_bytes), Some(ind_bytes)) =
            (size(BCL_FIXED, BCL_BYTES_PER_DRAW), size(IND_FIXED, IND_BYTES_PER_DRAW))
        else {
            glog!("yakogl/v3d71: a job of {} draws does not fit 32-bit arenas: not starting", draws);
            return -ENOMEM;
        };
        let mut short = None;
        for (mem, bytes) in [
            (&mut g.bcl, bcl_bytes as usize),
            (&mut g.rcl, RCL_BYTES),
            (&mut g.ind, ind_bytes as usize),
            (&mut g.data, DATA_BYTES),
        ] {
            if !alloc(mem, bytes) {
                short = Some(bytes);
                break;
            }
        }
        if let Some(bytes) = short {
            glog!("yakogl/v3d71: out of GPU memory for the job buffers ({} bytes)", bytes);
            free_all(g);
            return -ENOMEM;
        }
        // SAFETY: the data arena's CPU mapping is DATA_BYTES long.
        unsafe { core::ptr::write_bytes(g.data.cpu as *mut u8, 0, DUMMY_BYTES) };
        glog!(
            "yakogl/v3d71: V3D {}.{}.{}; job arenas for {} draws ({}): BCL {}, indirect {} bytes",
            table.ident.ver / 10,
            table.ident.ver % 10,
            table.ident.iprev,
            draws,
            from,
            bcl_bytes,
            ind_bytes
        );
        g.table = Some(table);
        g.draws_per_job = draws as usize;
        g.data_used = DUMMY_BYTES;
        g.inited = true;
        INITED.store(true, Ordering::Relaxed);
        0
    }

    pub fn shutdown() {
        let g = g();
        if g.inited {
            /* nothing may read the buffers when they are freed */
            let _ = wait_in_flight(g);
            free_all(g);
        }
        *g = State::ZERO;
        INITED.store(false, Ordering::Relaxed);
    }

    /* ---- waiting ---------------------------------------------------------------- */

    fn fence_done(fence: i64) -> bool {
        match osal::ops().and_then(|o| o.completed) {
            // SAFETY: a plain query.
            Some(c) => (unsafe { c() }) >= fence,
            None => false,
        }
    }

    /// Wait for the job in flight, if there is one: 0, or its error.
    fn wait_in_flight(g: &mut State) -> i32 {
        let Some(f) = g.in_flight.take() else {
            return 0;
        };
        let Some(wait) = osal::ops().and_then(|o| o.wait) else {
            return -ENODEV;
        };
        // SAFETY: a wait on a fence the OSAL returned.
        let rc = unsafe { wait(f.fence, JOB_TIMEOUT_US) };
        g.waits += 1;
        if rc != 0 {
            glog!("yakogl/v3d71: job {} did not complete ({})", f.kick, rc);
            return if rc < 0 { rc } else { -EIO };
        }
        0
    }

    /* ---- the framebuffer's history ------------------------------------------------ */

    fn history(g: &State, frame: &glue71::Frame71, ds_id: u32) -> fill71::History {
        let mut h = fill71::History::NEW;
        /* a buffer not in a full list counts as stored: a load of contents
         * nobody kept is harmless, a load left out loses a picture */
        let unknown = g.stored.len() >= HISTORY;
        for (n, c) in frame.colors[..frame.n].iter().enumerate() {
            h.colors[n] = unknown || g.stored.contains(&c.address);
        }
        if frame.ds.is_some() {
            (h.z, h.s) = match g.ds.iter().find(|e| e.0 == ds_id) {
                Some(e) => (e.1, e.2),
                /* not in a full list: written and not kept, the answer
                 * that refuses a read */
                None if g.ds.len() >= HISTORY => (Known71::Written, Known71::Written),
                None => (Known71::Untouched, Known71::Untouched),
            };
        }
        h
    }

    fn record_history(g: &mut State, frame: &glue71::Frame71, ds_id: u32, after: &fill71::History) {
        for (n, c) in frame.colors[..frame.n].iter().enumerate() {
            if after.colors[n] && !g.stored.contains(&c.address) && g.stored.len() < HISTORY {
                g.stored.push(c.address);
            }
        }
        if frame.ds.is_some() {
            match g.ds.iter().position(|e| e.0 == ds_id) {
                Some(i) => g.ds[i] = (ds_id, after.z, after.s),
                None if g.ds.len() < HISTORY => g.ds.push((ds_id, after.z, after.s)),
                None => {}
            }
        }
    }

    /* ---- GlesDraw to an owned queue entry -------------------------------------------- */

    /// Copy `n` bytes the GL core staged for this draw into the data arena.
    fn stage(g: &mut State, src: *const c_void, n: usize) -> Result<u32, Fail> {
        let at = g.data_used.next_multiple_of(16);
        if n == 0 || at + n > DATA_BYTES {
            return Err(Fail::Full);
        }
        // SAFETY: the arena's CPU mapping is DATA_BYTES long and `at + n`
        // fits; the GL core's staging holds `n` bytes at `src`.
        unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (g.data.cpu as *mut u8).add(at), n) };
        g.data_used = at + n;
        Ok(g.data.gpu as u32 + at as u32)
    }

    fn words(cpu: *const c_void, dwords: u16) -> Vec<u32> {
        if cpu.is_null() || dwords == 0 {
            return Vec::new();
        }
        // SAFETY: the GlesDraw's contract: `dwords` words at `cpu`. The block
        // may be unaligned for u32.
        (0..dwords as usize).map(|i| unsafe { (cpu as *const u32).add(i).read_unaligned() }).collect()
    }

    /// The program of a draw, as the 4.2 table's `programs` reads it (the
    /// same checks and strings; -EINVAL, a malformed table).
    fn program(d: &GlesDraw, fs: &ShcShader) -> Result<Prog, Fail> {
        let bad = |why: &str| Fail::Refused(why.to_string(), -EINVAL);
        let p = &fs.v3d;
        if p.linked != 1 {
            return Err(bad("the fragment shader is not a program linked for the V3D"));
        }
        let bytes = fs.usc_dw as u64 * 4;
        let code = |off: u32, len: u32| -> Result<u32, Fail> {
            if !off.is_multiple_of(8) || len == 0 || off as u64 + len as u64 > bytes {
                return Err(bad("a program's code lies outside the uploaded code"));
            }
            gpu32(d.fs_usc_gpu + off as u64)
                .ok_or_else(|| bad("the program's code is outside the V3D's 32-bit address space"))
        };
        let list = |u: *const ShcV3dUniform, n: u32| -> Vec<uniform71::Word> {
            if u.is_null() {
                return Vec::new();
            }
            // SAFETY: a linked program's list has `n` entries.
            unsafe { core::slice::from_raw_parts(u, n as usize) }
                .iter()
                .map(|w| uniform71::Word { contents: w.contents, data: w.data })
                .collect()
        };
        let vertex = |h: &ShcV3dVs| -> Result<record71::VertexDesc, Fail> {
            Ok(record71::VertexDesc {
                prog: record71::ProgramDesc {
                    code: code(h.code_off, h.code_bytes)?,
                    uniforms: 0,
                    threads: h.threads as u32,
                    single_seg: h.single_seg != 0,
                    propagate_nans: false,
                },
                vpm_input_size: h.vpm_input_size,
                vpm_output_size: h.vpm_output_size,
                /* v3d_compute_vpm_config without a geometry shader, what
                 * v3d_job71 writes (fill71's REF_RECORD_FIXED) */
                min_input_segments: 1,
                min_output_segments: 0,
                uses_vid: h.uses_vid != 0,
                uses_iid: h.uses_iid != 0,
                uses_base_instance: h.uses_biid != 0,
                uses_base_vertex: false,
                uses_draw_index: false,
            })
        };
        let f = &p.fs;
        let desc = record71::DrawDesc {
            fs: record71::FragmentDesc {
                prog: record71::ProgramDesc {
                    code: code(f.code_off, f.code_bytes)?,
                    uniforms: 0,
                    threads: f.threads as u32,
                    single_seg: f.single_seg != 0,
                    propagate_nans: false,
                },
                writes_z: f.writes_z != 0,
                disable_ez: f.disable_ez != 0,
                num_varyings: f.num_inputs,
                uses_center_w: f.uses_center_w != 0,
                uses_pid: f.uses_pid != 0,
                lock_scoreboard_on_first_thrsw: f.lock_scoreboard_on_first_thrsw != 0,
                uses_implicit_point_line_varyings: f.uses_implicit_point_line_varyings != 0,
                sample_rate_shading: false,
            },
            vs: vertex(&p.vs)?,
            cs: vertex(&p.cs)?,
            point_size: p.vs.writes_psiz != 0,
        };
        Ok(Prog {
            desc,
            cs_reads: p.cs.vattr_sizes,
            vs_reads: p.vs.vattr_sizes,
            cs_vcm_cache_size: p.cs.vcm_cache_size,
            vs_vcm_cache_size: p.vs.vcm_cache_size,
            separate_segments: p.vs.separate_segments != 0 || p.cs.separate_segments != 0,
            varyings: fill71::VaryingFlags {
                flat: [f.flat_shade_flags, f.flat_shade_flags_hi],
                noperspective: [f.noperspective_flags, f.noperspective_flags_hi],
                centroid: [f.centroid_flags, f.centroid_flags_hi],
            },
            discards: fs.uses_discard != 0,
            fs_uniforms: list(f.uniforms, f.num_uniforms),
            vs_uniforms: list(p.vs.uniforms, p.vs.num_uniforms),
            cs_uniforms: list(p.cs.uniforms, p.cs.num_uniforms),
        })
    }

    /// One GlesDraw as a queue entry: its program, its arrays staged and put
    /// in attribute record order, its uniform blocks.
    ///
    /// # Safety
    /// `d`'s pointers are live (the draw hook's contract).
    unsafe fn queued(g: &mut State, d: &GlesDraw) -> Result<Queued, Fail> {
        let mut q =
            Queued { d: *d, prog: None, attrs: Vec::new(), vs_words: Vec::new(), fs_words: Vec::new(), indices: None };
        if d.index_count == 0 {
            return Ok(q); /* a clear alone */
        }
        // SAFETY: the contract: null or a live linked shader.
        let Some(fs) = (unsafe { d.fs.as_ref() }) else {
            return Ok(q); /* glue71 names it: a draw with no linked program */
        };
        let prog = program(d, fs)?;
        let v3d = &fs.v3d;
        let list = &d.attribs[..(d.num_attribs as usize).min(d.attribs.len())];
        /* stage what the program reads; an array it does not read is never
         * fetched, so it gets the dummy's address */
        let dummy = g.data.gpu as u32;
        let mut staged = Vec::with_capacity(list.len());
        for a in list {
            let read = v3d.record_of_location.get(a.vi_reg as usize).is_some_and(|&r| r != SHC_V3D_NO_RECORD);
            staged.push(if read && !a.cpu.is_null() { stage(g, a.cpu, a.cpu_bytes as usize)? } else { dummy });
        }
        let by_location = glue71::attrs(d, &staged)?;
        /* attr71's index is the attribute record (the programs'
         * `vattr_sizes` are per record): the location's record, as the 4.2
         * table places them (`record_arrays_and_draw`) */
        let mut attrs = vec![attr71::Attr::disabled(); (v3d.num_records as usize).min(attr71::MAX_ATTRS)];
        for (loc, a) in by_location.iter().enumerate() {
            let rec = v3d.record_of_location.get(loc).copied().unwrap_or(SHC_V3D_NO_RECORD) as usize;
            if a.enabled && rec < attrs.len() {
                attrs[rec] = *a;
            }
        }
        /* staged indices: backend71 assembles glDrawArrays only and names
         * the refusal; the indices are not copied for a draw it refuses */
        if !d.index_cpu.is_null() {
            q.indices = Some(glue71::IndexPlace { addr: dummy, size: u64::from(d.index_bytes) });
        }
        q.prog = Some(prog);
        q.attrs = attrs;
        q.vs_words = words(d.vs_uniform_cpu, d.vs_uniform_dwords);
        q.fs_words = words(d.fs_uniform_cpu, d.fs_uniform_dwords);
        Ok(q)
    }

    /* ---- the queue to fill71's jobs --------------------------------------------------- */

    /// What a job's lists must hold for these draws: the binning list and
    /// the indirect buffer (uniform streams, the 32-byte record on a 32-byte
    /// boundary, the attribute records).
    fn list_bytes(jobs: &[fill71::FilledJob]) -> (usize, usize) {
        let mut most = (0, 0);
        for j in jobs {
            let ind: usize = j
                .draws
                .iter()
                .map(|d| {
                    4 * (d.fs_uniforms.len() + d.vs_uniforms.len() + d.cs_uniforms.len())
                        + 64
                        + record71::RECORD_SIZE
                        + attr71::RECORD_BYTES * d.attrs.len().max(1)
                })
                .sum();
            most = (most.0.max(j.draws.len() * BCL_BYTES_PER_DRAW as usize), most.1.max(ind));
        }
        most
    }

    /// `fill71::fill` over `queue` (and `extra`, the draw being admitted) on
    /// the open frame, with what the builder path would refuse later: a
    /// clear quad ([`REF_CLEAR_QUAD`]) and backend71's own refusals.
    fn fill(
        g: &State,
        frame: &glue71::Frame71,
        ds_id: u32,
        queue: &[Queued],
        extra: Option<&Queued>,
    ) -> Result<Vec<fill71::FilledJob>, fill71::Refusal> {
        let all: Vec<&Queued> = queue.iter().chain(extra).collect();
        let dummy = g.data.gpu as u32;
        let programs: Vec<Option<fill71::Program<'_>>> = all.iter().map(|q| q.prog.as_ref().map(Prog::view)).collect();
        let mut ops = Vec::with_capacity(all.len() + 1);
        for (q, p) in all.iter().zip(&programs) {
            let input = p.as_ref().map(|program| glue71::DrawInputs {
                program,
                attrs: &q.attrs,
                vs_words: &q.vs_words,
                fs_words: &q.fs_words,
                dummy_addr: dummy,
                oq_addr: 0,
                indices: q.indices,
            });
            ops.extend(glue71::ops(&q.d, frame, input.as_ref())?);
        }
        /* fill71 takes a clear quad's program; the table has none yet. This
         * stand-in lets fill71 plan the flush: a job that then holds a clear
         * quad is refused below, so the stand-in is never drawn. */
        let vertex = record71::VertexDesc {
            prog: record71::ProgramDesc { code: dummy, threads: 1, single_seg: true, ..Default::default() },
            vpm_input_size: 1,
            vpm_output_size: 1,
            min_input_segments: 1,
            ..Default::default()
        };
        let mut reads = [0u8; attr71::MAX_ATTRS];
        reads[0] = 4;
        let stand_in = fill71::Program {
            desc: record71::DrawDesc {
                fs: record71::FragmentDesc { prog: vertex.prog, ..Default::default() },
                vs: vertex,
                cs: vertex,
                point_size: false,
            },
            cs_reads: reads,
            vs_reads: reads,
            cs_vcm_cache_size: 1,
            vs_vcm_cache_size: 1,
            separate_segments: false,
            varyings: fill71::VaryingFlags::default(),
            discards: false,
            fs_uniforms: &[],
            vs_uniforms: &[],
            cs_uniforms: &[],
        };
        let quad = fill71::ClearQuad {
            program: &stand_in,
            attr: attr71::Attr {
                enabled: true,
                components: 4,
                stride: 16,
                buffer_addr: dummy,
                ..attr71::Attr::disabled()
            },
            dummy_addr: dummy,
        };
        let fb = frame.framebuffer(g.tile_alloc.gpu as u32);
        let rev = g.table.map_or(record71::BCM2712_D0, |t| t.rev);
        let jobs = fill71::fill(&fb, rev, &history(g, frame, ds_id), &quad, &ops)?;
        for j in &jobs {
            for d in &j.draws {
                if d.is_clear {
                    return Err(fill71::Refusal { module: "table71", reason: REF_CLEAR_QUAD.to_string() });
                }
                backend71::draw_of(d)?;
            }
        }
        Ok(jobs)
    }

    /// The binner's memory for a `w` x `h` frame of `tile` pixels a tile:
    /// what `v3d_job71_alloc_sizes` can ask for any job on it.
    fn tile_memory(g: &mut State, w: u32, h: u32, tile: (u32, u32)) -> i32 {
        let tiles = u64::from(w.div_ceil(tile.0.max(1))) * u64::from(h.div_ceil(tile.1.max(1)));
        let page = u64::from(backend71::PAGE_SIZE);
        let alloc = ((tiles * TILE_ALLOC_INITIAL_BLOCK).next_multiple_of(4096) + 8192 + TILE_ALLOC_POOL_CAP)
            .next_multiple_of(page) as usize;
        let state = (tiles * 256).next_multiple_of(4096) as usize;
        if g.tile_alloc.size >= alloc && g.tile_state.size >= state {
            return 0;
        }
        /* the job in flight bins into the buffers being replaced */
        let rc = wait_in_flight(g);
        if rc < 0 {
            return rc;
        }
        for (mem, bytes) in [(&mut g.tile_alloc, alloc), (&mut g.tile_state, state)] {
            if mem.size >= bytes {
                continue;
            }
            osal::gpu_free(mem);
            if !alloc_mem(mem, bytes) {
                glog!("yakogl/v3d71: out of GPU memory for the binner's buffers ({} bytes)", bytes);
                return -ENOMEM;
            }
        }
        0
    }

    fn alloc_mem(mem: &mut OsalGpuMem, bytes: usize) -> bool {
        *mem = OsalGpuMem::ZERO;
        alloc(mem, bytes)
    }

    /* ---- flush ------------------------------------------------------------------------- */

    /// A list's CPU mapping.
    ///
    /// # Safety
    /// `m` is a live allocation with a CPU mapping, not aliased while the
    /// slice is.
    unsafe fn list<'m>(m: &OsalGpuMem) -> (&'m mut [u8], u32) {
        // SAFETY: the caller's contract.
        (unsafe { core::slice::from_raw_parts_mut(m.cpu as *mut u8, m.size) }, m.gpu as u32)
    }

    /// Fill, build and submit the open frame's queue; the frame stays open,
    /// its queue empty. 0, or the error of a job that was not submitted (the
    /// queue is dropped with it: the next draw starts a new one).
    fn flush_queue(g: &mut State) -> i32 {
        let Some(mut open) = g.open.take() else {
            return 0;
        };
        let queue = core::mem::take(&mut open.queue);
        open.jobs = 0;
        let (frame, ds_id) = (open.frame, open.ds_id);
        g.open = Some(open);
        if queue.is_empty() {
            return 0;
        }
        job_open_to_osal(false);
        let jobs = match fill(g, &frame, ds_id, &queue, None) {
            Ok(j) => j,
            Err(r) => {
                /* the dry run admitted every draw of it */
                glog!("yakogl/v3d71: flush: the queued draws are refused ({}): not submitted", r);
                return -EIO;
            }
        };
        let Some(table) = g.table else {
            return -ENODEV;
        };
        let mut rc = 0;
        for job in &jobs {
            /* one arena set: the job in flight reads the lists */
            let waited = wait_in_flight(g);
            if waited < 0 {
                rc = waited;
            }
            let fb = frame.framebuffer(g.tile_alloc.gpu as u32);
            // SAFETY: the three arenas are live, apart, and nothing else
            // holds their mappings while the builder writes them.
            let mut b = unsafe { CBuilder71::new(list(&g.bcl), list(&g.rcl), list(&g.ind), backend71::PAGE_SIZE) };
            let a = match backend71::assemble_filled(&table, &fb, job, &mut b) {
                Ok(a) => a,
                Err(r) => {
                    glog!("yakogl/v3d71: flush: the job does not assemble ({}): not submitted", r);
                    return -EIO;
                }
            };
            let used = [List71::Bcl, List71::Rcl, List71::Ind].map(|l| b.bytes(l).len());
            if a.built.tile_alloc_size as usize > g.tile_alloc.size
                || a.built.tile_state_size as usize > g.tile_state.size
            {
                glog!(
                    "yakogl/v3d71: flush: the job bins into {} + {} bytes, the buffers hold {} + {}: not submitted",
                    a.built.tile_alloc_size,
                    a.built.tile_state_size,
                    g.tile_alloc.size,
                    g.tile_state.size
                );
                return -ENOMEM;
            }
            // SAFETY: the bytes written are inside the CPU mappings.
            unsafe {
                osal::cache_flush(g.bcl.cpu, used[0]);
                osal::cache_flush(g.rcl.cpu, used[1]);
                osal::cache_flush(g.ind.cpu, used[2]);
                osal::cache_flush(g.data.cpu, g.data_used);
            }
            let kick = V3dKick {
                magic: V3D_KICK_MAGIC,
                bcl_start: g.bcl.gpu as u32,
                bcl_end: g.bcl.gpu as u32 + used[0] as u32,
                rcl_start: g.rcl.gpu as u32,
                rcl_end: g.rcl.gpu as u32 + used[1] as u32,
                tile_alloc: g.tile_alloc.gpu as u32,
                tile_alloc_size: a.built.tile_alloc_size,
                tile_state: g.tile_state.gpu as u32,
                tile_state_size: a.built.tile_state_size,
                pad: 0,
            };
            let Some(submit) = osal::ops().and_then(|o| o.submit) else {
                return -ENODEV;
            };
            // SAFETY: `kick` is a live V3dKick of the size passed.
            let fence = unsafe { submit(&kick as *const V3dKick as *const c_void, core::mem::size_of::<V3dKick>()) };
            if fence < 0 {
                glog!("yakogl/v3d71: submit failed ({})", fence);
                return -EIO;
            }
            g.kicks += 1;
            g.in_flight = Some(InFlight { kick: g.kicks, fence, alias: core::mem::take(&mut g.open_alias) });
            record_history(g, &frame, ds_id, &job.after);
        }
        rc
    }

    /* ---- draw --------------------------------------------------------------------------- */

    /// # Safety
    /// `draw` is null or a valid `GlesDraw` whose pointers are live.
    pub unsafe fn draw(draw: *const GlesDraw) -> i32 {
        let g = g();
        if !g.inited {
            return -ENODEV;
        }
        // SAFETY: the caller's contract.
        let Some(d) = (unsafe { draw.as_ref() }) else {
            return -EINVAL;
        };
        /* the id the GL core stamped this draw's target and buffers with */
        let stamped = g.kicks.wrapping_add(1);
        // SAFETY: the caller's contract, passed on.
        let rc = match unsafe { draw_call(g, d) } {
            Ok(()) => 0,
            Err(Fail::Refused(why, errno)) => {
                count_refusal(g, &why);
                errno
            }
            Err(Fail::Full) => {
                count_refusal(g, "the draw's arrays do not fit the job's data arena");
                -ENOMEM
            }
            Err(Fail::Lost(rc)) => rc,
        };
        if g.open.as_ref().is_some_and(|o| !o.queue.is_empty()) && g.kicks.wrapping_add(1) != stamped {
            /* the draw ended the frame it was stamped for and is in this
             * one: what it uses stays pending until this job has run (the
             * 4.2 table's `open_alias`) */
            g.open_alias = stamped;
        }
        rc
    }

    /// Before the first draw of an empty queue stages anything: the job in
    /// flight reads the data arena.
    fn start_queue(g: &mut State) -> Result<(), Fail> {
        let rc = wait_in_flight(g);
        if rc < 0 {
            return Err(Fail::Lost(rc));
        }
        g.data_used = DUMMY_BYTES;
        Ok(())
    }

    fn flush_for_draw(g: &mut State) -> Result<(), Fail> {
        match flush_queue(g) {
            0 => Ok(()),
            rc => Err(Fail::Lost(rc)),
        }
    }

    /// # Safety
    /// `d`'s pointers are live.
    unsafe fn draw_call(g: &mut State, d: &GlesDraw) -> Result<(), Fail> {
        /* the shared 4.2 decisions, the same strings on both boards */
        if let Some(why) = state::refusal(d) {
            return Err(Fail::Refused(why.to_string(), -EINVAL));
        }
        if let Some(why) = state::unsupported(d) {
            return Err(Fail::Refused(why.to_string(), -ENOTSUP));
        }
        if let Some(r) = glue71::refusal(d) {
            return Err(r.into());
        }
        /* depth/stencil stays in the tile buffer in this cut */
        let frame = glue71::frame(d, glue71::DsPlace::default())?;
        let ds_id = if frame.ds.is_some() { d.ds_id } else { 0 };
        if g.open.as_ref().is_some_and(|o| o.frame != frame || o.ds_id != ds_id) {
            flush_for_draw(g)?;
            g.open = None;
        }
        if g.open.is_none() {
            let admitted = backend71::admit_framebuffer(&frame.framebuffer(0))?;
            match tile_memory(g, frame.width, frame.height, admitted.tile) {
                0 => {}
                rc if rc == -ENOMEM => {
                    return Err(Fail::Refused("out of GPU memory for the binner's buffers".to_string(), -ENOMEM))
                }
                rc => return Err(Fail::Lost(rc)),
            }
            g.open = Some(Open { frame, ds_id, queue: Vec::new(), jobs: 0 });
        }
        let queue_len = |g: &State| g.open.as_ref().map_or(0, |o| o.queue.len());
        if queue_len(g) >= g.draws_per_job {
            flush_for_draw(g)?;
        }
        if queue_len(g) == 0 {
            start_queue(g)?;
        }

        let mark = g.data_used;
        // SAFETY: the caller's contract.
        let q = match unsafe { queued(g, d) } {
            Err(Fail::Full) if queue_len(g) != 0 => {
                /* the open frame's arrays fill the arena: submit it and
                 * stage this draw into an empty one */
                flush_for_draw(g)?;
                start_queue(g)?;
                // SAFETY: as above.
                unsafe { queued(g, d) }
            }
            r => r,
        };
        let q = match q {
            Ok(q) => q,
            Err(e) => {
                g.data_used = mark.min(g.data_used);
                return Err(e);
            }
        };
        let mark = if queue_len(g) == 0 { DUMMY_BYTES } else { mark };

        /* the dry run: the refusal is this call's, the queue is untouched */
        let mut tries = 0;
        let jobs = loop {
            let open = g.open.as_ref().expect("opened above");
            let jobs = match fill(g, &frame, ds_id, &open.queue, Some(&q)) {
                Ok(j) => j,
                Err(r) => {
                    g.data_used = mark;
                    return Err(r.into());
                }
            };
            let (bcl, ind) = list_bytes(&jobs);
            let fits = bcl + BCL_FIXED as usize <= g.bcl.size && ind + IND_FIXED as usize <= g.ind.size;
            /* a job more than the queue alone: a whole clear after queued
             * draws opens a job. The queue goes first, so that the kick
             * after this call holds the draw. The same when the lists
             * cannot take the draw with the queue. */
            let split = !open.queue.is_empty() && (jobs.len() > open.jobs.max(1) || !fits);
            if split && tries == 0 {
                tries = 1;
                flush_for_draw(g)?;
                continue;
            }
            if !fits {
                g.data_used = mark;
                return Err(Fail::Refused("the draw does not fit an empty job's control lists".to_string(), -ENOMEM));
            }
            break jobs.len();
        };
        if jobs == 0 {
            /* nothing to do (a clear every mask drops) */
            g.data_used = mark;
            return Ok(());
        }
        let open = g.open.as_mut().expect("opened above");
        if open.queue.is_empty() {
            job_open_to_osal(true);
        }
        open.queue.push(q);
        open.jobs = jobs;
        Ok(())
    }

    /* ---- the other hooks ------------------------------------------------------------------ */

    pub fn flush() -> i32 {
        let g = g();
        if !g.inited {
            return -ENODEV;
        }
        flush_queue(g)
    }

    pub fn finish() -> i32 {
        let g = g();
        if !g.inited {
            return -ENODEV;
        }
        let rc = flush_queue(g);
        let waited = wait_in_flight(g);
        if rc < 0 {
            rc
        } else {
            waited
        }
    }

    /// One job is outstanding at most, and it is the last submitted.
    pub fn finish_previous() -> i32 {
        if g().inited {
            0
        } else {
            -ENODEV
        }
    }

    pub fn kick_id() -> u32 {
        g().kicks.wrapping_add(1)
    }

    pub fn kick_pending(kick_id: u32) -> bool {
        let g = g();
        kick_id != 0
            && (kick_id > g.kicks
                || (g.open.as_ref().is_some_and(|o| !o.queue.is_empty()) && kick_id == g.open_alias)
                || g.in_flight.is_some_and(|f| (kick_id >= f.kick || kick_id == f.alias) && !fence_done(f.fence)))
    }

    pub fn wait_kick(kick_id: u32) -> i32 {
        let g = g();
        if !g.inited {
            return -ENODEV;
        }
        if kick_id == 0 {
            return 0;
        }
        if kick_id > g.kicks {
            let rc = flush_queue(g);
            let waited = wait_in_flight(g);
            return if rc < 0 { rc } else { waited };
        }
        if g.in_flight.is_some_and(|f| f.kick >= kick_id || f.alias == kick_id) {
            wait_in_flight(g)
        } else {
            0
        }
    }

    pub fn counters() -> (u32, u32) {
        let g = g();
        (g.kicks, g.waits)
    }
}

/// Init: the VPM and hub identity checks and the job arenas (`host-test`);
/// nothing in a product build, which records nothing.
pub extern "C" fn v3d71_init() -> i32 {
    #[cfg(feature = "host-test")]
    {
        rec::init()
    }
    #[cfg(not(feature = "host-test"))]
    {
        if !INITED.swap(true, Ordering::Relaxed) {
            REFUSED.store(0, Ordering::Relaxed);
        }
        0
    }
}

/// Shutdown: waits for the job in flight and frees what init allocated.
pub extern "C" fn v3d71_shutdown() {
    #[cfg(feature = "host-test")]
    rec::shutdown();
    INITED.store(false, Ordering::Relaxed);
}

/// The draw (see [`crate::backend::draw`] for the contract): queued on the
/// open frame, or refused by name (module docs).
///
/// # Safety
/// `draw` is null or a valid `GlesDraw` whose pointers are live for the
/// call.
pub unsafe extern "C" fn v3d71_draw(draw: *const GlesDraw) -> i32 {
    #[cfg(feature = "host-test")]
    {
        // SAFETY: the caller's contract, passed on.
        unsafe { rec::draw(draw) }
    }
    #[cfg(not(feature = "host-test"))]
    {
        let _ = draw;
        REFUSED.fetch_add(1, Ordering::Relaxed);
        crate::glog_once!("{}{}", REFUSAL_PREFIX, REF_NO_RECORDING);
        -crate::util::ENOTSUP
    }
}

/// Flush: submit the open frame's jobs; does not wait for them.
pub extern "C" fn v3d71_flush() -> i32 {
    #[cfg(feature = "host-test")]
    {
        rec::flush()
    }
    #[cfg(not(feature = "host-test"))]
    0
}

/// Finish: flush, then wait for the job in flight.
pub extern "C" fn v3d71_finish() -> i32 {
    #[cfg(feature = "host-test")]
    {
        rec::finish()
    }
    #[cfg(not(feature = "host-test"))]
    0
}

/// Finish the previous frame: at most one job is outstanding and it is the
/// last submitted (the 7.1 OSAL runs one job at a time), so there is none
/// before it to wait for.
pub extern "C" fn v3d71_finish_previous() -> i32 {
    #[cfg(feature = "host-test")]
    {
        rec::finish_previous()
    }
    #[cfg(not(feature = "host-test"))]
    0
}

/// The kick being recorded: the open frame's, or the next one's (the jobs
/// submitted + 1). 0, "no kick", before init and in a product build, which
/// submits nothing.
pub extern "C" fn v3d71_kick_id() -> u32 {
    #[cfg(feature = "host-test")]
    {
        if inited() {
            rec::kick_id()
        } else {
            0
        }
    }
    #[cfg(not(feature = "host-test"))]
    0
}

/// Whether `kick_id` has still to complete: the kick being recorded, and
/// the job in flight until the OSAL completed it.
pub extern "C" fn v3d71_kick_pending(kick_id: u32) -> bool {
    #[cfg(feature = "host-test")]
    {
        inited() && rec::kick_pending(kick_id)
    }
    #[cfg(not(feature = "host-test"))]
    {
        let _ = kick_id;
        false
    }
}

/// Wait for `kick_id`: submits it first when it is the open frame.
pub extern "C" fn v3d71_wait_kick(kick_id: u32) -> i32 {
    #[cfg(feature = "host-test")]
    {
        rec::wait_kick(kick_id)
    }
    #[cfg(not(feature = "host-test"))]
    {
        let _ = kick_id;
        0
    }
}

/// The heaps' bases: 0. The 7.1 OSAL places every allocation in its one
/// carve-out and ignores the base (`src/osal/zephyr_v3d71.c`, `z_gpu_alloc`);
/// a base of 0 also keeps the owner tally from sorting blocks into heaps
/// (`src/osal/mod.rs`, `OwnerTally::add`).
pub extern "C" fn v3d71_heap_base(_heap: GlesHeap) -> u64 {
    0
}

/// The jobs submitted and the CPU waits since init.
///
/// # Safety
/// Both pointers are null or writable.
pub unsafe extern "C" fn v3d71_counters(kicks: *mut u32, waits: *mut u32) {
    #[cfg(feature = "host-test")]
    let (k, w) = if inited() { rec::counters() } else { (0, 0) };
    #[cfg(not(feature = "host-test"))]
    let (k, w) = (0, 0);
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(p) = kicks.as_mut() {
            *p = k;
        }
        if let Some(p) = waits.as_mut() {
            *p = w;
        }
    }
}

/// The V3D 7.1 table (`g_backend` on a `gpu-v3d71` build).
#[no_mangle]
pub static yakogl_backend_v3d71: YakoglBackend = YakoglBackend {
    name: NAME.as_ptr(),
    /* fill71 takes float records only (REF_FLOAT_ATTRS); the GL core
     * converts the rest, as for 4.2 */
    native_attrib_formats: gles_attribf_bit(GLES_ATTRIBF_FLOAT32),
    /* rt71 keeps r_b_swap false: the RGBA8 store writes R, G, B, A */
    rt_rgba8888_is_bgra: false,
    /* the texture answers stay the 4.2 table's (section 1): right for P10 */
    mip_level_align: 0,
    compressed_blocks_in_rows: true,
    native_layered_textures: true,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: true,
    init: Some(v3d71_init),
    shutdown: Some(v3d71_shutdown),
    draw: Some(v3d71_draw),
    flush: Some(v3d71_flush),
    finish: Some(v3d71_finish),
    finish_previous: Some(v3d71_finish_previous),
    kick_id: Some(v3d71_kick_id),
    kick_pending: Some(v3d71_kick_pending),
    heap_base: Some(v3d71_heap_base),
    counters: Some(v3d71_counters),
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    /* backend71 refuses instancing (REF_INSTANCED): no divisor above 1 */
    max_instance_divisor: 1,
    wait_kick: Some(v3d71_wait_kick),
    /* nothing is sampled yet: false makes the GL core wait as for a GPU
     * reader, the safe answer */
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    /* fill71 takes the uniform blocks as host words */
    uniforms_host_only: true,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};
