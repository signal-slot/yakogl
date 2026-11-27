// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The textured half of the V3D 7.1 job assembly (P10 of
//! docs/notes/pre-study-rpi5.md; docs/notes/v3d71-gl-table.md section 3):
//! what `backend71` does for an untextured draw, for a draw that samples
//! textures.
//!
//! Self-contained: no `crate::` path. The siblings are reached as
//! `super::{admit71, backend71, ffi71, fill71, texfill71, texuni71,
//! uniform71}`, the
//! names a test gives them when it includes the files with `#[path]` at its
//! crate root (tests/v3d71_backend71_tex.rs). Nothing hooks this module into
//! `src/v3d/mod.rs` yet (the item backend71 does).
//!
//! # What it takes
//!
//! A draw as `backend71` takes it ([`backend71::Draw71`], or a
//! [`fill71::FilledDraw`]) and, per stage, its sampler bindings: the unit the
//! program's uniform names and the texture and sampler values already
//! composed ([`texfill71::Filled71`], from `texture71` and `sampler71`). The
//! level layout of a mipmapped texture is `texture71`'s, the one
//! `tfuplan71` generates the levels into (tests/v3d71_slices.rs).
//!
//! # What it does
//!
//! 1. allocates, per distinct texture, one TEXTURE_SHADER_STATE record
//!    (24 bytes, 32-byte aligned) and, per distinct sampler, one
//!    SAMPLER_STATE record (24 bytes, 8-byte aligned) in the job's
//!    [`Arena71`]: Mesa's alignments (`texuni71::TEX_STATE_ALIGN`,
//!    `SAMPLER_STATE_ALIGN`). Equal values share a record within a job, as
//!    Mesa's one record per sampler view and per sampler does;
//! 2. gives each binding its `struct v3d_job71_tmu` (the record addresses
//!    and the base level's size), the C builder's input for the TMU
//!    configuration words (`driver/src/v3d/v3d_job71.c`, `tmu_config`);
//! 3. predicts the uniform streams: the words `uniform71` refuses by name
//!    (contents 40, 41 and 44 to 47) are `texuni71::word`'s, every other one
//!    `uniform71`'s. This is done by handing `backend71::plan_draw` the
//!    lists with each texture word replaced by the constant it must be, so
//!    the shader record and the attribute records are planned by the same
//!    code as an untextured draw's;
//! 4. hands the draws, the bindings and the records to a [`TexBuilder71`]
//!    (in the table the C builders through [`CTexBuilder71`]), and
//! 5. checks what the builder wrote with `backend71::check_draw`: each
//!    uniform stream (the TMU words included), record and attribute record,
//!    and that every texture and sampler record lies in the arena.
//!
//! Every refusal is a [`backend71::Refusal`]; this module's own are tagged
//! `backend71tex`, the other modules' carry their own text (`texuni71`,
//! `uniform71`, ...).
//!
//! # fill71 and the texture words
//!
//! `fill71::fill` runs `uniform71::fill` over every list of a draw and so
//! refuses a program with texture words. Until fill71 leaves those words to
//! this module, a textured program is given to it with [`masked`] lists and
//! [`assemble_filled`] takes the program's own in [`DrawTex71::lists`]; it
//! refuses lists that are not each other's ([`REF_LISTS`]) and a binding no
//! uniform reads ([`REF_UNREAD`]).
//!
//! # Reused and duplicated
//!
//! Reused from `backend71` unchanged: `Table71`, `Draw71`, `DrawPlan71`,
//! `plan_draw`, `check_draw`, `draw_of`, `admit_framebuffer`, `Built71`,
//! `DrawLayout71`, `List71` and `BuildError71`. Duplicated (about 80 lines):
//! the FFI calls of `backend71::CBuilder71::build`, because its
//! `Builder71::build` has no sampler bindings to pass.

use super::{admit71, backend71, ffi71, fill71, texfill71, texuni71, uniform71};

pub use backend71::{Built71, Draw71, DrawLayout71, DrawPlan71, List71, Refusal, Table71};

/// TEXTURE_SHADER_STATE and SAMPLER_STATE at 7.1, bytes.
pub const TEX_RECORD_BYTES: u32 = ffi71::V3D_TEX71_STATE_BYTES as u32;
pub const SAMPLER_RECORD_BYTES: u32 = ffi71::V3D_TEX71_SAMPLER_BYTES as u32;

fn own(reason: impl core::fmt::Display) -> Refusal {
    Refusal { module: "backend71tex", reason: format!("{reason}") }
}

fn refuse(module: &'static str, reason: impl core::fmt::Display) -> Refusal {
    Refusal { module, reason: format!("{reason}") }
}

/// This module's own refusals.
pub const REF_ARENA_FULL: &str = "the job's texture record arena is full (end the job, draw into a new one)";
pub const REF_BINDINGS: &str = "the sampler bindings are not one entry per draw of the job";
pub const REF_UNIT_TWICE: &str = "a stage binds the same sampler unit twice";
pub const REF_LISTS: &str = "a draw's uniform lists are not its program's lists masked for fill71";
pub const REF_UNREAD: &str =
    "a sampler binding that no uniform of its stage reads (masked lists without the program's own?)";
pub const REF_RECORD_OUTSIDE: &str = "the builder did not write a texture or sampler record inside the arena";

// ---- the arena ---------------------------------------------------------------

/// The job's record arena: GPU memory the texture and sampler records of one
/// job are allocated from, front to back. The caller owns the memory and
/// keeps it until the job has run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arena71 {
    gpu: u32,
    size: u32,
    used: u32,
}

impl Arena71 {
    /// `gpu`: the address of the first byte (32-byte aligned, so that the
    /// record alignments hold in GPU addresses), `size` bytes.
    pub fn new(gpu: u32, size: u32) -> Result<Arena71, Refusal> {
        if gpu == 0 || !gpu.is_multiple_of(texuni71::TEX_STATE_ALIGN) || gpu.checked_add(size).is_none() {
            return Err(own(format_args!("the record arena at {gpu:#x} (+{size:#x}) is not a 32-byte aligned range")));
        }
        Ok(Arena71 { gpu, size, used: 0 })
    }

    pub fn gpu(&self) -> u32 {
        self.gpu
    }

    pub fn used(&self) -> u32 {
        self.used
    }

    /// `bytes` at `align` (a power of two): the GPU address.
    pub fn alloc(&mut self, bytes: u32, align: u32) -> Result<u32, Refusal> {
        let start = self.used.checked_next_multiple_of(align).ok_or_else(|| own(REF_ARENA_FULL))?;
        let end = start.checked_add(bytes).ok_or_else(|| own(REF_ARENA_FULL))?;
        if end > self.size {
            return Err(own(REF_ARENA_FULL));
        }
        self.used = end;
        Ok(self.gpu + start)
    }
}

/// The records of one job: where each goes and the values the C packers
/// (`v3d_tex71_pack_state`, `v3d_tex71_pack_sampler`) take.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Records71 {
    pub tex: Vec<(u32, ffi71::V3dTex71State)>,
    pub samplers: Vec<(u32, ffi71::V3dTex71Sampler)>,
}

impl Records71 {
    fn tex(&mut self, arena: &mut Arena71, s: &ffi71::V3dTex71State) -> Result<u32, Refusal> {
        if let Some((a, _)) = self.tex.iter().find(|(_, v)| v == s) {
            return Ok(*a);
        }
        let a = arena.alloc(TEX_RECORD_BYTES, texuni71::TEX_STATE_ALIGN)?;
        self.tex.push((a, *s));
        Ok(a)
    }

    fn sampler(&mut self, arena: &mut Arena71, s: &ffi71::V3dTex71Sampler) -> Result<u32, Refusal> {
        // bit patterns, not f32 equality: a NaN LOD must not make every
        // lookup miss (the packer refuses it later, by name)
        let key = |v: &ffi71::V3dTex71Sampler| {
            (
                [v.wrap_s, v.wrap_t, v.wrap_r, v.compare_func, v.max_anisotropy, v.border_color_mode],
                [v.min_nearest, v.mag_nearest, v.mip_nearest, v.mip_none],
                [v.min_lod.to_bits(), v.max_lod.to_bits(), v.lod_bias.to_bits()],
                v.border_color,
            )
        };
        if let Some((a, _)) = self.samplers.iter().find(|(_, v)| key(v) == key(s)) {
            return Ok(*a);
        }
        let a = arena.alloc(SAMPLER_RECORD_BYTES, texuni71::SAMPLER_STATE_ALIGN)?;
        self.samplers.push((a, *s));
        Ok(a)
    }
}

// ---- the draws ---------------------------------------------------------------

/// One sampler binding of a stage: the unit the program's uniforms name
/// (bits 24..31 of a TMU config uniform's data, the data of a size uniform)
/// and the composed texture and sampler.
#[derive(Clone, Copy, Debug)]
pub struct Binding71<'a> {
    pub unit: u32,
    pub filled: &'a texfill71::Filled71,
}

/// A program's three uniform lists as it has them (texture words included).
#[derive(Clone, Copy, Debug)]
pub struct Lists71<'a> {
    pub fs: &'a [ffi71::V3dUniform],
    pub vs: &'a [ffi71::V3dUniform],
    pub cs: &'a [ffi71::V3dUniform],
}

/// The sampler bindings of one draw: the fragment stage's, and the vertex
/// stage's (the coordinate shader reads the vertex stage's).
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawTex71<'a> {
    pub fs: &'a [Binding71<'a>],
    pub vs: &'a [Binding71<'a>],
    /// The program's own lists, for a draw that came through `fill71` with
    /// [`masked`] lists (see there). `None`: the draw's lists are the
    /// program's.
    pub lists: Option<Lists71<'a>>,
}

/// A uniform list as `fill71::fill` accepts it: every texture word a
/// `CONSTANT` 0. `fill71` runs `uniform71::fill` over each list of a draw,
/// which refuses the texture words by name, so a textured program reaches
/// `fill71` with masked lists and [`assemble_filled`] is given the program's
/// own in [`DrawTex71::lists`] (it checks that the two agree word for word
/// outside the texture words). Nothing else of the `FilledDraw` depends on
/// the lists.
pub fn masked(list: &[uniform71::Word]) -> Vec<uniform71::Word> {
    list.iter()
        .map(
            |w| {
                if is_texture_word(w.contents) {
                    uniform71::Word { contents: uniform71::CONSTANT, data: 0 }
                } else {
                    *w
                }
            },
        )
        .collect()
}

fn unmasked<'a>(
    filled: &[ffi71::V3dUniform],
    own_list: &'a [ffi71::V3dUniform],
) -> Result<&'a [ffi71::V3dUniform], Refusal> {
    let same = filled.len() == own_list.len()
        && filled.iter().zip(own_list).all(|(f, o)| {
            if is_texture_word(o.contents) {
                f.contents == uniform71::CONSTANT && f.data == 0
            } else {
                f == o
            }
        });
    if same {
        Ok(own_list)
    } else {
        Err(own(REF_LISTS))
    }
}

/// What steps 1 to 3 decide for one draw.
#[derive(Clone, Debug, PartialEq)]
pub struct TexDrawPlan71 {
    /// The builder's `struct v3d_job71_tmu` arrays (FS; VS and CS).
    pub fs_tmu: Vec<ffi71::V3dJob71Tmu>,
    pub vs_tmu: Vec<ffi71::V3dJob71Tmu>,
    /// The streams (TMU words included), attribute records and record
    /// description, as `backend71::plan_draw` gives them.
    pub plan: DrawPlan71,
}

fn tmus(
    bindings: &[Binding71<'_>],
    arena: &mut Arena71,
    records: &mut Records71,
) -> Result<Vec<ffi71::V3dJob71Tmu>, Refusal> {
    let mut out: Vec<ffi71::V3dJob71Tmu> = Vec::with_capacity(bindings.len());
    for b in bindings {
        if out.iter().any(|t| t.unit == b.unit) {
            return Err(own(REF_UNIT_TWICE));
        }
        let t = &b.filled.tex;
        let level = |v: u32| (v >> t.base_level).max(1);
        out.push(ffi71::V3dJob71Tmu {
            unit: b.unit,
            tex_state: records.tex(arena, &b.filled.state)?,
            sampler: records.sampler(arena, &b.filled.sampler)?,
            // the base level's size; a 2D texture and a cube map have depth
            // and array size 1 (texture71 composes no 3D or array texture)
            width: level(t.width),
            height: level(t.height),
            depth: 1,
            array_size: 1,
        });
    }
    Ok(out)
}

/// The sampler unit a texture uniform names (texuni71's rule: bits 24..31
/// of a TMU config uniform's data, the whole data of a size uniform).
fn unit_of(u: &ffi71::V3dUniform) -> u32 {
    match u.contents {
        texuni71::TMU_CONFIG_P0 | texuni71::TMU_CONFIG_P1 => u.data >> 24,
        _ => u.data,
    }
}

fn is_texture_word(contents: u32) -> bool {
    matches!(
        contents,
        texuni71::TMU_CONFIG_P0
            | texuni71::TMU_CONFIG_P1
            | texuni71::TEXTURE_WIDTH
            | texuni71::TEXTURE_HEIGHT
            | texuni71::TEXTURE_DEPTH
            | texuni71::TEXTURE_ARRAY_SIZE
    )
}

/// A stage's uniform list with every texture word replaced by the constant
/// `texuni71::word` says it is: the list `uniform71` fills to the stream the
/// builder must write.
pub fn resolve(list: &[ffi71::V3dUniform], tmu: &[ffi71::V3dJob71Tmu]) -> Result<Vec<ffi71::V3dUniform>, Refusal> {
    let units: Vec<texuni71::Unit> = tmu
        .iter()
        .map(|t| texuni71::Unit {
            unit: t.unit,
            tex_state: t.tex_state,
            sampler: t.sampler,
            width: t.width,
            height: t.height,
            depth: t.depth,
            array_size: t.array_size,
        })
        .collect();
    list.iter()
        .map(|u| {
            if !is_texture_word(u.contents) {
                return Ok(*u);
            }
            let data = texuni71::word(u.contents, u.data, &units).map_err(|e| refuse("texuni71", e))?;
            Ok(ffi71::V3dUniform { contents: uniform71::CONSTANT, data })
        })
        .collect()
}

/// Steps 1 to 3 for one draw. `dummy_addr` is the indirect buffer's first
/// byte, as for `backend71::plan_draw`.
pub fn plan_draw(
    table: &Table71,
    d: &Draw71<'_>,
    tex: &DrawTex71<'_>,
    dummy_addr: u32,
    arena: &mut Arena71,
    records: &mut Records71,
) -> Result<TexDrawPlan71, Refusal> {
    // a binding nothing reads is a caller's mistake, most likely fill71's
    // masked lists handed on without the program's own: the stream would
    // hold a constant 0 where a TMU word belongs
    let reads = |lists: [&[ffi71::V3dUniform]; 2], unit: u32| {
        lists.iter().any(|l| l.iter().any(|u| is_texture_word(u.contents) && unit_of(u) == unit))
    };
    if tex.fs.iter().any(|b| !reads([d.fs_uniforms, &[]], b.unit))
        || tex.vs.iter().any(|b| !reads([d.vs_uniforms, d.cs_uniforms], b.unit))
    {
        return Err(own(REF_UNREAD));
    }
    let fs_tmu = tmus(tex.fs, arena, records)?;
    let vs_tmu = tmus(tex.vs, arena, records)?;
    let fs = resolve(d.fs_uniforms, &fs_tmu)?;
    let vs = resolve(d.vs_uniforms, &vs_tmu)?;
    let cs = resolve(d.cs_uniforms, &vs_tmu)?;
    let resolved = Draw71 { fs_uniforms: &fs, vs_uniforms: &vs, cs_uniforms: &cs, ..*d };
    let plan = backend71::plan_draw(table, &resolved, dummy_addr)?;
    Ok(TexDrawPlan71 { fs_tmu, vs_tmu, plan })
}

// ---- the builder -------------------------------------------------------------

/// One draw as the builder takes it: the draw with its own uniform lists
/// (the texture words as the program has them) and the stages' bindings.
#[derive(Clone, Copy, Debug)]
pub struct BuildDraw71<'a> {
    pub draw: Draw71<'a>,
    pub fs_tmu: &'a [ffi71::V3dJob71Tmu],
    pub vs_tmu: &'a [ffi71::V3dJob71Tmu],
}

/// Why the builder did not record the job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexBuildError71 {
    /// The job builder's own (`v3d_job71_begin`, `_draw`, `_end`).
    Job(backend71::BuildError71),
    /// `v3d_tex71_pack_state` refused texture record `index`.
    TexState { index: usize },
    /// `v3d_tex71_pack_sampler` refused sampler record `index`.
    Sampler { index: usize },
    /// A record's address is outside the arena the builder has.
    Outside,
}

impl core::fmt::Display for TexBuildError71 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            TexBuildError71::Job(e) => write!(f, "{e}"),
            TexBuildError71::TexState { index } => {
                write!(f, "v3d_tex71_pack_state refused texture record {index}")
            }
            TexBuildError71::Sampler { index } => {
                write!(f, "v3d_tex71_pack_sampler refused sampler record {index}")
            }
            TexBuildError71::Outside => write!(f, "{REF_RECORD_OUTSIDE}"),
        }
    }
}

/// The C builders over memory the implementation owns: `v3d_tex71_pack_state`
/// and `v3d_tex71_pack_sampler` for the records, then the job builder's calls
/// with each draw's `tmu` arrays. [`CTexBuilder71`] is the in-process FFI; a
/// host test runs the same C files in a child process.
pub trait TexBuilder71 {
    /// The GPU address of a list's first byte.
    fn gpu(&self, l: List71) -> u32;
    /// The record arena: GPU address and size.
    fn arena(&self) -> (u32, u32);
    fn build(
        &mut self,
        target: &ffi71::V3dJob71Target,
        draws: &[BuildDraw71<'_>],
        clear: &ffi71::V3dJob71Clear,
        records: &Records71,
    ) -> Result<Built71, TexBuildError71>;
    /// The bytes the last build wrote into a list.
    fn bytes(&self, l: List71) -> &[u8];
    /// The arena's bytes after the last build, from its first byte.
    fn arena_bytes(&self) -> &[u8];
}

fn ix(l: List71) -> usize {
    match l {
        List71::Bcl => 0,
        List71::Rcl => 1,
        List71::Ind => 2,
    }
}

/// The in-process C builders over caller-owned CPU mappings of GPU memory
/// (the three lists and the arena). Not exercised by the host test (which
/// cannot link C into a test binary without a build script); the table's
/// hook links it.
pub struct CTexBuilder71<'m> {
    mem: [&'m mut [u8]; 3],
    gpu: [u32; 3],
    used: [u32; 3],
    arena: &'m mut [u8],
    arena_gpu: u32,
    arena_used: usize,
    page_size: u32,
}

impl<'m> CTexBuilder71<'m> {
    /// `bcl`, `rcl`, `ind`, `arena`: CPU views and their GPU addresses.
    /// `page_size`: the tile allocation granule (`backend71::PAGE_SIZE`).
    pub fn new(
        bcl: (&'m mut [u8], u32),
        rcl: (&'m mut [u8], u32),
        ind: (&'m mut [u8], u32),
        arena: (&'m mut [u8], u32),
        page_size: u32,
    ) -> CTexBuilder71<'m> {
        CTexBuilder71 {
            gpu: [bcl.1, rcl.1, ind.1],
            mem: [bcl.0, rcl.0, ind.0],
            used: [0; 3],
            arena: arena.0,
            arena_gpu: arena.1,
            arena_used: 0,
            page_size,
        }
    }

    fn record(&mut self, addr: u32, len: usize) -> Result<&mut [u8], TexBuildError71> {
        let off = addr.checked_sub(self.arena_gpu).ok_or(TexBuildError71::Outside)? as usize;
        let end = off.checked_add(len).ok_or(TexBuildError71::Outside)?;
        self.arena_used = self.arena_used.max(end);
        self.arena.get_mut(off..end).ok_or(TexBuildError71::Outside)
    }
}

impl TexBuilder71 for CTexBuilder71<'_> {
    fn gpu(&self, l: List71) -> u32 {
        self.gpu[ix(l)]
    }

    fn arena(&self) -> (u32, u32) {
        (self.arena_gpu, self.arena.len() as u32)
    }

    fn build(
        &mut self,
        target: &ffi71::V3dJob71Target,
        draws: &[BuildDraw71<'_>],
        clear: &ffi71::V3dJob71Clear,
        records: &Records71,
    ) -> Result<Built71, TexBuildError71> {
        self.arena_used = 0;
        for (index, (addr, s)) in records.tex.iter().enumerate() {
            let out = self.record(*addr, ffi71::V3D_TEX71_STATE_BYTES)?;
            // SAFETY: `out` is the record's 24 bytes; `s` a live struct.
            if !unsafe { ffi71::v3d_tex71_pack_state(out.as_mut_ptr(), s) } {
                return Err(TexBuildError71::TexState { index });
            }
        }
        for (index, (addr, s)) in records.samplers.iter().enumerate() {
            let out = self.record(*addr, ffi71::V3D_TEX71_SAMPLER_BYTES)?;
            // SAFETY: as above.
            if !unsafe { ffi71::v3d_tex71_pack_sampler(out.as_mut_ptr(), s) } {
                return Err(TexBuildError71::Sampler { index });
            }
        }

        // backend71::CBuilder71::build's calls, with the bindings
        let job = |e| TexBuildError71::Job(e);
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
            return Err(job(backend71::BuildError71::Begin));
        }
        let ptr = |t: &[ffi71::V3dJob71Tmu]| if t.is_empty() { core::ptr::null() } else { t.as_ptr() };
        let mut layouts = Vec::with_capacity(draws.len());
        for (index, bd) in draws.iter().enumerate() {
            let d = &bd.draw;
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
                tmu: ptr(bd.fs_tmu),
                num_tmu: bd.fs_tmu.len() as u32,
                vs_tmu: ptr(bd.vs_tmu),
                num_vs_tmu: bd.vs_tmu.len() as u32,
                vs_writes_psiz: false,
                instances: d.instances,
            };
            let mut lo = ffi71::V3dJob71DrawLayout::ZERO;
            // SAFETY: as above; `draw` and what it points to live past the call.
            let code = unsafe { ffi71::v3d_job71_draw(&mut j, &draw, &mut lo) };
            if code != ffi71::V3D_JOB71_OK {
                return Err(job(backend71::BuildError71::Draw { index, code }));
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
            return Err(job(backend71::BuildError71::End));
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

    fn arena_bytes(&self) -> &[u8] {
        &self.arena[..self.arena_used]
    }
}

// ---- the job -----------------------------------------------------------------

/// An assembled textured job: the admission, each draw's plan, the records
/// and the builder's result.
#[derive(Clone, Debug, PartialEq)]
pub struct AssembledTexJob71 {
    pub admitted: admit71::FrameAdmitted,
    pub draws: Vec<TexDrawPlan71>,
    pub records: Records71,
    pub built: Built71,
}

/// The bytes of a record the builder wrote at `addr` in the arena.
pub fn record_at(builder: &dyn TexBuilder71, addr: u32, len: u32) -> Option<&[u8]> {
    let off = addr.checked_sub(builder.arena().0)? as usize;
    builder.arena_bytes().get(off..off + len as usize)
}

/// One of fill71's jobs through `builder`, as `backend71::assemble_filled`,
/// with the sampler bindings of each draw (`tex[i]` is `job.draws[i]`'s; a
/// clear quad's is [`DrawTex71::default`]). Steps 1 to 5 of the module docs;
/// the first refusal ends it (nothing may then be submitted).
pub fn assemble_filled(
    table: &Table71,
    fb: &fill71::Framebuffer<'_>,
    job: &fill71::FilledJob,
    tex: &[DrawTex71<'_>],
    builder: &mut dyn TexBuilder71,
) -> Result<AssembledTexJob71, Refusal> {
    let admitted = backend71::admit_framebuffer(fb)?;
    let t = &job.target;
    if t.width != fb.width || t.height != fb.height || t.rts as usize != fb.colors.len() {
        return Err(refuse("backend71", backend71::FILLED_REFUSALS[2]));
    }
    if tex.len() != job.draws.len() {
        return Err(own(REF_BINDINGS));
    }
    let mut draws = job.draws.iter().map(backend71::draw_of).collect::<Result<Vec<_>, _>>()?;
    for (d, x) in draws.iter_mut().zip(tex) {
        if let Some(l) = &x.lists {
            d.fs_uniforms = unmasked(d.fs_uniforms, l.fs)?;
            d.vs_uniforms = unmasked(d.vs_uniforms, l.vs)?;
            d.cs_uniforms = unmasked(d.cs_uniforms, l.cs)?;
        }
    }

    let (arena_gpu, arena_size) = builder.arena();
    let mut arena = Arena71::new(arena_gpu, arena_size)?;
    let mut records = Records71::default();
    let ind_gpu = builder.gpu(List71::Ind);
    let plans = draws
        .iter()
        .zip(tex)
        .map(|(d, x)| plan_draw(table, d, x, ind_gpu, &mut arena, &mut records))
        .collect::<Result<Vec<_>, _>>()?;

    let build: Vec<BuildDraw71<'_>> =
        draws.iter().zip(&plans).map(|(d, p)| BuildDraw71 { draw: *d, fs_tmu: &p.fs_tmu, vs_tmu: &p.vs_tmu }).collect();
    let built = builder.build(t, &build, &job.clear, &records).map_err(|e| match e {
        TexBuildError71::Job(_) => refuse("v3d_job71", e),
        TexBuildError71::Outside => own(e),
        _ => refuse("v3d_tex71", e),
    })?;

    let got = (built.tiling.tile_w, built.tiling.tile_h);
    if got != admitted.tile {
        return Err(own(format_args!(
            "v3d_job71 chose a {}x{} tile, limits71 {}x{}",
            got.0, got.1, admitted.tile.0, admitted.tile.1
        )));
    }
    if built.layouts.len() != draws.len() {
        return Err(own("v3d_job71 recorded another number of draws"));
    }
    let ind = builder.bytes(List71::Ind);
    for (i, (p, lo)) in plans.iter().zip(&built.layouts).enumerate() {
        backend71::check_draw(table, i, &p.plan, lo, ind, ind_gpu)?;
    }
    let inside = records.tex.iter().all(|(a, _)| record_at(builder, *a, TEX_RECORD_BYTES).is_some())
        && records.samplers.iter().all(|(a, _)| record_at(builder, *a, SAMPLER_RECORD_BYTES).is_some());
    if !inside {
        return Err(own(REF_RECORD_OUTSIDE));
    }
    Ok(AssembledTexJob71 { admitted, draws: plans, records, built })
}
