// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The V3D 7.1 (Raspberry Pi 5, BCM2712 D0, V3D 7.1.10) job assembly the 7.1
//! GL table calls (docs/notes/v3d71-gl-table.md sections 2 to 6): the init
//! identity check and, per frame, the admission, the clear plan, the
//! depth/stencil decision, the uniform streams, the D0 shader record and the
//! attribute records, then the C job builder `driver/src/v3d/v3d_job71.c`.
//!
//! Self-contained: no `crate::` path. The siblings are reached as
//! `super::{admit71, attr71, clearplan71, ffi71, ident71, record71, rt71,
//! uniform71, zskeep71}` (and, through admit71, `format71` and `limits71`):
//! the same names a test gives them when it includes the files with
//! `#[path]` at its crate root (tests/v3d71_backend71.rs). Nothing hooks this
//! module into `src/v3d/mod.rs` yet; that, the `gpu-v3d71` feature and the
//! table glue are the next item (backend71).
//!
//! # What it takes and what it leaves to others
//!
//! The GL frame and draw state arrive already mapped to the `ffi71`
//! descriptors (`V3dJob71Target`, `V3dJob71State`, the programs): that
//! mapping is `fill71`'s, not repeated here. Next to the descriptors the
//! caller gives the GL-level facts the pure modules decide on: the frame's
//! GL formats and buffer uses ([`admit71::Frame`]), the `glClear` that opens
//! the job ([`clearplan71::ClearIn`]), and the depth/stencil history and use
//! ([`ZsKeep71`]). [`assemble`] then
//!
//! 1. admits the frame (`admit71::admit_frame`: size, target count, formats,
//!    limits71's tile size) and checks the descriptor against it;
//! 2. plans the clear (`clearplan71::plan_clear`): a full clear is the
//!    tile-buffer clear (`Clear Render Targets`, packet 25) with each target's
//!    clear value packed by `rt71::clear_words71`; a masked or scissored
//!    clear (a clear draw) and a clear that must open a new job are refused by
//!    name here: they are the six gl_test shapes' work (cases71);
//! 3. decides the depth and stencil loads, stores and clear values
//!    (`zskeep71::decide`) into `V3dJob71Clear`;
//! 4. per draw: fills the three uniform streams (`uniform71::fill` with the
//!    program's uniform list, its GL uniform block and the viewport), plans the
//!    attribute records without a default-attribute BO (`attr71::plan` via
//!    `admit71::admit_draw`, the GFXH-930 dummy record at the indirect
//!    buffer's first byte) and describes the D0 draw-index record
//!    (`record71::describe` on the identity's revision);
//! 5. hands the descriptors to a [`Builder71`] (in the table the C builder
//!    through [`CBuilder71`]), and
//! 6. checks what the builder wrote against step 4: the tile size against
//!    limits71's, each uniform stream, each 32-byte record and each 16-byte
//!    attribute record byte for byte. A difference is a named refusal: the job
//!    must not be submitted.
//!
//! Every refusal is an [`admit71::Refusal`] (module and reason), the
//! modules' own text where they have one; this module's own are tagged
//! `backend71`, the builder's `v3d_job71`.

use super::{admit71, attr71, clearplan71, ffi71, ident71, record71, rt71, uniform71, zskeep71};

pub use admit71::Refusal;

/// The Pi 5 kernel's buffer granule: the tile allocation size is rounded up
/// to it (`v3d_job71_alloc_sizes`).
pub const PAGE_SIZE: u32 = 16 * 1024;

fn refuse(module: &'static str, reason: impl core::fmt::Display) -> Refusal {
    Refusal { module, reason: format!("{reason}") }
}

fn own(reason: impl core::fmt::Display) -> Refusal {
    refuse("backend71", reason)
}

// ---- init ------------------------------------------------------------------

/// What the table keeps from its init: the hub identity and the revision the
/// shader record is described for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Table71 {
    pub ident: ident71::HubIdent,
    pub rev: record71::Revision,
}

/// The table's init: `HUB_IDENT1` and `HUB_IDENT3` as the OSAL read them.
/// Only 7.1 with IPREV >= 10 is served; anything else is ident71's named
/// refusal ("V3D 4.2: build gpu-v3d", "V3D 7.1.5: this table is 7.1.10
/// (BCM2712 D0) only", ...). The refusal's `to_string()` is the message the
/// table reports.
pub fn init(hub_ident1: u32, hub_ident3: u32) -> Result<Table71, Refusal> {
    let ident = admit71::admit_ident(hub_ident1, hub_ident3)?;
    let rev = record71::Revision { ver: ident.ver, rev: ident.iprev };
    if !record71::has_draw_index_record(rev) {
        // ident71 and record71 state the same rule; a disagreement is a bug
        // in one of them, never a guess here
        return Err(refuse("record71", record71::Refusal::NoDrawIndexRecord { ver: rev.ver, rev: rev.rev }.message()));
    }
    Ok(Table71 { ident, rev })
}

// ---- the frame ---------------------------------------------------------------

/// The depth/stencil side of a job (zskeep71's inputs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZsKeep71 {
    pub target: zskeep71::Target71,
    pub z: zskeep71::Known71,
    pub s: zskeep71::Known71,
    pub job: zskeep71::Job71,
}

impl ZsKeep71 {
    /// A framebuffer without depth or stencil.
    pub const NONE: ZsKeep71 = ZsKeep71 {
        target: zskeep71::Target71 { layout: zskeep71::Layout71::TileOnly, depth: false, stencil: false },
        z: zskeep71::Known71::Untouched,
        s: zskeep71::Known71::Untouched,
        job: zskeep71::Job71::NONE,
    };
}

/// One job's frame.
#[derive(Clone, Debug)]
pub struct Frame71<'a> {
    /// The GL formats, buffer uses and clear colours (admission).
    pub gl: admit71::Frame<'a>,
    /// fill71's target descriptor for the same frame.
    pub target: ffi71::V3dJob71Target,
    /// The `glClear` that opens the job, if any.
    pub clear: Option<clearplan71::ClearIn>,
    pub zs: ZsKeep71,
}

/// What the frame steps decided.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramePlan71 {
    pub admitted: admit71::FrameAdmitted,
    /// The clear plan (`None`: the job has no `glClear`).
    pub clear_plan: Option<clearplan71::ClearPlan>,
    /// Bit n: render target n is cleared through the tile buffer.
    pub cleared_rts: u8,
    pub zs: zskeep71::FrameZs71,
    /// The builder's `struct v3d_job71_clear`.
    pub clear: ffi71::V3dJob71Clear,
}

/// Steps 1 to 3 of [`assemble`].
pub fn plan_frame(f: &Frame71<'_>) -> Result<FramePlan71, Refusal> {
    let admitted = admit71::admit_frame(&f.gl)?;
    let t = &f.target;
    if t.width != f.gl.width || t.height != f.gl.height || t.rts as usize != f.gl.colors.len() {
        return Err(own(format_args!(
            "the target descriptor ({}x{}, {} targets) is not the GL frame ({}x{}, {} targets)",
            t.width,
            t.height,
            t.rts,
            f.gl.width,
            f.gl.height,
            f.gl.colors.len()
        )));
    }

    let clear_plan = match &f.clear {
        Some(ci) => Some(clearplan71::plan_clear(ci).map_err(|e| refuse("clearplan71", e))?),
        None => None,
    };
    let (cleared_rts, plan_depth, plan_stencil) = match clear_plan {
        None | Some(clearplan71::ClearPlan::Nothing) => (0u8, false, false),
        Some(clearplan71::ClearPlan::TileClear { new_job: true, .. }) => {
            return Err(own("a clear after queued draws opens a new job: the caller ends the open job first"));
        }
        Some(clearplan71::ClearPlan::TileClear { targets, depth, stencil, new_job: false }) => {
            (targets, depth, stencil)
        }
        Some(clearplan71::ClearPlan::Draw(_)) => {
            return Err(own(
                "a masked, scissored or partial clear is a clear draw, which this table does not assemble yet",
            ));
        }
    };

    let mut clear = ffi71::V3dJob71Clear::default();
    for (i, c) in f.gl.colors.iter().enumerate() {
        if cleared_rts & (1 << i) != 0 {
            let color =
                c.clear_color.ok_or_else(|| own(format_args!("render target {i} is cleared without a colour")))?;
            let rt = rt71::rt_format71(c.gl_format).map_err(|e| refuse("rt71", e.as_str()))?;
            clear.color[i] = rt71::clear_words71(&rt, color).map_err(|e| refuse("rt71", e.as_str()))?.words;
        } else if c.uses.load {
            clear.load_rts |= 1 << i;
        }
    }

    let zs = &f.zs;
    if plan_depth != zs.job.z.tlb_clear.is_some() || plan_stencil != zs.job.s.tlb_clear.is_some() {
        return Err(own("the clear plan and the depth/stencil job disagree on what the tile buffer clears"));
    }
    let d = zskeep71::decide(&zs.target, zs.z, zs.s, &zs.job).map_err(|e| refuse("zskeep71", e))?;
    let fz = d.frame();
    clear.z = f32::from_bits(fz.z_clear_bits);
    clear.stencil = u32::from(fz.stencil_clear);
    clear.zs_load = fz.zs_load_z || fz.zs_load_s;
    clear.zs_store = fz.zs_store_z || fz.zs_store_s;
    let aspects = |z: bool, s: bool| {
        (if z { ffi71::V3D_JOB71_ZS_DEPTH } else { 0 }) | (if s { ffi71::V3D_JOB71_ZS_STENCIL } else { 0 })
    };
    clear.zs_load_aspects = aspects(fz.zs_load_z, fz.zs_load_s);
    clear.zs_store_aspects = aspects(fz.zs_store_z, fz.zs_store_s);
    clear.s_load = fz.stencil_load;
    clear.s_store = fz.stencil_store;
    clear.zs_cleared =
        matches!(d.z.start, zskeep71::Start71::Clear(_)) || matches!(d.s.start, zskeep71::Start71::Clear(_));
    if fz.zs_surface != (t.zs.address != 0) || fz.stencil_surface != (t.s.address != 0) {
        return Err(own("the depth/stencil decision and the target descriptor disagree on the buffers"));
    }
    Ok(FramePlan71 { admitted, clear_plan, cleared_rts, zs: fz, clear })
}

// ---- the draws ---------------------------------------------------------------

/// One draw, as fill71 and the program give it. The `uniforms` pointers of
/// the program structs are not read: the lists are the slices here.
#[derive(Clone, Copy, Debug)]
pub struct Draw71<'a> {
    pub st: ffi71::V3dJob71State,
    pub fs: ffi71::V3dFsProg,
    pub vs: ffi71::V3dVsProg,
    pub cs: ffi71::V3dVsProg,
    /// The programs' uniform lists, `(contents, data)` in stream order.
    pub fs_uniforms: &'a [ffi71::V3dUniform],
    pub vs_uniforms: &'a [ffi71::V3dUniform],
    pub cs_uniforms: &'a [ffi71::V3dUniform],
    /// The GL uniform block words of the fragment and the vertex stages (the
    /// coordinate shader reads the vertex stage's).
    pub fs_gl: &'a [u32],
    pub vs_gl: &'a [u32],
    /// Vertex attribute arrays (float), record i is the programs'
    /// `vattr_sizes[i]`.
    pub attrs: &'a [ffi71::V3dVertexAttr],
    /// `V3D_PRIM_*`, vertex count and first vertex (arrays, one instance).
    pub prim: u32,
    pub count: u32,
    pub first: u32,
}

/// What step 4 predicts the builder writes for one draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawPlan71 {
    /// The FS, VS and CS uniform streams.
    pub streams: [Vec<u32>; 3],
    pub attrs: attr71::Plan71,
    /// The record's description, uniforms addresses 0 (the builder places
    /// the streams; [`check_draw`] fills them in).
    pub desc: record71::DrawDesc,
}

fn stream(list: &[ffi71::V3dUniform], vp: &uniform71::Viewport, gl: &[u32]) -> Result<Vec<u32>, Refusal> {
    let words: Vec<uniform71::Word> =
        list.iter().map(|u| uniform71::Word { contents: u.contents, data: u.data }).collect();
    let mut out = vec![0u32; words.len()];
    uniform71::fill(&words, vp, gl, &mut out).map_err(|e| refuse("uniform71", e))?;
    Ok(out)
}

fn vertex_desc(p: &ffi71::V3dVsProg) -> record71::VertexDesc {
    record71::VertexDesc {
        prog: record71::ProgramDesc {
            code: p.code,
            uniforms: 0,
            threads: p.threads,
            single_seg: p.single_seg,
            propagate_nans: false,
        },
        vpm_input_size: p.vpm_input_size,
        vpm_output_size: p.vpm_output_size,
        // v3d_compute_vpm_config without a GS, as v3d_job71.c writes them
        min_input_segments: 1,
        min_output_segments: 0,
        uses_vid: p.uses_vid,
        uses_iid: p.uses_iid,
        uses_base_instance: p.uses_biid,
        uses_base_vertex: false,
        uses_draw_index: false,
    }
}

/// Step 4 of [`assemble`] for one draw. `dummy_addr` is the indirect
/// buffer's first byte (the GFXH-930 dummy record's address at 7.1).
pub fn plan_draw(table: &Table71, d: &Draw71<'_>, dummy_addr: u32) -> Result<DrawPlan71, Refusal> {
    let vp = uniform71::Viewport { scale: d.st.vp.scale, translate: d.st.vp.translate };
    let streams = [
        stream(d.fs_uniforms, &vp, d.fs_gl)?,
        stream(d.vs_uniforms, &vp, d.vs_gl)?,
        stream(d.cs_uniforms, &vp, d.vs_gl)?,
    ];
    if d.vs.uses_biid || d.cs.uses_biid {
        return Err(own("a program reads the base instance, which v3d_job71 does not write"));
    }
    let attrs: Vec<attr71::Attr> = d
        .attrs
        .iter()
        .map(|a| attr71::Attr {
            enabled: true,
            components: a.components,
            gl_type: attr71::GL_FLOAT,
            stride: u64::from(a.stride),
            buffer_addr: a.address,
            divisor: a.divisor,
            ..attr71::Attr::disabled()
        })
        .collect();
    let desc = record71::DrawDesc {
        fs: record71::FragmentDesc {
            prog: record71::ProgramDesc {
                code: d.fs.code,
                uniforms: 0,
                threads: d.fs.threads,
                single_seg: d.fs.single_seg,
                propagate_nans: false,
            },
            writes_z: d.fs.writes_z,
            disable_ez: d.fs.disable_ez,
            num_varyings: d.fs.num_inputs,
            uses_center_w: d.fs.uses_center_w,
            uses_pid: d.fs.uses_pid,
            lock_scoreboard_on_first_thrsw: d.fs.lock_scoreboard_on_first_thrsw,
            uses_implicit_point_line_varyings: d.fs.uses_implicit_point_line_varyings,
            sample_rate_shading: false,
        },
        vs: vertex_desc(&d.vs),
        cs: vertex_desc(&d.cs),
        point_size: false,
    };
    let admitted = admit71::admit_draw(
        table.rev,
        &admit71::Draw {
            attrs: &attrs,
            cs_reads: d.cs.vattr_sizes,
            vs_reads: d.vs.vattr_sizes,
            cs_builtins: d.cs.uses_vid || d.cs.uses_iid,
            dummy_addr,
            separate_segments: d.vs.separate_segments || d.cs.separate_segments,
            programs: desc,
        },
    )?;
    Ok(DrawPlan71 { streams, attrs: admitted.attrs, desc })
}

// ---- the builder -------------------------------------------------------------

/// The three control lists of a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List71 {
    /// The binning list.
    Bcl,
    /// The render list.
    Rcl,
    /// The indirect buffer: uniform streams, records, the generic tile list.
    Ind,
}

/// Where the builder put one draw's data (`struct v3d_job71_draw_layout`'s
/// addresses).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawLayout71 {
    pub fs_uniforms: u32,
    pub vs_uniforms: u32,
    pub cs_uniforms: u32,
    pub record: u32,
    pub attr_records: u32,
}

/// What the builder hands back for a recorded job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built71 {
    pub tiling: ffi71::V3dTiling,
    pub layouts: Vec<DrawLayout71>,
    /// The generic tile list in the indirect buffer, `[start, end)`.
    pub tile_list: (u32, u32),
    pub tile_alloc_size: u32,
    pub tile_state_size: u32,
}

/// Why the builder did not record the job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError71 {
    /// `v3d_job71_begin` refused the target (or a list overflowed).
    Begin,
    /// `v3d_job71_draw` returned `code` (`V3D_JOB71_FULL`, `_REFUSED`) for
    /// draw `index`.
    Draw { index: usize, code: i32 },
    /// `v3d_job71_end` refused the job.
    End,
}

impl core::fmt::Display for BuildError71 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            BuildError71::Begin => write!(f, "v3d_job71_begin refused the target"),
            BuildError71::Draw { index, code: ffi71::V3D_JOB71_FULL } => {
                write!(f, "draw {index}: a control list is full (end the job, draw into a new one)")
            }
            BuildError71::Draw { index, code } => write!(f, "draw {index}: v3d_job71_draw refused it ({code})"),
            BuildError71::End => write!(f, "v3d_job71_end refused the job"),
        }
    }
}

/// The C job builder's three calls, over memory the implementation owns.
/// [`CBuilder71`] is the in-process FFI; a host test runs the same C file in
/// a child process.
pub trait Builder71 {
    /// The GPU address of a list's first byte.
    fn gpu(&self, l: List71) -> u32;
    /// `v3d_job71_begin`, one `v3d_job71_draw` per draw, `v3d_job71_end`
    /// and `v3d_job71_alloc_sizes`.
    fn build(
        &mut self,
        target: &ffi71::V3dJob71Target,
        draws: &[Draw71<'_>],
        clear: &ffi71::V3dJob71Clear,
    ) -> Result<Built71, BuildError71>;
    /// The bytes the last [`Builder71::build`] wrote into a list.
    fn bytes(&self, l: List71) -> &[u8];
}

/// The in-process C builder over three caller-owned CPU mappings of GPU
/// memory. Not exercised by the host test (which cannot link C into a test
/// binary without a build script); the table's hook links it.
pub struct CBuilder71<'m> {
    mem: [&'m mut [u8]; 3],
    gpu: [u32; 3],
    used: [u32; 3],
    page_size: u32,
}

impl<'m> CBuilder71<'m> {
    /// `bcl`, `rcl`, `ind`: CPU views and their GPU addresses. `page_size`:
    /// the tile allocation granule ([`PAGE_SIZE`] on the Pi 5's kernel).
    pub fn new(
        bcl: (&'m mut [u8], u32),
        rcl: (&'m mut [u8], u32),
        ind: (&'m mut [u8], u32),
        page_size: u32,
    ) -> CBuilder71<'m> {
        CBuilder71 { gpu: [bcl.1, rcl.1, ind.1], mem: [bcl.0, rcl.0, ind.0], used: [0; 3], page_size }
    }
}

fn ix(l: List71) -> usize {
    match l {
        List71::Bcl => 0,
        List71::Rcl => 1,
        List71::Ind => 2,
    }
}

impl Builder71 for CBuilder71<'_> {
    fn gpu(&self, l: List71) -> u32 {
        self.gpu[ix(l)]
    }

    fn build(
        &mut self,
        target: &ffi71::V3dJob71Target,
        draws: &[Draw71<'_>],
        clear: &ffi71::V3dJob71Clear,
    ) -> Result<Built71, BuildError71> {
        let mut cls: [ffi71::V3dCl; 3] = core::array::from_fn(|i| {
            ffi71::V3dCl::new(self.mem[i].as_mut_ptr(), self.gpu[i], self.mem[i].len() as u32)
        });
        let [bcl, rcl, ind] = &mut cls;
        let mut j = ffi71::V3dJob71::ZERO;
        self.used = [0; 3];
        // SAFETY: every pointer is to a live local or to the caller's
        // mappings, which outlive the calls; the builder writes only within
        // each list's `size`.
        if !unsafe { ffi71::v3d_job71_begin(&mut j, target, bcl, rcl, ind) } {
            return Err(BuildError71::Begin);
        }
        let mut layouts = Vec::with_capacity(draws.len());
        for (index, d) in draws.iter().enumerate() {
            let mut fs = d.fs;
            fs.uniforms = d.fs_uniforms.as_ptr();
            fs.num_uniforms = d.fs_uniforms.len() as u32;
            let mut vs = d.vs;
            vs.uniforms = d.vs_uniforms.as_ptr();
            vs.num_uniforms = d.vs_uniforms.len() as u32;
            let mut cs = d.cs;
            cs.uniforms = d.cs_uniforms.as_ptr();
            cs.num_uniforms = d.cs_uniforms.len() as u32;
            let draw = ffi71::V3dJob71Draw {
                st: &d.st,
                fs: &fs,
                vs: &vs,
                cs: &cs,
                vs_storage: ffi71::V3dUniformStorage { words: d.vs_gl.as_ptr(), count: d.vs_gl.len() as u32 },
                fs_storage: ffi71::V3dUniformStorage { words: d.fs_gl.as_ptr(), count: d.fs_gl.len() as u32 },
                attrs: d.attrs.as_ptr(),
                num_attrs: d.attrs.len() as u32,
                prim: d.prim,
                count: d.count,
                first: d.first,
                indexed: false,
                index_type: 0,
                index_buffer: 0,
                index_buffer_bytes: 0,
                index_offset: 0,
                tmu: core::ptr::null(),
                num_tmu: 0,
                vs_tmu: core::ptr::null(),
                num_vs_tmu: 0,
                vs_writes_psiz: false,
                instances: 0,
            };
            let mut lo = ffi71::V3dJob71DrawLayout::ZERO;
            // SAFETY: as above; `draw` and what it points to live past the call.
            let code = unsafe { ffi71::v3d_job71_draw(&mut j, &draw, &mut lo) };
            if code != ffi71::V3D_JOB71_OK {
                return Err(BuildError71::Draw { index, code });
            }
            layouts.push(DrawLayout71 {
                fs_uniforms: lo.fs_uniforms,
                vs_uniforms: lo.vs_uniforms,
                cs_uniforms: lo.cs_uniforms,
                record: lo.record,
                attr_records: lo.attr_records,
            });
        }
        let (mut ls, mut le) = (0u32, 0u32);
        // SAFETY: as above.
        if !unsafe { ffi71::v3d_job71_end(&mut j, clear, &mut ls, &mut le) } {
            return Err(BuildError71::End);
        }
        let (mut qms, mut qts) = (0u32, 0u32);
        // SAFETY: `j` is the job just ended.
        unsafe { ffi71::v3d_job71_alloc_sizes(&j, self.page_size, &mut qms, &mut qts) };
        self.used = [cls[0].used, cls[1].used, cls[2].used];
        Ok(Built71 { tiling: j.tiling, layouts, tile_list: (ls, le), tile_alloc_size: qms, tile_state_size: qts })
    }

    fn bytes(&self, l: List71) -> &[u8] {
        &self.mem[ix(l)][..self.used[ix(l)] as usize]
    }
}

// ---- the job -----------------------------------------------------------------

/// An assembled job: the decisions and the builder's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Assembled71 {
    pub frame: FramePlan71,
    pub draws: Vec<DrawPlan71>,
    pub built: Built71,
}

fn at(ind: &[u8], base: u32, addr: u32, len: usize) -> Option<&[u8]> {
    let off = addr.checked_sub(base)? as usize;
    ind.get(off..off + len)
}

fn words(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// Step 6 for one draw: the builder's streams, record and attribute records
/// against the plan. `ind` is the indirect buffer from `ind_gpu`.
pub fn check_draw(
    table: &Table71,
    index: usize,
    plan: &DrawPlan71,
    lo: &DrawLayout71,
    ind: &[u8],
    ind_gpu: u32,
) -> Result<(), Refusal> {
    let names = ["fragment", "vertex", "coordinate"];
    let addrs = [lo.fs_uniforms, lo.vs_uniforms, lo.cs_uniforms];
    for s in 0..3 {
        let want = &plan.streams[s];
        let got = at(ind, ind_gpu, addrs[s], 4 * want.len()).map(words);
        if got.as_deref() != Some(want.as_slice()) {
            return Err(own(format_args!(
                "draw {index}: v3d_job71 wrote a {} uniform stream other than uniform71's",
                names[s]
            )));
        }
    }
    let mut desc = plan.desc;
    desc.fs.prog.uniforms = lo.fs_uniforms;
    desc.vs.prog.uniforms = lo.vs_uniforms;
    desc.cs.prog.uniforms = lo.cs_uniforms;
    let rec = record71::pack_draw(table.rev, &desc).map_err(|e| refuse("record71", e.message()))?;
    if at(ind, ind_gpu, lo.record, record71::RECORD_SIZE) != Some(&rec[..]) {
        return Err(own(format_args!("draw {index}: v3d_job71 wrote a shader record other than record71's")));
    }
    for i in 0..plan.attrs.count {
        let want = attr71::pack(&plan.attrs.records[i]);
        let addr = lo.attr_records + (i * attr71::RECORD_BYTES) as u32;
        if at(ind, ind_gpu, addr, attr71::RECORD_BYTES) != Some(&want[..]) {
            return Err(own(format_args!("draw {index}: v3d_job71 wrote attribute record {i} other than attr71's")));
        }
    }
    Ok(())
}

/// One job: the frame, its draws in order, through `builder`. Steps 1 to 6
/// of the module docs; the first refusal ends it (nothing may then be
/// submitted).
pub fn assemble(
    table: &Table71,
    frame: &Frame71<'_>,
    draws: &[Draw71<'_>],
    builder: &mut dyn Builder71,
) -> Result<Assembled71, Refusal> {
    let fp = plan_frame(frame)?;
    let ind_gpu = builder.gpu(List71::Ind);
    let plans = draws.iter().map(|d| plan_draw(table, d, ind_gpu)).collect::<Result<Vec<_>, _>>()?;
    let built = builder.build(&frame.target, draws, &fp.clear).map_err(|e| refuse("v3d_job71", e))?;
    let tile = (built.tiling.tile_w, built.tiling.tile_h);
    if tile != fp.admitted.tile {
        return Err(own(format_args!(
            "v3d_job71 chose a {}x{} tile, limits71 {}x{}",
            tile.0, tile.1, fp.admitted.tile.0, fp.admitted.tile.1
        )));
    }
    if built.layouts.len() != draws.len() {
        return Err(own("v3d_job71 recorded another number of draws"));
    }
    let ind = builder.bytes(List71::Ind);
    for (i, (p, lo)) in plans.iter().zip(&built.layouts).enumerate() {
        check_draw(table, i, p, lo, ind, ind_gpu)?;
    }
    Ok(Assembled71 { frame: fp, draws: plans, built })
}
