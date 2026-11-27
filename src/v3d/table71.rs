// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Raspberry Pi 5's V3D 7.1 table, `yakogl_backend_v3d71`: the build's
//! `g_backend` when `gpu-v3d71` is on (src/backend.rs, `v3d_table`), in
//! place of the 4.2 table `yakogl_backend_v3d` (roadmap stage 5, P9;
//! docs/notes/v3d71-table71.md, docs/notes/v3d71-gl-table.md section 4).
//!
//! # What it records (plan items `table71-draw`, `table71-cases`)
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
//! * **clear quad**: a `glClear` the tile buffer cannot do (scissored, or
//!   under a partial colour or stencil mask: clearplan71's `Draw`) is
//!   fill71's clear quad, Mesa's blitter clear: a TRIANGLE_FAN of four
//!   vertices with the clear program, the 4.2 table's own source
//!   (`backend::CLEAR_VS`, `CLEAR_FS`: the position passes through, the
//!   colour is the uniform `u_color`). The program is built through the
//!   shader back end on the first quad (as an application's program is:
//!   compile, link, the code uploaded), checked as the 4.2 table checks its
//!   own (`u_color` at fragment dwords 0 to 3, render target 0 only, one
//!   attribute) and kept until shutdown. Each quad's four vertices
//!   (`FilledDraw::quad`) are written into the data arena at the flush, a
//!   64-byte slot a quad, and the quad's attribute record names its slot.
//!
//! # What this cut refuses by name
//!
//! * a clear quad when the clear program did not build
//!   ([`REF_CLEAR_PROGRAM`], the 4.2 table's string);
//! * what `backend71` does not assemble (8-bit and 32-bit indices,
//!   instanced, point size) and
//!   everything `glue71`, `fill71` and their modules refuse;
//! * what the texture path does not sample (texglue71's 3D and 2D array
//!   textures, table71tex's own; docs/notes/v3d71-refusals.md section 10);
//! * a depth/stencil the framebuffer keeps between jobs: the table keeps it
//!   in the tile buffer only (`glue71::DsPlace` at address 0), so a draw
//!   that reads what an earlier job left is zskeep71's refusal, the 4.2
//!   table's string (`src/v3d/backend.rs`, `fn record`).
//!
//! # gl_test's six cases (P9; tests/v3d71_table_cases.rs runs them)
//!
//! * 5 (scissor), 58 (clear with scissor), 66 (clear not bounded by the
//!   viewport), 102 (clear under a partial colour mask): recorded whole. The
//!   scissored and the masked clears are clear quads.
//! * 59 (clear with write masks): **refused by name**, as on the 4.2 table
//!   without a kept buffer. Its colour and depth parts and the stencil clear
//!   through the 0x0f mask (a quad with REPLACE through the mask) are
//!   recorded; the last draw tests stencil the job before the `glFinish`
//!   wrote, the window's stencil was in the tile buffer only, and zskeep71
//!   still refuses that read: "the draw reads stencil an earlier job wrote,
//!   and the target has no depth/stencil buffer to keep it in". The GL core
//!   gets -ENOTSUP (`GL_INVALID_OPERATION`) and the log line names it.
//! * 67 (FBO limits): **refused by the same name**, at its last frame (8):
//!   the `GL_STENCIL_INDEX8` framebuffer's stencil box is read across a
//!   `glFinish`. Frames 1 to 7 are recorded, the GL_RGB frame among them: the
//!   refusal "a colour format with fewer than 8 alpha bits" the Pi 4's first
//!   run met is no longer in the shared `state.rs` (tests/v3d71_refuse67.rs),
//!   so nothing produces it on either table.
//!
//! Both refusals end when the table keeps a D24S8 buffer per `ds_id`
//! (docs/notes/v3d71-table71.md section 2.1); that is not in this cut.
//!
//! # A product build records nothing yet
//!
//! The pure 7.1 modules are in the library only in a `host-test` build
//! (src/v3d/mod.rs: they build their results in `Vec`s and `String`s, and
//! the product library is `no_std` without an allocator). The recording
//! path is therefore `host-test` only too; in a product `gpu-v3d71` build
//! every draw and clear is still refused by [`REF_NO_RECORDING`]. `backend71`
//! and `backend71tex` are not in src/v3d/mod.rs either: they are included
//! here with `#[path]` until that file names them. When the recording path
//! leaves `host-test`, the 7.1 image must build the texture path's C too
//! (driver/src/v3d/v3d_tex71.c, v3d_tfu71.c and v3d_tiling_mesa.c; the 7.1
//! list of zephyr/CMakeLists.txt has v3d_job71.c only), as the host rig
//! does (tests/v3d71_table_rig/mod.rs).
//!
//! # Textures and mip chains (P10; src/v3d/table71tex.rs)
//!
//! A draw whose program reads texture words samples each unit through a
//! tiled copy of the GL core's storage (`table71tex::Textures`, at most
//! `MAX_COPIES` live): texglue71 and texfill71 give the two records'
//! values, fill71 gets the program's lists with the texture words masked
//! (`backend71tex::masked`), and the flush assembles a textured job with
//! `backend71tex::assemble_filled`, which places the records in the job's
//! record arena ([`RECS_BYTES`]) and hands the bindings to v3d_job71. A
//! unit read with no binding samples the black stand-in. An evicted copy is
//! marked dead and freed only once the queue is empty and no job is in
//! flight. `glGenerateMipmap` of a 2D RGBA8 texture is the TFU's
//! (`v3d_tfu71_pack`, `osal_v3d71_tfu`), the chain kept in the copy until
//! `return_mipmap_chain`. `samples_texel_copies` is true: a draw reads the
//! copy made when it was recorded.
//!
//! The hooks the design note leaves `None` in the first cut (depth/stencil
//! resolve and write, queries) are `None` here: the GL core then refuses by
//! name what needs them, or does it on the CPU.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::backend::*;

#[cfg(feature = "host-test")]
#[allow(unused_imports)]
use super::{
    admit71, attr71, clearplan71, ffi71, fill71, ident71, record71, rt71, texfill71, texuni71, uniform71, zskeep71,
};

/// The job assembly (its siblings are this module's imports above).
#[cfg(feature = "host-test")]
#[allow(dead_code)]
#[path = "backend71.rs"]
mod backend71;

/// The textured job assembly (P10): backend71's, with the sampler bindings.
#[cfg(feature = "host-test")]
#[allow(dead_code)]
#[path = "backend71tex.rs"]
mod backend71tex;

/// The table's name (`GL_RENDERER`'s backend half), as section 1 of
/// docs/notes/v3d71-table71.md names it.
pub const NAME: &core::ffi::CStr = c"Broadcom V3D 7.1 (Raspberry Pi 5)";

/// The refusal of every draw and clear in a product build, which has no
/// recording path yet (module docs): the log line is `yakogl/v3d71: draw
/// refused: ` followed by this.
pub const REF_NO_RECORDING: &str =
    "V3D 7.1: the table records no draw or clear in this build (the 7.1 modules are host-test only)";

/// A `glClear` that needs a clear quad (masked or scissored) when the clear
/// program did not build: the 4.2 table's string (src/v3d/backend.rs
/// `clear_program`). Why it did not build is logged once, when it is built.
pub const REF_CLEAR_PROGRAM: &str = "the clear program did not build, so a clear the TLB cannot do is not drawn";
/// A clear quad's vertices in the data arena: four of x, y, z, w.
pub const QUAD_BYTES: usize = 64;

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
/// A job's texture and sampler records (backend71tex's arena): 24 bytes
/// each, the texture record 32-byte aligned and the sampler 8-byte aligned.
pub const RECS_BYTES: usize = 64 * 1024;
/// The record bytes one sampled unit can take at most: a texture record
/// (32 with its alignment) and a sampler record (24, 8-aligned, up to 32).
pub const RECS_BYTES_PER_BINDING: usize = 64;

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
    use super::backend71tex::{self, Binding71, CTexBuilder71, DrawTex71, Lists71, TexBuilder71};
    use super::{
        BCL_BYTES_PER_DRAW, BCL_FIXED, DATA_BYTES, DEFAULT_DRAWS_PER_JOB, IND_BYTES_PER_DRAW, IND_FIXED, INITED,
        QUAD_BYTES, RCL_BYTES, RECS_BYTES, RECS_BYTES_PER_BINDING, REFUSAL_PREFIX, REFUSED, REF_CLEAR_PROGRAM,
    };
    use crate::backend::{GlesDraw, GlesTextureBind};
    use crate::osal::{
        self, OsalGpuMem, OSAL_MEM_DATA, OSAL_QUERY_V3D_DRAWS_PER_JOB, OSAL_QUERY_V3D_HUB_IDENT,
        OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_VPM_BYTES,
    };
    use crate::shaderc::{ShcShader, ShcV3dUniform, ShcV3dVs, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX, SHC_V3D_NO_RECORD};
    use crate::util::{Global, EINVAL, EIO, ENODEV, ENOMEM, ENOTSUP};
    use crate::v3d::backend::{CLEAR_FS, CLEAR_VS};
    use crate::v3d::kick::{V3dKick, V3D_KICK_MAGIC};
    use crate::v3d::table71tex::{CopyInfo, Textures};
    use crate::v3d::zskeep71::Known71;
    use crate::v3d::{attr71, ffi71, fill71, glue71, record71, state, texfill71, texuni71, uniform71};

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

    /// The clear quad's program (module docs).
    enum ClearProgram {
        NotBuilt,
        /// It did not build; why was logged, and every quad is refused.
        Failed,
        Built(Box<BuiltClear>),
    }

    struct BuiltClear {
        prog: Prog,
        /// The linked program's record (one reference, given back at
        /// shutdown) and its code where the V3D fetches it.
        fs: ShcShader,
        code: OsalGpuMem,
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
        /// A draw that samples textures: its bindings and the program's
        /// own lists (`prog`'s are masked for fill71, backend71tex's
        /// `masked`).
        tex: Option<Box<QTex>>,
    }

    /// The sampler bindings of a textured draw (P10).
    struct QTex {
        /// (unit, the records' values) per stage; the vertex stage's serve
        /// the coordinate shader too.
        fs: Vec<(u32, texfill71::Filled71)>,
        vs: Vec<(u32, texfill71::Filled71)>,
        /// The units the GlesDraw binds, read or not (glue71's TexInputs).
        fs_units: Vec<ffi71::V3dJob71Tmu>,
        vs_units: Vec<ffi71::V3dJob71Tmu>,
        /// The program's FS, VS and CS lists, texture words included.
        lists: [Vec<ffi71::V3dUniform>; 3],
        /// The copies sampled (their GPU addresses).
        copies: Vec<u32>,
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
        /// A job's texture and sampler records (backend71tex's arena).
        recs: OsalGpuMem,
        /// The tiled copies the TMU samples (P10).
        tex: Textures,
        /// Per submitted kick, the copies its draws sample (the last
        /// [`HISTORY`] kicks), for a test.
        sampled: Vec<(u32, Vec<u32>)>,
        clear_program: ClearProgram,
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
            recs: OsalGpuMem::ZERO,
            tex: Textures::NEW,
            sampled: Vec::new(),
            clear_program: ClearProgram::NotBuilt,
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
        /// yet (-ENOTSUP, `GL_INVALID_OPERATION` with the reason in the log,
        /// as `<module>: <reason>`). zskeep71's reasons are the 4.2 table's
        /// own strings (tests/v3d71_zskeep.rs pins them to backend.rs), so
        /// they are logged as the 4.2 table logs them, with no module in
        /// front: gl_test cases 59 and 67 name the same refusal on both
        /// boards.
        fn from(r: fill71::Refusal) -> Fail {
            let why = if r.module == "zskeep71" { r.reason } else { r.to_string() };
            Fail::Refused(why, -ENOTSUP)
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
        if let ClearProgram::Built(mut b) = core::mem::replace(&mut g.clear_program, ClearProgram::NotBuilt) {
            crate::shaderc::release_shader(&b.fs);
            osal::gpu_free(&mut b.code);
        }
        g.tex.free_all();
        for mem in [&mut g.bcl, &mut g.rcl, &mut g.ind, &mut g.data, &mut g.tile_alloc, &mut g.tile_state, &mut g.recs]
        {
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
            (&mut g.recs, RECS_BYTES),
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
    /// `code_gpu` is where the program's code was uploaded (`fs_usc_gpu`).
    fn program(code_gpu: u64, fs: &ShcShader) -> Result<Prog, Fail> {
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
            gpu32(code_gpu + off as u64)
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
        let mut q = Queued {
            d: *d,
            prog: None,
            attrs: Vec::new(),
            vs_words: Vec::new(),
            fs_words: Vec::new(),
            indices: None,
            tex: None,
        };
        if d.index_count == 0 {
            return Ok(q); /* a clear alone */
        }
        // SAFETY: the contract: null or a live linked shader.
        let Some(fs) = (unsafe { d.fs.as_ref() }) else {
            return Ok(q); /* glue71 names it: a draw with no linked program */
        };
        let mut prog = program(d.fs_usc_gpu, fs)?;
        let reads = |l: &[uniform71::Word]| l.iter().any(|w| tex_unit(w).is_some());
        if d.num_textures != 0
            || d.num_vs_textures != 0
            || reads(&prog.fs_uniforms)
            || reads(&prog.vs_uniforms)
            || reads(&prog.cs_uniforms)
        {
            q.tex = Some(Box::new(textures(g, d, &mut prog)?));
        }
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
        /* staged indices (client memory, or the GL core's u16 scratch): a
         * copy in the data arena for the job's life. backend71 assembles u16
         * indices only and names the other types (REF_INDEX_TYPE), so those
         * are not copied: the dummy stands in until the refusal */
        if !d.index_cpu.is_null() {
            let addr = if d.index_size == 2 { stage(g, d.index_cpu, d.index_bytes as usize)? } else { dummy };
            q.indices = Some(glue71::IndexPlace { addr, size: u64::from(d.index_bytes) });
        }
        q.prog = Some(prog);
        q.attrs = attrs;
        q.vs_words = words(d.vs_uniform_cpu, d.vs_uniform_dwords);
        q.fs_words = words(d.fs_uniform_cpu, d.fs_uniform_dwords);
        Ok(q)
    }

    /* ---- textures (P10) ---------------------------------------------------------------- */

    /// The sampler unit a texture word names (texuni71's rule), or `None`
    /// for a word that is not one.
    fn tex_unit(w: &uniform71::Word) -> Option<u32> {
        match w.contents {
            texuni71::TMU_CONFIG_P0 | texuni71::TMU_CONFIG_P1 => Some(w.data >> 24),
            texuni71::TEXTURE_WIDTH
            | texuni71::TEXTURE_HEIGHT
            | texuni71::TEXTURE_DEPTH
            | texuni71::TEXTURE_ARRAY_SIZE => Some(w.data),
            _ => None,
        }
    }

    /// Bind one stage's textures: each binding its program reads, through a
    /// copy (`table71tex`); `pinned` gathers the copies this draw holds.
    fn bind_stage(
        g: &mut State,
        binds: &[GlesTextureBind],
        lists: [&[uniform71::Word]; 2],
        pinned: &mut Vec<usize>,
        copies: &mut Vec<u32>,
    ) -> Result<Vec<(u32, texfill71::Filled71)>, Fail> {
        let t = g.table.ok_or(Fail::Lost(-ENODEV))?;
        let words = || lists.iter().flat_map(|l| l.iter());
        /* the units the program reads, in order of first use; a binding
         * nobody reads gets no record */
        let mut units: Vec<u32> = Vec::new();
        for u in words().filter_map(tex_unit) {
            if !units.contains(&u) {
                units.push(u);
            }
        }
        let mut out = Vec::with_capacity(units.len());
        for unit in units {
            /* a unit the GL core passes no binding for samples as an
             * incomplete texture: the black stand-in (GL ES 2.0 3.7.10) */
            let none = GlesTextureBind { unit: unit as u8, incomplete: 1, ..GlesTextureBind::ZERO };
            let b = binds.iter().find(|b| u32::from(b.unit) == unit).unwrap_or(&none);
            let p1 = words().find(|w| w.contents == texuni71::TMU_CONFIG_P1 && w.data >> 24 == unit).map(|w| w.data);
            let bound = g.tex.bind(b, pinned, p1, t.ident.ver, t.ident.iprev)?;
            pinned.push(bound.copy);
            copies.push(bound.gpu);
            out.push((unit, bound.filled));
        }
        Ok(out)
    }

    /// A textured draw's bindings; `prog`'s lists are masked for fill71 and
    /// kept whole in the result.
    fn textures(g: &mut State, d: &GlesDraw, prog: &mut Prog) -> Result<QTex, Fail> {
        let fs_binds = &d.textures[..(d.num_textures as usize).min(d.textures.len())];
        let vs_binds = &d.vs_textures[..(d.num_vs_textures as usize).min(d.vs_textures.len())];
        let (mut pinned, mut copies) = (Vec::new(), Vec::new());
        let fs = bind_stage(g, fs_binds, [&prog.fs_uniforms, &[]], &mut pinned, &mut copies)?;
        let vs = bind_stage(g, vs_binds, [&prog.vs_uniforms, &prog.cs_uniforms], &mut pinned, &mut copies)?;
        let own = |l: &[uniform71::Word]| -> Vec<ffi71::V3dUniform> {
            l.iter().map(|w| ffi71::V3dUniform { contents: w.contents, data: w.data }).collect()
        };
        let lists = [own(&prog.fs_uniforms), own(&prog.vs_uniforms), own(&prog.cs_uniforms)];
        prog.fs_uniforms = backend71tex::masked(&prog.fs_uniforms);
        prog.vs_uniforms = backend71tex::masked(&prog.vs_uniforms);
        prog.cs_uniforms = backend71tex::masked(&prog.cs_uniforms);
        let units = |bs: &[GlesTextureBind]| -> Vec<ffi71::V3dJob71Tmu> {
            bs.iter().map(|b| ffi71::V3dJob71Tmu { unit: u32::from(b.unit), ..Default::default() }).collect()
        };
        Ok(QTex { fs, vs, fs_units: units(fs_binds), vs_units: units(vs_binds), lists, copies })
    }

    /// The textured draw of each draw of `jobs` (`None` for a clear quad and
    /// an untextured draw): fill71 gives one draw per [`fill71::Op::Draw`],
    /// in order, and each queued GlesDraw with indices has one.
    fn tex_of<'q>(all: &[&'q Queued], jobs: &[fill71::FilledJob]) -> Vec<Vec<Option<&'q QTex>>> {
        let mut drawn = all.iter().filter(|q| q.d.index_count != 0 && q.prog.is_some());
        jobs.iter()
            .map(|j| {
                j.draws
                    .iter()
                    .map(|d| if d.is_clear { None } else { drawn.next().and_then(|q| q.tex.as_deref()) })
                    .collect()
            })
            .collect()
    }

    /// The bindings of one textured draw as backend71tex takes them.
    fn bindings(v: &[(u32, texfill71::Filled71)]) -> Vec<Binding71<'_>> {
        v.iter().map(|(unit, filled)| Binding71 { unit: *unit, filled }).collect()
    }

    /// What the flush will plan for the textured draws of `jobs`, run dry:
    /// a refusal of backend71tex (texuni71's words, a binding twice) is the
    /// draw's. `Ok(false)`: a job's records do not fit the record arena.
    fn check_textured(g: &State, all: &[&Queued], jobs: &[fill71::FilledJob]) -> Result<bool, fill71::Refusal> {
        let Some(table) = g.table else { return Ok(true) };
        let recs = u32::try_from(g.recs.gpu).unwrap_or(0);
        for (job, tex) in jobs.iter().zip(tex_of(all, jobs)) {
            let n: usize = tex.iter().flatten().map(|t| t.fs.len() + t.vs.len()).sum();
            if n * RECS_BYTES_PER_BINDING > RECS_BYTES {
                return Ok(false);
            }
            let mut arena = backend71tex::Arena71::new(recs, RECS_BYTES as u32)?;
            let mut records = backend71tex::Records71::default();
            for (fd, t) in job.draws.iter().zip(tex) {
                let Some(t) = t else { continue };
                let d = backend71::draw_of(fd)?;
                let d = backend71::Draw71 {
                    fs_uniforms: &t.lists[0],
                    vs_uniforms: &t.lists[1],
                    cs_uniforms: &t.lists[2],
                    ..d
                };
                let (fs, vs) = (bindings(&t.fs), bindings(&t.vs));
                let x = DrawTex71 { fs: &fs, vs: &vs, lists: None };
                backend71tex::plan_draw(
                    &table,
                    &d,
                    &x,
                    u32::try_from(g.ind.gpu).unwrap_or(0),
                    &mut arena,
                    &mut records,
                )?;
            }
        }
        Ok(true)
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
    /// the open frame, with what the builder path would refuse later:
    /// backend71's own refusals, and a clear quad without its program
    /// ([`REF_CLEAR_PROGRAM`]; [`NEEDS_CLEAR_PROGRAM`] while it has not been
    /// built, which the caller answers by building it).
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
            match &q.tex {
                /* fill71 gets the masked lists; the units named here are
                 * what glue71 checks against the GlesDraw */
                Some(t) => ops.extend(
                    glue71::ops_textured(
                        &q.d,
                        frame,
                        input.as_ref(),
                        &glue71::TexInputs { fs: &t.fs_units, vs: &t.vs_units },
                    )?
                    .ops,
                ),
                None => ops.extend(glue71::ops(&q.d, frame, input.as_ref())?),
            }
        }
        /* fill71 takes the clear quad's program for every flush. Until the
         * program is built this stand-in lets fill71 plan the flush: a job
         * that then holds a quad is refused below, so it is never drawn. */
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
        let built = match &g.clear_program {
            ClearProgram::Built(b) => Some(b.prog.view()),
            _ => None,
        };
        let quad = fill71::ClearQuad {
            program: built.as_ref().unwrap_or(&stand_in),
            /* the dummy here; the flush gives each quad its own slot
             * (`place_quads`) */
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
                if d.is_clear && built.is_none() {
                    let reason = match g.clear_program {
                        ClearProgram::NotBuilt => NEEDS_CLEAR_PROGRAM,
                        _ => REF_CLEAR_PROGRAM,
                    };
                    return Err(fill71::Refusal { module: "table71", reason: reason.to_string() });
                }
                backend71::draw_of(d)?;
            }
        }
        Ok(jobs)
    }

    /// [`fill`]'s answer for a clear quad before the clear program was
    /// built. Never logged: the caller builds the program and fills again.
    const NEEDS_CLEAR_PROGRAM: &str = "the clear program is not built yet";

    /// The clear quads of `jobs`.
    fn quads(jobs: &[fill71::FilledJob]) -> usize {
        jobs.iter().flat_map(|j| &j.draws).filter(|d| d.quad.is_some()).count()
    }

    /// Write each clear quad's vertices into the data arena and point the
    /// quad's attribute record at them. False: the arena is full (the dry
    /// run counted the quads, so it is not).
    fn place_quads(g: &mut State, job: &mut fill71::FilledJob) -> bool {
        for d in &mut job.draws {
            let Some(q) = d.quad else { continue };
            let Ok(at) = stage(g, q.as_ptr() as *const c_void, QUAD_BYTES) else {
                return false;
            };
            match d.attrs.first_mut() {
                Some(a) => a.address = at,
                None => return false,
            }
        }
        true
    }

    /* ---- the clear program ---------------------------------------------------------- */

    /// The text of a compiler log (NUL-terminated in `log`).
    fn log_text(log: &[core::ffi::c_char]) -> String {
        log.iter().take_while(|&&c| c != 0).map(|&c| c as u8 as char).collect()
    }

    /// Build the clear program and say once what happened: its code size, or
    /// the step that failed and the compiler's own error (the 4.2 table's
    /// `made_clear_program`).
    fn build_clear_program(g: &mut State) {
        let mut log = [0 as core::ffi::c_char; 256];
        g.clear_program = match clear_program(&mut log) {
            Ok(p) => {
                if let ClearProgram::Built(b) = &p {
                    glog!("yakogl/v3d71: clear program built: {} bytes of code", b.fs.usc_dw * 4);
                }
                p
            }
            Err(why) => {
                glog!("yakogl/v3d71: clear program did not build: {}: {}", why, log_text(&log));
                ClearProgram::Failed
            }
        };
    }

    /// Compile and link the clear program through the shader back end, as
    /// the GL core compiles an application's, and upload its code: the 4.2
    /// table's `build_clear_program` with its checks and strings (the
    /// colour at fragment dwords 0 to 3, which is where fill71 puts it;
    /// render target 0 and no other; one attribute, the quad's positions).
    fn clear_program(log: &mut [core::ffi::c_char; 256]) -> Result<ClearProgram, &'static str> {
        use core::ffi::c_char;
        let be = crate::shaderc::backend().ok_or("no shader back end")?;
        let (compile, link) = match (be.compile, be.link) {
            (Some(c), Some(l)) => (c, l),
            _ => return Err("the shader back end cannot compile and link"),
        };
        let (mut vs, mut fs) = (ShcShader::ZERO, ShcShader::ZERO);
        // SAFETY: NUL-terminated sources, live out-parameters, a log of the
        // size given.
        let rc = unsafe {
            compile(SHC_STAGE_VERTEX, CLEAR_VS.as_ptr() as *const c_char, &mut vs, log.as_mut_ptr(), log.len())
        };
        if rc != 0 {
            return Err("the vertex shader did not compile");
        }
        // SAFETY: as above.
        let rc = unsafe {
            compile(SHC_STAGE_FRAGMENT, CLEAR_FS.as_ptr() as *const c_char, &mut fs, log.as_mut_ptr(), log.len())
        };
        if rc != 0 {
            crate::shaderc::release_shader(&vs);
            return Err("the fragment shader did not compile");
        }
        // SAFETY: two compiled shaders; `fs` becomes the linked program.
        let rc = unsafe { link(&vs, &mut fs, log.as_mut_ptr(), log.len()) };
        crate::shaderc::release_shader(&vs);
        if rc == 0 {
            log[0] = 0; /* what fails below is ours, not the compiler's */
        }
        let give_back = |fs: &ShcShader| crate::shaderc::release_shader(fs);
        if rc != 0 || fs.v3d.linked != 1 {
            give_back(&fs);
            return Err("the program did not link");
        }
        let placed = fs
            .uniform_table()
            .iter()
            .any(|u| crate::util::name_bytes(&u.name) == b"u_color" && u.dword_off == 0 && u.dwords == 4);
        if !placed {
            give_back(&fs);
            return Err("a colour uniform is not at its four dwords of the fragment block");
        }
        if fs.v3d.fs.rt_mask != 1 {
            give_back(&fs);
            return Err("the program does not write exactly the colour targets of the clear");
        }
        /* the quad's one array is attribute record 0 (fill71's ClearQuad) */
        if fs.v3d.num_records != 1 || !fs.v3d.record_of_location.contains(&0) {
            give_back(&fs);
            return Err("the program does not read exactly one attribute");
        }
        let bytes = fs.usc_dw as usize * 4;
        let mut code = OsalGpuMem::ZERO;
        if bytes == 0 || fs.usc.is_null() || !alloc(&mut code, bytes) {
            if !code.impl_.is_null() || code.gpu != 0 {
                osal::gpu_free(&mut code);
            }
            give_back(&fs);
            return Err("no GPU memory for its code");
        }
        // SAFETY: the allocation is `bytes` long, and the program's code is
        // `usc_dw` dwords.
        unsafe {
            core::ptr::copy_nonoverlapping(fs.usc as *const u8, code.cpu as *mut u8, bytes);
            osal::cache_flush(code.cpu, bytes);
        }
        match program(code.gpu, &fs) {
            Ok(prog) => Ok(ClearProgram::Built(Box::new(BuiltClear { prog, fs, code }))),
            Err(_) => {
                give_back(&fs);
                osal::gpu_free(&mut code);
                Err("the linked program's code table is malformed")
            }
        }
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
        let mut jobs = match fill(g, &frame, ds_id, &queue, None) {
            Ok(j) => j,
            Err(r) => {
                /* the dry run admitted every draw of it */
                glog!("yakogl/v3d71: flush: the queued draws are refused ({}): not submitted", r);
                return -EIO;
            }
        };
        /* the quads' vertices, after the draws' arrays; every job of the
         * flush before the first is submitted (a job in flight reads the
         * arena, and nothing it reads is written again) */
        for job in &mut jobs {
            if !place_quads(g, job) {
                glog!("yakogl/v3d71: flush: the clear quads do not fit the job's data arena: not submitted");
                return -ENOMEM;
            }
        }
        let Some(table) = g.table else {
            return -ENODEV;
        };
        let all: Vec<&Queued> = queue.iter().collect();
        let texs = tex_of(&all, &jobs);
        let mut rc = 0;
        for (n, job) in jobs.iter().enumerate() {
            /* one arena set: the job in flight reads the lists */
            let waited = wait_in_flight(g);
            if waited < 0 {
                rc = waited;
            }
            let fb = frame.framebuffer(g.tile_alloc.gpu as u32);
            let tex = &texs[n];
            let sampled: Vec<u32> = tex.iter().flatten().flat_map(|t| t.copies.iter().copied()).collect();
            let assembled = if tex.iter().all(Option::is_none) {
                // SAFETY: the three arenas are live, apart, and nothing else
                // holds their mappings while the builder writes them.
                let mut b = unsafe { CBuilder71::new(list(&g.bcl), list(&g.rcl), list(&g.ind), backend71::PAGE_SIZE) };
                backend71::assemble_filled(&table, &fb, job, &mut b)
                    .map(|a| (a.built, [List71::Bcl, List71::Rcl, List71::Ind].map(|l| b.bytes(l).len()), 0))
            } else {
                /* a textured job: backend71tex places the records in the
                 * record arena and hands the bindings to the builder */
                let binds: Vec<(Vec<Binding71<'_>>, Vec<Binding71<'_>>)> = tex
                    .iter()
                    .map(|t| t.map_or((Vec::new(), Vec::new()), |t| (bindings(&t.fs), bindings(&t.vs))))
                    .collect();
                let draws: Vec<DrawTex71<'_>> = tex
                    .iter()
                    .zip(&binds)
                    .map(|(t, (fs, vs))| match t {
                        Some(t) => DrawTex71 {
                            fs,
                            vs,
                            lists: Some(Lists71 { fs: &t.lists[0], vs: &t.lists[1], cs: &t.lists[2] }),
                        },
                        None => DrawTex71::default(),
                    })
                    .collect();
                // SAFETY: the four arenas are live, apart, and nothing else
                // holds their mappings while the builders write them.
                let mut b = unsafe {
                    CTexBuilder71::new(list(&g.bcl), list(&g.rcl), list(&g.ind), list(&g.recs), backend71::PAGE_SIZE)
                };
                backend71tex::assemble_filled(&table, &fb, job, &draws, &mut b).map(|a| {
                    (a.built, [List71::Bcl, List71::Rcl, List71::Ind].map(|l| b.bytes(l).len()), b.arena_bytes().len())
                })
            };
            let (built, used, recs_used) = match assembled {
                Ok(a) => a,
                Err(r) => {
                    glog!("yakogl/v3d71: flush: the job does not assemble ({}): not submitted", r);
                    return -EIO;
                }
            };
            if built.tile_alloc_size as usize > g.tile_alloc.size || built.tile_state_size as usize > g.tile_state.size
            {
                glog!(
                    "yakogl/v3d71: flush: the job bins into {} + {} bytes, the buffers hold {} + {}: not submitted",
                    built.tile_alloc_size,
                    built.tile_state_size,
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
                osal::cache_flush(g.recs.cpu, recs_used);
            }
            let kick = V3dKick {
                magic: V3D_KICK_MAGIC,
                bcl_start: g.bcl.gpu as u32,
                bcl_end: g.bcl.gpu as u32 + used[0] as u32,
                rcl_start: g.rcl.gpu as u32,
                rcl_end: g.rcl.gpu as u32 + used[1] as u32,
                tile_alloc: g.tile_alloc.gpu as u32,
                tile_alloc_size: built.tile_alloc_size,
                tile_state: g.tile_state.gpu as u32,
                tile_state_size: built.tile_state_size,
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
            if g.sampled.len() >= HISTORY {
                g.sampled.remove(0);
            }
            g.sampled.push((g.kicks, sampled));
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
        /* nothing queued, nothing in flight: no draw names a dead copy */
        g.tex.free_dead();
        Ok(())
    }

    /// Whether `d` samples a texture whose rendering kick is still queued
    /// or in flight (`kick_pending`'s rule).
    fn renders_sampled(g: &State, d: &GlesDraw) -> bool {
        let fs = &d.textures[..(d.num_textures as usize).min(d.textures.len())];
        let vs = &d.vs_textures[..(d.num_vs_textures as usize).min(d.vs_textures.len())];
        fs.iter().chain(vs).any(|b| {
            let k = b.target_kick;
            k != 0
                && (k > g.kicks || g.in_flight.is_some_and(|f| (k >= f.kick || k == f.alias) && !fence_done(f.fence)))
        })
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
        /* glue71's checks but its "textures" one: the table samples them
         * (P10), glue71::ops_textured runs the rest again with the units */
        let mut probe = *d;
        (probe.num_textures, probe.num_vs_textures) = (0, 0);
        if let Some(r) = glue71::refusal(&probe) {
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
        /* a texture a job still renders into is copied from its storage
         * once that job has run */
        if renders_sampled(g, d) {
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
                Err(r) if r.reason == NEEDS_CLEAR_PROGRAM => {
                    /* the first clear quad: build its program, fill again */
                    build_clear_program(g);
                    continue;
                }
                Err(r) => {
                    g.data_used = mark;
                    return Err(r.into());
                }
            };
            let (bcl, ind) = list_bytes(&jobs);
            /* the flush writes each quad's vertices after the arrays */
            let data = g.data_used.next_multiple_of(16) + quads(&jobs) * QUAD_BYTES;
            let all: Vec<&Queued> = open.queue.iter().chain(Some(&q)).collect();
            let recs_fit = match check_textured(g, &all, &jobs) {
                Ok(f) => f,
                Err(r) => {
                    g.data_used = mark;
                    return Err(r.into());
                }
            };
            let fits = bcl + BCL_FIXED as usize <= g.bcl.size
                && ind + IND_FIXED as usize <= g.ind.size
                && data <= DATA_BYTES
                && recs_fit;
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
        if waited == 0 && g.open.as_ref().is_none_or(|o| o.queue.is_empty()) {
            g.tex.free_dead();
        }
        if rc < 0 {
            rc
        } else {
            waited
        }
    }

    /* ---- textures: the hooks (P10) -------------------------------------------------------- */

    /// glGenerateMipmap (`YakoglBackend::generate_mipmap`): the chain by the
    /// TFU in a copy (1), or 0. The jobs that render into the texture are
    /// submitted and waited for first.
    ///
    /// # Safety
    /// `b` is null or a live binding.
    pub unsafe fn generate_mipmap(b: *const GlesTextureBind, levels: u32) -> i32 {
        let g = g();
        // SAFETY: the caller's contract.
        let (true, Some(b), Some(t)) = (g.inited, unsafe { b.as_ref() }, g.table) else {
            return 0;
        };
        let rc = flush_queue(g);
        if rc < 0 || wait_in_flight(g) < 0 {
            return 0;
        }
        g.tex.free_dead();
        g.tex.generate(b, levels, t.ident.ver, t.ident.iprev)
    }

    /// # Safety
    /// `b` is null or a live binding.
    pub unsafe fn return_mipmap_chain(b: *const GlesTextureBind) {
        let g = g();
        // SAFETY: the caller's contract.
        if let (true, Some(b)) = (g.inited, unsafe { b.as_ref() }) {
            g.tex.return_chain(b);
        }
    }

    pub fn texture_released(base: u64, bytes: u64) {
        let g = g();
        if g.inited {
            g.tex.released(base, bytes);
        }
    }

    pub fn texture_copies() -> Vec<CopyInfo> {
        g().tex.info()
    }

    pub fn tfu_counts() -> (u32, u32) {
        let g = g();
        (g.tex.tfu_jobs, g.tex.tfu_failed)
    }

    pub fn sampled_by(kick: u32) -> Option<Vec<u32>> {
        g().sampled.iter().find(|s| s.0 == kick).map(|s| s.1.clone())
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

/// glGenerateMipmap through the 7.1 TFU (P10): 1 when the chain was made
/// (`MipChain::Whole`), 0 when the GL core filters on the CPU; always 0 in
/// a product build, which records nothing.
///
/// # Safety
/// `b` is null or a live binding for the call.
pub unsafe extern "C" fn v3d71_generate_mipmap(b: *const GlesTextureBind, levels: u32) -> i32 {
    #[cfg(feature = "host-test")]
    {
        // SAFETY: the caller's contract, passed on.
        unsafe { rec::generate_mipmap(b, levels) }
    }
    #[cfg(not(feature = "host-test"))]
    {
        let _ = (b, levels);
        0
    }
}

/// The chain the TFU made goes back into the GL core's storage.
///
/// # Safety
/// `b` is null or a live binding for the call.
pub unsafe extern "C" fn v3d71_return_mipmap_chain(b: *const GlesTextureBind, levels: u32) {
    let _ = levels;
    #[cfg(feature = "host-test")]
    // SAFETY: the caller's contract, passed on.
    unsafe {
        rec::return_mipmap_chain(b)
    };
    #[cfg(not(feature = "host-test"))]
    let _ = b;
}

/// Texel storage is released: the copies of it are dropped (once no job
/// can sample them).
pub extern "C" fn v3d71_texture_released(base_gpu: u64, bytes: u64) {
    #[cfg(feature = "host-test")]
    rec::texture_released(base_gpu, bytes);
    #[cfg(not(feature = "host-test"))]
    let _ = (base_gpu, bytes);
}

/// The tiled copies the table holds (a host test's view).
#[cfg(feature = "host-test")]
pub fn texture_copies() -> Vec<crate::v3d::table71tex::CopyInfo> {
    rec::texture_copies()
}

/// The TFU jobs run and failed since init (a host test's view).
#[cfg(feature = "host-test")]
pub fn tfu_counts() -> (u32, u32) {
    rec::tfu_counts()
}

/// The GPU addresses of the copies kick `kick` samples (a host test's
/// view; the last 64 kicks).
#[cfg(feature = "host-test")]
pub fn sampled_by(kick: u32) -> Option<Vec<u32>> {
    rec::sampled_by(kick)
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
    texture_released: Some(v3d71_texture_released),
    surface_swapped: None,
    texture_uploaded: None,
    /* backend71 refuses instancing (REF_INSTANCED): no divisor above 1 */
    max_instance_divisor: 1,
    wait_kick: Some(v3d71_wait_kick),
    /* a draw samples a copy made when it was recorded (table71tex) */
    samples_texel_copies: true,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: Some(v3d71_generate_mipmap),
    return_mipmap_chain: Some(v3d71_return_mipmap_chain),
    linear_row_align: 0,
    /* fill71 takes the uniform blocks as host words */
    uniforms_host_only: true,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};
