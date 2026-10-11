// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 1.00 text -> [`ShcShader`]: the in-process compiler's pipeline.
//!
//! ```text
//!   text  --(es100, M2)-->  ir::Module  --(this file, M3)-->  instructions
//!                                       --(isa, M1)-------->  USC dwords
//!                                       --(abi.rs, M3)----->  ShcShader
//! ```
//!
//! **The back half is wired; the front end is the gap.** On a `gpu-vivante`
//! build [`assemble_from_ir`] runs the whole chain — selection, the register
//! allocator, the encoder, the `ShcShader` assembly — and produces a real
//! shader from a real `ir::Module`. [`compile`] cannot reach it yet only
//! because nothing turns GLSL text into that `Module`, so it still reports
//! [`CompileError::NotImplemented`] naming the front end.
//!
//! That split is deliberate: `assemble_from_ir` is testable today from a
//! hand-written IR, which is how the back half is known to work before
//! es100-frontend's first delivery rather than after it.

use super::arena::{Arena, ArenaError};
#[cfg(feature = "gpu-vivante")]
use crate::shaderc::online::select::SelInstr;
use crate::shaderc::ShcShader;
#[cfg(feature = "gpu-vivante")]
use {super::abi, super::ir, super::lower, super::reflect, super::select, super::sema, super::vivante::alloc};

/// Why a compile did not produce a shader.
///
/// Each variant carries what the log needs. An exhausted arena is its own
/// variant and not a rejection: a compiler that ran out of room must not look
/// like a compiler that refused the shader (see `arena.rs`).
/// The offending token, copied out of the front end's arena.
///
/// `Error::tok` borrows the source text and the compile's scratch arena, and
/// both are gone by the time an application reads `glGetShaderInfoLog`, so the
/// token cannot be carried out by reference. It is copied instead.
///
/// Carrying it at all is the point: with the token dropped, the front end's
/// messages arrive as "assignment to this ___ is not lowered yet" with a hole
/// where the construct belongs -- and one dEQP run produced that same string
/// **1,841 times byte-identical**, which is a log nobody can act on and a
/// bucket nobody can split.
///
/// A name longer than the buffer is truncated with a trailing `...` rather
/// than dropped: a truncated name still identifies the construct, an absent
/// one does not.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Token {
    buf: [u8; Token::CAP],
    len: u8,
}

impl Token {
    /// Long enough for every identifier in either shader table and for the
    /// longest `kind_name`; the truncation path exists for what a dEQP
    /// negative test can invent, not for real shaders.
    const CAP: usize = 40;

    pub const EMPTY: Token = Token { buf: [0; Token::CAP], len: 0 };

    pub fn new(s: &str) -> Token {
        let mut t = Token::EMPTY;
        /* Cut on a character boundary: a token may hold a non-ASCII byte the
         * lexer refused, and half a character in a log is worse than a short
         * one. */
        let mut end = s.len().min(Token::CAP);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        if end < s.len() && end >= 3 {
            /* Room for the marker, on a boundary again. */
            let mut cut = end - 3;
            while cut > 0 && !s.is_char_boundary(cut) {
                cut -= 1;
            }
            t.buf[..cut].copy_from_slice(&s.as_bytes()[..cut]);
            t.buf[cut..cut + 3].copy_from_slice(b"...");
            t.len = (cut + 3) as u8;
            return t;
        }
        t.buf[..end].copy_from_slice(&s.as_bytes()[..end]);
        t.len = end as u8;
        t
    }

    pub fn as_str(&self) -> &str {
        /* Built only by `new`, which copies whole characters. */
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl core::fmt::Debug for Token {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompileError {
    /// The part of the pipeline that does not exist yet.
    NotImplemented(Stage),
    /// An arena ran out, with the bytes it still had. The arena reports no
    /// "wanted" size ([`ArenaError`] carries none), so the log says what is
    /// left and which constant to raise rather than inventing a number.
    OutOfMemory { which: Which, left: usize },
    /// A compile was started inside a compile; see
    /// [`store::with_scratch`](super::store::with_scratch).
    Reentered,
    /// The front end rejected the source.
    ///
    /// It carries the diagnostic rather than pointing at a log the caller may
    /// not have: the first run of the oracle harness reported 302 sources
    /// "not compiled" with an empty reason, because this variant had no
    /// payload and the front end's `Error` was being discarded. A harness that
    /// cannot say *why* is not measuring anything.
    ///
    /// `line` is 1-based in the text the application passed to
    /// `glShaderSource`, and `msg` is the front end's own wording, so the info
    /// log reads as it did before.
    Rejected { line: u32, tok: Token, msg: &'static str },
    /// The stage number was neither vertex nor fragment.
    BadStage(u32),
    /// A stage of the back half refused, keeping which and why.
    BackEnd(BackEndError),
}

/// Which part of the pipeline a [`CompileError::NotImplemented`] stopped at,
/// so that the log names the milestone that owns it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// GLSL ES 1.00 front end and reflection (M2, es100-frontend).
    FrontEnd,
    /// IR -> USC instructions (M3, this file).
    CodeGen,
    /// Instructions -> dwords (M1).
    Encode,
    /// Instructions + reflection -> `ShcShader` (M3, `abi.rs`).
    Abi,
}

impl Stage {
    /// What is missing, and which milestone owns it, for the compile log.
    pub fn describe(self) -> &'static str {
        match self {
            Stage::FrontEnd => "the GLSL ES 1.00 front end (M2)",
            Stage::CodeGen => "the IR-to-USC code generator (M3)",
            Stage::Encode => "the USC instruction encoder (M1)",
            Stage::Abi => "the ShcShader assembly (M3)",
        }
    }
}

/// Which arena ran out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Which {
    /// The per-compile arena: raise `store::SCRATCH_BYTES`.
    Scratch,
    /// The arena compiled shaders keep: raise `store::KEEP_BYTES`.
    Keep,
}

impl Which {
    /// The arena's name, for the compile log.
    pub fn name(self) -> &'static str {
        match self {
            Which::Scratch => "scratch",
            Which::Keep => "keep",
        }
    }

    /// The constant to raise, for the compile log.
    pub fn knob(self) -> &'static str {
        match self {
            Which::Scratch => "shaderc::online::store::SCRATCH_BYTES",
            Which::Keep => "shaderc::online::store::KEEP_BYTES, in an image CONFIG_YAKOGL_SHADERC_KEEP_BYTES",
        }
    }
}

/// An arena refusal inside the compiler is the scratch arena's: the `keep`
/// arena is only ever asked by `backend.rs`, which builds the error itself
/// because only the caller knows which arena it was asking.
/// A stage of the back half failed. Each keeps the original error rather than
/// flattening it: "the allocator refused source slot 0" and "the ABI refused a
/// varying at register 0" send a reader to different files, and a single
/// `BackEnd` variant would send them to neither.
#[cfg(feature = "gpu-vivante")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackEndError {
    Select(select::SelectError),
    Alloc(alloc::AllocError),
    Encode(super::vivante::decode::DecodeError),
    Abi(abi::AbiError),
    /* the link-time attribute layout (the shared `powervr::abi` helpers:
     * with_attrib_map, deferred_vertex's declared list) */
    Attrib(super::powervr::abi::UscAbiError),
}

/// The PowerVR back half's refusals: where the USC chain stopped, which
/// already names the stage (`chain::ChainError`).
#[cfg(feature = "gpu-powervr")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackEndError {
    Usc(super::powervr::chain::ChainError),
}

/// The V3D back half's refusals: the program chain (`v3d::chain`, which
/// names its stage) or the record assembly (`v3d_abi`).
#[cfg(feature = "gpu-v3d")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackEndError {
    Program(super::v3d::chain::V3dError),
    Abi(super::v3d_abi::V3dAbiError),
    /* the link-time attribute layout (the shared `powervr::abi` helpers:
     * with_attrib_map, deferred_vertex's declared list), as on the GC7000 */
    Attrib(super::powervr::abi::UscAbiError),
}

#[cfg(feature = "gpu-v3d")]
impl From<super::v3d_abi::V3dAbiError> for CompileError {
    fn from(e: super::v3d_abi::V3dAbiError) -> CompileError {
        use super::v3d_abi::V3dAbiError as E;
        match e {
            /* Capacity, reported as capacity (as the other GPUs do), not as
             * a code generator defect. */
            E::OutOfScratch => CompileError::OutOfMemory { which: Which::Scratch, left: 0 },
            E::OutOfKeep => CompileError::OutOfMemory { which: Which::Keep, left: super::store::keep().left() },
            e => CompileError::BackEnd(BackEndError::Abi(e)),
        }
    }
}

#[cfg(feature = "gpu-v3d")]
impl From<super::v3d::chain::V3dError> for CompileError {
    fn from(e: super::v3d::chain::V3dError) -> CompileError {
        match e {
            super::v3d::chain::V3dError::OutOfScratch => CompileError::OutOfMemory { which: Which::Scratch, left: 0 },
            e => CompileError::BackEnd(BackEndError::Program(e)),
        }
    }
}

/// The V3D's VPM in bytes, which the program is compiled for (it sizes
/// `vcm_cache_size`). The Raspberry Pi 4 reports 65536 (CTL_IDENT1
/// 0x81001422 read under Linux, 2026-09-24,
/// driver/docs/rpi4-v3d.md). The backend's init compares the
/// identified value with this and refuses to start on another.
#[cfg(feature = "gpu-v3d")]
pub const V3D_VPM_BYTES: u32 = 65536;

/// GLSL text to `Lowered`, the same front end every GPU runs: the text is
/// copied into the arena, lowered, the zero list applied, and the front
/// end's garbage given back (`lower::relocate`).
#[cfg(feature = "gpu-v3d")]
fn lower_source<'a>(
    stage: u32,
    glsl: &[u8],
    zero: &[&str],
    scratch: &Arena<'a>,
) -> Result<super::lower::Lowered<'a>, CompileError> {
    let st = match stage {
        crate::shaderc::SHC_STAGE_VERTEX => super::sema::Stage::Vertex,
        crate::shaderc::SHC_STAGE_FRAGMENT => super::sema::Stage::Fragment,
        other => return Err(CompileError::BadStage(other)),
    };
    let src = core::str::from_utf8(glsl).map_err(|_| CompileError::Rejected {
        line: 0,
        tok: Token::EMPTY,
        msg: "shader source is not valid UTF-8",
    })?;
    let src = scratch.alloc_str(src)?;
    let mark = scratch.used();
    super::lower::compile_to_ir(scratch, st, src)
        .and_then(|l| super::lower::zero_varyings(scratch, l, zero))
        .and_then(|l| super::lower::relocate(scratch, l, src, mark))
        .map_err(|e| CompileError::Rejected { line: e.line, tok: Token::new(e.tok), msg: e.msg })
}

#[cfg(feature = "gpu-vivante")]
impl From<select::SelectError> for CompileError {
    fn from(e: select::SelectError) -> CompileError {
        match e {
            /* The scratch ran out: a capacity refusal, reported as one (as
             * the PowerVR chain does), not as a code generator defect. */
            select::SelectError::OutOfScratch => CompileError::OutOfMemory { which: Which::Scratch, left: 0 },
            e => CompileError::BackEnd(BackEndError::Select(e)),
        }
    }
}

#[cfg(feature = "gpu-vivante")]
impl From<alloc::AllocError> for CompileError {
    fn from(e: alloc::AllocError) -> CompileError {
        match e {
            /* As for selection: out of scratch is capacity. On the full
             * GLSL ES 3.00 stack uniform_api.random.23 gets through the
             * front end on the GC7000 and runs out here. */
            alloc::AllocError::OutOfScratch => CompileError::OutOfMemory { which: Which::Scratch, left: 0 },
            e => CompileError::BackEnd(BackEndError::Alloc(e)),
        }
    }
}

#[cfg(feature = "gpu-vivante")]
impl From<super::vivante::decode::DecodeError> for CompileError {
    fn from(e: super::vivante::decode::DecodeError) -> CompileError {
        CompileError::BackEnd(BackEndError::Encode(e))
    }
}

#[cfg(feature = "gpu-vivante")]
impl From<abi::AbiError> for CompileError {
    fn from(e: abi::AbiError) -> CompileError {
        CompileError::BackEnd(BackEndError::Abi(e))
    }
}

impl From<ArenaError> for CompileError {
    fn from(_: ArenaError) -> CompileError {
        CompileError::OutOfMemory { which: Which::Scratch, left: 0 }
    }
}

/// Compile one stage of GLSL ES 1.00 into a [`ShcShader`].
///
/// `scratch` dies with this call; everything the returned shader points at is
/// allocated from `keep`. `blend_key` is 0 for the shader as written, or the
/// blend state to lower into the fragment epilogue — blending and colour masks
/// are compiled into the FS on this GPU (`blending_recipe.md`).
/// The back half of the compiler: an `ir::Module` to a `ShcShader`.
///
/// Selection, register allocation, encoding and the ABI assembly, in that
/// order, with every intermediate living in `scratch` and only the finished
/// shader's bytes in `keep`.
///
/// `temps` comes from the allocator and is copied, never recomputed here:
/// inputs and fixed outputs are reservations inside the same register file as
/// the temporaries, so the count has to include them, and one place computing
/// it is the point.
#[cfg(feature = "gpu-vivante")]
pub fn assemble_from_ir(
    m: &ir::Module<'_>,
    scratch: &Arena<'_>,
    uniforms: &[crate::shaderc::ShcUniform],
) -> Result<ShcShader, CompileError> {
    assemble_from_ir_keyed(m, scratch, uniforms, false)
}

/// [`assemble_from_ir`] for a fragment shader's `SHC_VIV_KEY_FRONT_FLIPPED`
/// variant when `front_flipped` (`select::select_keyed`).
#[cfg(feature = "gpu-vivante")]
pub fn assemble_from_ir_keyed(
    m: &ir::Module<'_>,
    scratch: &Arena<'_>,
    uniforms: &[crate::shaderc::ShcUniform],
    front_flipped: bool,
) -> Result<ShcShader, CompileError> {
    let sel = select::select_keyed(m, scratch, front_flipped)?;

    /* The stage, because the fragment file's r0 is gl_FragCoord whether the
     * shader names it or not; the allocator cannot see that from `sel`. */
    let stage =
        if m.stage == crate::shaderc::SHC_STAGE_FRAGMENT { alloc::Stage::Fragment } else { alloc::Stage::Vertex };
    /* Selection emits a value where its source expression sits, which for a
     * shader that computes several alternatives and then chooses among them
     * leaves every alternative holding a register until the choice. Sinking
     * each definition to just before its first use is what decides the
     * register count -- see `alloc::sink_definitions`, which measured 64
     * values live against 18 on the dEQP shader that wedged the GPU.
     *
     * A mutable copy, because the pass reorders in place and `Selected` hands
     * out a shared slice. The copy lives in the per-compile scratch and costs
     * one `SelInstr` per instruction. */
    /* Where the allocator put each output selection wrote (see below). */
    let mut placed: alloc::Placed = [alloc::NOT_PLACED; alloc::MAX_TEMPS as usize];
    /* The loop branches by index, in a copy the passes below may renumber. */
    let back_edges = scratch.alloc_filled(sel.back_edges.len(), 0usize)?;
    back_edges.copy_from_slice(sel.back_edges);
    let ordered: &mut [SelInstr] = if let Some(first) = sel.instrs.first() {
        /* No `defer_io_hazards` here: an output some instruction writes is
         * placed by the allocator below, which sees every input's live range
         * and cannot put an output over an input still to be read. An output
         * nobody writes is left in place and writes nothing, so it cannot
         * collide either. (The pass stays for callers of the fixed layout.) */
        let buf = scratch.alloc_filled(sel.instrs.len() + alloc::MAX_DEFERRED_OUTPUTS, *first)?;
        buf[..sel.instrs.len()].copy_from_slice(sel.instrs);
        let len = sel.instrs.len();
        /* Then drop what nothing consumes: the slot writes and if-join selects
         * selection keeps because a slot is live to the end of the IR. It
         * remaps branch targets and the back-edge list, which `abi.rs` checks
         * backward jumps against by index. */
        let dead = alloc::eliminate_dead(scratch, &mut buf[..len], back_edges)?;
        let buf = &mut buf[..len - dead];
        alloc::sink_definitions(scratch, buf)?;
        /* `alloc::split_ranges` is NOT called here, and the reason is on the
         * function: its bound says a quarter of the straight-line registers
         * and the pipeline measured +5 temporaries when it ran. The bound and
         * the outcome disagree and the disagreement is not explained, so the
         * pass stays out of the product until it is. */
        /* Then delete the copies whose two values never meet. Sinking first
         * on purpose: it shortens live ranges, so a copy that straddled two
         * live values before may not afterwards, and coalescing sees the
         * shorter ranges. The pass returns how many instructions it removed
         * and compacts in place. */
        let removed = alloc::coalesce_copies(scratch, buf)?;
        let keep = buf.len() - removed;
        &mut buf[..keep]
    } else {
        &mut []
    };

    /* The program, encoded as the allocator builds it: four dwords per
     * instruction, never a 160-byte `Instr` array to encode afterwards (that
     * array was 475,200 of uniform_api.random.23's 2,251,896-byte peak). One
     * instruction's room at least, for the one-`nop` program below. */
    let words = scratch.alloc_filled(ordered.len().max(1) * 4, 0u32)?;

    /* The registers the ABI pins, which no instruction need mention. The set
     * is `abi.rs`'s -- the one it checks `temps` against -- because a
     * register it will refuse a count for is exactly a register that must be
     * reserved here. */
    let out = {
        /* Every output an instruction writes is placed like a value; the
         * rest stay reserved where selection put them. */
        let writes =
            |r: u8| ordered.iter().any(|s| matches!(s.dst, Some(alloc::SelDst::Fixed { reg, .. }) if reg == r));
        let mut placeable = [0u8; alloc::MAX_TEMPS as usize];
        let mut n_placeable = 0usize;
        let mut fixed = [0u8; alloc::MAX_TEMPS as usize];
        let mut n_fixed = 0usize;
        for r in abi_outputs(&sel) {
            if writes(r) {
                placeable[n_placeable] = r;
                n_placeable += 1;
            } else {
                fixed[n_fixed] = r;
                n_fixed += 1;
            }
        }
        alloc::allocate_and_encode_placed(
            scratch,
            ordered,
            words,
            stage,
            fixed[..n_fixed].iter().copied(),
            sel.io_in.iter().map(|io| io.reg).chain(sel.vs_id_in_reg),
            &placeable[..n_placeable],
            &mut placed,
        )?
    };
    /* Copies of a register onto itself, which placing outputs and sharing
     * registers leave behind, are deleted; branch targets and the back-edge
     * list follow the survivors. */
    let identity = alloc::drop_identity_movs_encoded(scratch, words, out.count, back_edges)?;
    let kept = out.count - identity;

    /* Four dwords per 128-bit instruction. */
    let n = if kept == 0 {
        /* A shader with nothing to do is still a program, and its program is
         * one `nop` -- four zero dwords, which is exactly how a `nop` with a
         * void destination and no sources encodes. It happens when every
         * output already sits in the register its source occupies (`gl_Position
         * = a_pos`), so `select` emits no instruction; the reference program is
         * the same four zeros (chain_tests
         * `the_passthrough_vertex_shader_becomes_a_real_shcshader`).
         *
         * Written here rather than as a `nop` through the allocator because
         * `SelInstr` cannot say "this operation has a DST field and it is
         * void": `alloc` requires a destination for any operation whose
         * encoding has the field, and `nop`'s is present-but-void. Bending
         * that model to emit four zeros would be the tail wagging the dog.
         * The four dwords are zeroed here: the identity pass may have
         * compacted every instruction away and left the first one's words. */
        words[..4].fill(0);
        4
    } else {
        kept * 4
    };

    /* The allocator's number, with nothing added. It covers the ABI's
     * registers because they were reserved above; the `.max()` that used to
     * stand here was a workaround for the allocator not being told about
     * them. `abi.rs` refuses a count that does not clear them, so if this
     * ever stops being true it is loud.
     *
     * MEASURED before the workaround came out, over the GC7000 corpus: see
     * the commit message. */
    let temps = out.temps;

    /* The io rows name where each output ENDED UP, not where selection
     * wrote it: the allocator placed every output an instruction writes. */
    let place = |r: u8| placed.get(r as usize).copied().filter(|p| *p != alloc::NOT_PLACED).unwrap_or(r);
    let io_out: &[abi::IoReg<'_>] = match sel.io_out.first() {
        None => sel.io_out,
        Some(first) => {
            let rows = scratch.alloc_filled(sel.io_out.len(), *first)?;
            for (d, s) in rows.iter_mut().zip(sel.io_out.iter()) {
                *d = abi::IoReg { reg: place(s.reg), ..*s };
            }
            rows
        }
    };
    let p = abi::VivProgram {
        stage: m.stage,
        code: &words[..n],
        temps,
        io_in: sel.io_in,
        io_out,
        consts: sel.consts,
        back_edges,
        vs_pos_out_reg: sel.vs_pos_out_reg.map(place),
        vs_pointsize_out_reg: sel.vs_pointsize_out_reg.map(place),
        vs_id_in_reg: sel.vs_id_in_reg,
        ps_color_out_reg: sel.ps_color_out_reg.map(place),
        ps_depth_out_reg: None,
        /* Copied, never chosen: gc7000-abi.md lists both as not understood,
         * and a compiler picking them freshly is an unmeasured change to a
         * word nobody can check. Zero is what the M3 shaders carry. */
    };
    Ok(abi::assemble(&p, uniforms, m.uniform_dwords)?)
}

/// Every OUTPUT register the ABI pins for this shader: these must hold their
/// value at the end, so they are reservations inside the file the allocator
/// allocates from -- and an instruction can mention none of them.
///
/// The inputs (`io_in`) are passed apart: each is a value in its register
/// from the start to its last read, and the register is free after that
/// (`alloc::allocate_and_build_io`). Together the two are exactly the set
/// `abi.rs` checks `temps` against ([`abi::AbiError::TempsBelowIo`]),
/// including `gl_PointSize`; the allocator's count covers both, and the two
/// must not drift.
#[cfg(feature = "gpu-vivante")]
fn abi_outputs<'s>(sel: &'s select::Selected<'_>) -> impl Iterator<Item = u8> + 's {
    sel.io_out
        .iter()
        .map(|io| io.reg)
        .chain(sel.vs_pos_out_reg)
        .chain(sel.vs_pointsize_out_reg)
        .chain(sel.ps_color_out_reg)
}

/// Compile one stage of GLSL ES 1.00 into a [`ShcShader`], in process.
///
/// Text through the front end into an `ir::Module`, then the back half. Every
/// intermediate lives in `scratch`; only the finished shader's bytes are in
/// `keep`.
#[cfg(feature = "gpu-vivante")]
pub fn compile(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    scratch: &Arena<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, CompileError> {
    compile_linked(stage, glsl, blend_key, &[], scratch, keep)
}

/// A compile's uniform-block table (`reflect::block_table`) and its uniform
/// rows (the vertex stage's and the fragment stage's; one of them empty for a
/// one-stage compile), in the compile's scratch; the online back end copies
/// what it needs into the keep record next to the shader
/// (`backend::keep_blocks`).
pub type Blocks<'s> = (
    &'s [super::reflect::BlockDesc<'s>],
    &'s [super::reflect::BlockMember<'s>],
    [&'s [super::reflect::UniformRow<'s>]; 2],
);

/// [`compile`], for a fragment shader linked to a vertex shader that declares
/// the varyings in `zero` but never writes them: they read as 0
/// ([`lower::zero_varyings`]). Empty for every other compile.
pub fn compile_linked(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    zero: &[&str],
    scratch: &Arena<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, CompileError> {
    compile_linked_blocks(stage, glsl, blend_key, zero, scratch, keep).map(|(sh, _)| sh)
}

/// [`compile_linked`], and the shader's uniform-block table.
#[cfg(feature = "gpu-vivante")]
pub fn compile_linked_blocks<'s>(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    zero: &[&str],
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    compile_vivante(stage, glsl, blend_key, zero, None, scratch, keep)
}

/// A vertex shader compiled with the attribute locations the link decided
/// (`ShcBackend::compile_vs_linked`, SHADERC_NOTES.md "Attribute registers
/// decided at link"), and its uniform-block table: attributes bound to one
/// location read that location's input register, which one vertex element
/// feeds (`with_attrib_map` moves their loads; `abi` lists the location once).
#[cfg(feature = "gpu-vivante")]
pub fn compile_vs_mapped_blocks<'s>(
    glsl: &[u8],
    map: &crate::shaderc::ShcAttribMap,
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    compile_vivante(crate::shaderc::SHC_STAGE_VERTEX, glsl, 0, &[], Some(map), scratch, keep)
}

/// The GC7000's compile of one stage, with the zero-varying list and, for a
/// vertex shader at link, the attribute locations.
#[cfg(feature = "gpu-vivante")]
fn compile_vivante<'s>(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    zero: &[&str],
    map: Option<&crate::shaderc::ShcAttribMap>,
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    let st = match stage {
        crate::shaderc::SHC_STAGE_VERTEX => sema::Stage::Vertex,
        crate::shaderc::SHC_STAGE_FRAGMENT => sema::Stage::Fragment,
        other => return Err(CompileError::BadStage(other)),
    };
    /* The pixel engine blends on this GPU, so the one variant a fragment
     * shader has is gl_FrontFacing's other sense (SHC_VIV_KEY_FRONT_FLIPPED,
     * asked for by a draw whose glFrontFace and target y flip disagree; see
     * `YakoglBackend::front_facing_shader_variant`). Any other non-zero key
     * is the caller asking for something this GPU does not have. */
    let front_flipped = match blend_key {
        0 => false,
        crate::shaderc::SHC_VIV_KEY_FRONT_FLIPPED if stage == crate::shaderc::SHC_STAGE_FRAGMENT => true,
        _ => return Err(CompileError::NotImplemented(Stage::CodeGen)),
    };

    /* The front end takes `&str`, and the GL core's copy is application bytes,
     * so a source that is not UTF-8 is refused as a shader rather than mangled
     * into one. The text is then COPIED into the arena: the front end's output
     * borrows its input for the arena's lifetime, and the caller's buffer is
     * not the arena's to borrow. A transmute would have made the lifetimes
     * agree without making the claim true. One copy of a shader's text is not
     * a cost worth an unsafe block in the compiler's entry path.
     */
    let src = core::str::from_utf8(glsl).map_err(|_| CompileError::Rejected {
        line: 0,
        tok: Token::EMPTY,
        msg: "shader source is not valid UTF-8",
    })?;
    let src = scratch.alloc_str(src)?;
    let mark = scratch.used();

    /* The front end's garbage is given back before selection runs: only
     * the module and the reflection survive (`lower::relocate`). */
    /* the c = 0.5 remap of a sampler-state variant (`lodkey`, asked for by
     * the online back end around the compile; 0 in every other compile) */
    let mag_switch = crate::shaderc::compile_lod_key().mag_switch;
    let lowered = lower::compile_to_ir(scratch, st, src)
        .and_then(|l| lower::zero_varyings(scratch, l, zero))
        .and_then(|l| super::lodkey::remap_mag_switch(scratch, l, mag_switch))
        .and_then(|l| lower::relocate(scratch, l, src, mark))
        .map_err(|e| CompileError::Rejected { line: e.line, tok: Token::new(e.tok), msg: e.msg })?;
    let uniforms = reflect::uniform_rows(&lowered.reflection, scratch)?;
    let blocks = (lowered.reflection.blocks, lowered.reflection.block_members, [lowered.reflection.uniforms, &[][..]]);
    let attrib = |e| CompileError::BackEnd(BackEndError::Attrib(e));
    if stage == crate::shaderc::SHC_STAGE_VERTEX {
        if let Some(map) = map {
            let (m, _) =
                super::powervr::abi::with_attrib_map(scratch, &lowered.module, lowered.reflection.attribs, map)
                    .map_err(attrib)?;
            return assemble_from_ir(m, scratch, uniforms).map(|sh| (sh, blocks));
        }
        /* More attribute locations declared than the front end has vertex
         * elements (16; a matrix takes one a column): which of them alias is
         * known only at link (glBindAttribLocation), so the layout waits for
         * it -- a record with no code and every declared attribute
         * (`layout_at_link`), compiled again with the program's locations.
         * Refused here before as Abi(TooManyIo { got: 32, max: 16 }) or, for
         * the shapes whose 32 inputs fitted the list, Abi(LoadBalancingTemps
         * Unmeasured) (dEQP attribute_location.bind_aliasing.max_*, 14). */
        let locations: u32 = lowered
            .reflection
            .attribs
            .iter()
            .map(|r| u32::from(super::powervr::abi::matrix_columns(r.gl_type).unwrap_or(1)))
            .sum();
        if locations > crate::shaderc::SHC_VIV_MAX_ATTRIB_LOCATIONS {
            return Ok((deferred_vertex(&lowered.module, lowered.reflection.attribs, keep, attrib)?, blocks));
        }
    }
    let _ = keep;
    let mut sh = assemble_from_ir_keyed(&lowered.module, scratch, uniforms, front_flipped)?;
    /* Says the draw has to pick between the two senses (the GL core asks for
     * the variant only for a shader that says so). */
    sh.reads_front_facing = u8::from(stage == crate::shaderc::SHC_STAGE_FRAGMENT && lowered.module.uses_front_facing);
    Ok((sh, blocks))
}

/// The V3D build: one stage compiles to its reflection and no code; the
/// code is the PROGRAM's, built at link ([`compile_program`]). The front end
/// runs here in full, so an invalid shader is refused at `glCompileShader`
/// as on the other GPUs.
#[cfg(feature = "gpu-v3d")]
pub fn compile(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    scratch: &Arena<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, CompileError> {
    compile_linked(stage, glsl, blend_key, &[], scratch, keep)
}

/// [`compile_linked`], and the shader's uniform-block table: the stage's
/// reflection, as on the other GPUs.
#[cfg(feature = "gpu-v3d")]
pub fn compile_linked_blocks<'s>(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    zero: &[&str],
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    /* The TLB blends on this GPU, so there is no blend variant to build; a
     * non-zero key is the caller asking for something this GPU has not got
     * (as on the GC7000). */
    if blend_key != 0 {
        return Err(CompileError::NotImplemented(Stage::CodeGen));
    }
    let l = lower_source(stage, glsl, zero, scratch)?;
    let blocks = (l.reflection.blocks, l.reflection.block_members, [l.reflection.uniforms, &[][..]]);
    /* More attribute locations declared than the V3D has attribute records
     * (16; a matrix takes one a column): which of them alias is known only
     * at link (glBindAttribLocation), so the layout waits for it -- a record
     * with no code and every declared attribute (`layout_at_link`), compiled
     * again with the program's locations ([`compile_vs_mapped_blocks`]).
     * Refused here before as Abi(AttribLocation { location: 16 }) (dEQP
     * attribute_location.bind_aliasing.max_cond_* / max_inactive_*, 14). */
    if stage == crate::shaderc::SHC_STAGE_VERTEX {
        let locations: u32 = l
            .reflection
            .attribs
            .iter()
            .map(|r| u32::from(super::powervr::abi::matrix_columns(r.gl_type).unwrap_or(1)))
            .sum();
        if locations > crate::shaderc::SHC_V3D_MAX_ATTRIB_LOCATIONS {
            let attrib = |e| CompileError::BackEnd(BackEndError::Attrib(e));
            return Ok((deferred_vertex(&l.module, l.reflection.attribs, keep, attrib)?, blocks));
        }
    }
    let sh = super::v3d_abi::stage_shader(stage, &l.module, &l.reflection, scratch, keep)?;
    Ok((sh, blocks))
}

/// A vertex shader's front end run with the attribute locations the link
/// decided (`ShcBackend::compile_vs_linked`, SHADERC_NOTES.md "Attribute
/// registers decided at link"): `with_attrib_map` moves each attribute's
/// loads and reflection row to its location, so two attributes bound to one
/// location load one attribute record. The unchanged front end without a map.
#[cfg(feature = "gpu-v3d")]
fn lower_vertex_mapped<'a>(
    glsl: &[u8],
    map: Option<&crate::shaderc::ShcAttribMap>,
    scratch: &'a Arena<'a>,
) -> Result<super::lower::Lowered<'a>, CompileError> {
    let l = lower_source(crate::shaderc::SHC_STAGE_VERTEX, glsl, &[], scratch)?;
    let Some(map) = map else {
        return Ok(l);
    };
    let super::lower::Lowered { module, reflection } = l;
    let module = scratch.alloc_copy(core::slice::from_ref(&module))?;
    let (m, rows) = super::powervr::abi::with_attrib_map(scratch, &module[0], reflection.attribs, map)
        .map_err(|e| CompileError::BackEnd(BackEndError::Attrib(e)))?;
    Ok(super::lower::Lowered { module: *m, reflection: super::reflect::Reflection { attribs: rows, ..reflection } })
}

/// A vertex shader compiled with the attribute locations the link decided
/// (`ShcBackend::compile_vs_linked`), and its uniform-block table. On the
/// V3D a stage is its reflection alone (the program's code is built at
/// link, [`link_sources`], which applies the same map); the record lists
/// each active location once.
#[cfg(feature = "gpu-v3d")]
pub fn compile_vs_mapped_blocks<'s>(
    glsl: &[u8],
    map: &crate::shaderc::ShcAttribMap,
    scratch: &'s Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    let l = lower_vertex_mapped(glsl, Some(map), scratch)?;
    let sh = super::v3d_abi::stage_shader(crate::shaderc::SHC_STAGE_VERTEX, &l.module, &l.reflection, scratch, keep)?;
    Ok((sh, (l.reflection.blocks, l.reflection.block_members, [l.reflection.uniforms, &[][..]])))
}

/// The primitive class a program's binary is built for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProgramPrim {
    Triangles,
    Lines,
    Points,
}

/// What selects one binary of a linked program, for every GPU: the
/// primitive class (the V3D's points and lines variants) and the blend key
/// (the PowerVR's blend variants; always 0 where the hardware blends).
/// docs/online-shaderc/v3d-backend.md, "The common back-end entry (planned)".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProgramKey {
    pub prim: ProgramPrim,
    pub blend_key: u32,
    /// The vertex stage's sampler-state key (the V3D's `cube_noseam`; the
    /// fragment stage's is the compile's own, `shaderc::compile_lod_key`).
    pub vs_lod: crate::shaderc::LodKey,
}

impl ProgramKey {
    /// The binary a link builds: triangles, no blend variant.
    pub const LINK: ProgramKey =
        ProgramKey { prim: ProgramPrim::Triangles, blend_key: 0, vs_lod: crate::shaderc::LodKey::NONE };
}

/// THE back-end entry of a linked program: the two stages' IR (each an
/// `ir::Module` with the reflection its ABI record carries), a key, the
/// arenas; out comes the program's code and its ABI record, as the
/// `ShcShader` that replaces the program's fragment shader. One shape meant
/// for every GPU (docs/online-shaderc/v3d-backend.md, "The common back-end
/// entry (planned)"); today only the V3D goes through it. A GPU that
/// compiles per stage would call its stage compiler twice in here.
///
/// The fragment IR comes with the zero-varying list already applied
/// (`lower::zero_varyings`, as on every GPU), so the key does not carry it.
#[cfg(feature = "gpu-v3d")]
pub fn compile_program(
    vs: &super::lower::Lowered<'_>,
    fs: &super::lower::Lowered<'_>,
    key: ProgramKey,
    scratch: &Arena<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, CompileError> {
    use super::v3d::chain::{compile_program_keyed as chain_program, Prim, Target};
    /* the TLB blends: no blend variant exists on this GPU */
    if key.blend_key != 0 {
        return Err(CompileError::NotImplemented(Stage::CodeGen));
    }
    /* the chain refuses Lines and Points by name (NotYet) until it builds them */
    let prim = match key.prim {
        ProgramPrim::Triangles => Prim::Triangles,
        ProgramPrim::Lines => Prim::Lines,
        ProgramPrim::Points => Prim::Points,
    };
    let fs_stage =
        super::v3d_abi::stage_shader(crate::shaderc::SHC_STAGE_FRAGMENT, &fs.module, &fs.reflection, scratch, keep)?;
    let prog = chain_program(
        &vs.module,
        &fs.module,
        prim,
        Target { vpm_size: V3D_VPM_BYTES },
        key.vs_lod,
        crate::shaderc::compile_lod_key(),
        scratch,
    )?;
    Ok(super::v3d_abi::program_shader(fs_stage, &prog, keep)?)
}

/// The V3D program of two sources at glLinkProgram: both through the front
/// end again (the records keep sources, not IR), the vertex shader's
/// attributes at the locations in `vs_map` when its record was compiled
/// with the link's layout ([`compile_vs_mapped_blocks`]), the fragment
/// shader's varyings in `zero` read as 0 (`lower::zero_varyings`, inside
/// `lower_source`), then [`compile_program`] with [`ProgramKey::LINK`] and
/// the vertex stage's sampler-state key `vs_key`.
///
/// The program record replaces the program's fragment shader, and the GL
/// core reads a program's uniform blocks from its two shaders
/// (`gl::program`, `shaderc::uniform_blocks`), so the program comes with the
/// FRAGMENT stage's block table: the same table the fragment stage's own
/// record carried before the link replaced it.
#[cfg(feature = "gpu-v3d")]
pub fn link_sources<'s>(
    vs_glsl: &[u8],
    vs_map: Option<&crate::shaderc::ShcAttribMap>,
    vs_key: crate::shaderc::LodKey,
    fs_glsl: &[u8],
    zero: &[&str],
    scratch: &'s Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    let vs = lower_vertex_mapped(vs_glsl, vs_map, scratch)?;
    /* the vertex stage's c = 0.5 remap (`lodkey`): in the vertex stage's key
     * only, the fragment stage's explicit levels are not remapped */
    let vs = super::lodkey::remap_mag_switch(scratch, vs, vs_key.mag_switch).map_err(|e| CompileError::Rejected {
        line: e.line,
        tok: Token::new(e.tok),
        msg: e.msg,
    })?;
    let fs = lower_source(crate::shaderc::SHC_STAGE_FRAGMENT, fs_glsl, zero, scratch)?;
    let sh = compile_program(&vs, &fs, ProgramKey { vs_lod: vs_key, ..ProgramKey::LINK }, scratch, keep)?;
    Ok((sh, (fs.reflection.blocks, fs.reflection.block_members, [vs.reflection.uniforms, fs.reflection.uniforms])))
}

/// The PowerVR build: the USC chain (`powervr::chain`). A vertex shader goes
/// through selection, preparation, allocation, the builder and the record; a
/// fragment shader too when it reads no varying and writes one colour, and
/// is refused by name otherwise.
#[cfg(feature = "gpu-powervr")]
pub fn compile(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    scratch: &Arena<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, CompileError> {
    compile_linked(stage, glsl, blend_key, &[], scratch, keep)
}

/// [`compile_linked`], and the shader's uniform-block table.
#[cfg(feature = "gpu-powervr")]
pub fn compile_linked_blocks<'s>(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    zero: &[&str],
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    compile_powervr(stage, glsl, blend_key, zero, None, scratch, keep)
}

/// A vertex shader compiled with the attribute locations the link decided
/// (`ShcBackend::compile_vs_linked`), and its uniform-block table.
#[cfg(feature = "gpu-powervr")]
pub fn compile_vs_mapped_blocks<'s>(
    glsl: &[u8],
    map: &crate::shaderc::ShcAttribMap,
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    compile_powervr(crate::shaderc::SHC_STAGE_VERTEX, glsl, 0, &[], Some(map), scratch, keep)
}

/// A vertex shader whose attributes do not fit the vertex-input registers
/// one location each, compiled for its reflection alone: no code, and
/// `layout_at_link` with every declared attribute, so that glLinkProgram
/// compiles it again with the program's locations
/// ([`compile_vs_mapped_blocks`]), where attributes bound to one location
/// share its registers (dEQP-GLES2 attribute_location.bind_aliasing.max_*:
/// 32 declared, 16 locations).
fn deferred_vertex(
    m: &super::ir::Module<'_>,
    attribs: &[super::reflect::AttribRow<'_>],
    keep: &'static Arena<'static>,
    record: impl Fn(super::powervr::abi::UscAbiError) -> CompileError,
) -> Result<ShcShader, CompileError> {
    use super::powervr::abi::{attribs_read, matrix_columns, UscAbiError};
    use crate::shaderc::{ShcAttribBinding, SHC_ATTRIB_NO_LOCATION, SHC_MAX_DECLARED_ATTRIBS};
    if attribs.len() > SHC_MAX_DECLARED_ATTRIBS {
        return Err(record(UscAbiError::TooManyAttribs { got: attribs.len(), max: SHC_MAX_DECLARED_ATTRIBS }));
    }
    /* active: loaded by any LoadInput in the lowered IR, dead branches
     * included (the front end's static use, ES 2.0 2.10.4) */
    let read = attribs_read(m).map_err(&record)?;
    let mut declared = [ShcAttribBinding::ZERO; SHC_MAX_DECLARED_ATTRIBS];
    for (i, (d, r)) in declared.iter_mut().zip(attribs).enumerate() {
        if r.name.len() >= d.name.len() {
            return Err(record(UscAbiError::NameTooLong { index: i, len: r.name.len() }));
        }
        for (c, b) in d.name.iter_mut().zip(r.name.bytes()) {
            *c = b as _;
        }
        d.location = SHC_ATTRIB_NO_LOCATION;
        d.columns = matrix_columns(r.gl_type).unwrap_or(1);
        d.gl_type = r.gl_type;
        d.active = u8::from(
            (0..u32::from(d.columns))
                .any(|c| u32::from(r.location) + c < 32 && read & (1 << (u32::from(r.location) + c)) != 0),
        );
    }
    let kept = keep
        .alloc_copy(&declared[..attribs.len()])
        .map_err(|_| CompileError::OutOfMemory { which: Which::Keep, left: keep.left() })?;
    let mut sh = ShcShader::ZERO;
    sh.meta_off = crate::shaderc::SHC_NO_META;
    sh.vs_psize_off = crate::shaderc::SHC_NO_PSIZE;
    sh.blend_consts_off = crate::shaderc::SHC_NO_BLEND_CONSTS;
    sh.layout_at_link = 1;
    sh.declared = kept.as_ptr();
    sh.num_declared = attribs.len() as u8;
    Ok(sh)
}

/// The PowerVR's compile of one stage, with the zero-varying list and, for
/// a vertex shader at link, the attribute locations.
#[cfg(feature = "gpu-powervr")]
fn compile_powervr<'s>(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    zero: &[&str],
    map: Option<&crate::shaderc::ShcAttribMap>,
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    use super::powervr::alloc::{AllocError, RegFile};
    use super::powervr::chain::{compile_fragment_for, compile_vertex_with, ChainError};
    let st = match stage {
        crate::shaderc::SHC_STAGE_VERTEX => super::sema::Stage::Vertex,
        crate::shaderc::SHC_STAGE_FRAGMENT => super::sema::Stage::Fragment,
        other => return Err(CompileError::BadStage(other)),
    };
    /* A blend variant is a fragment shader with the blend lowered into it
     * (`powervr::blend`); a vertex shader has none. */
    if blend_key != 0 && stage != crate::shaderc::SHC_STAGE_FRAGMENT {
        return Err(CompileError::BadStage(stage));
    }
    let src = core::str::from_utf8(glsl).map_err(|_| CompileError::Rejected {
        line: 0,
        tok: Token::EMPTY,
        msg: "shader source is not valid UTF-8",
    })?;
    let src = scratch.alloc_str(src)?;
    let mark = scratch.used();
    /* The front end's garbage is given back before selection runs: only
     * the module and the reflection survive (`lower::relocate`). */
    /* the c = 0.5 remap of a sampler-state variant (`lodkey`, asked for by
     * the online back end around the compile; 0 in every other compile) */
    let mag_switch = crate::shaderc::compile_lod_key().mag_switch;
    let lowered = super::lower::compile_to_ir(scratch, st, src)
        .and_then(|l| super::lower::zero_varyings(scratch, l, zero))
        .and_then(|l| super::lodkey::remap_mag_switch(scratch, l, mag_switch))
        .and_then(|l| super::lower::relocate(scratch, l, src, mark))
        .map_err(|e| CompileError::Rejected { line: e.line, tok: Token::new(e.tok), msg: e.msg })?;
    let r = if stage == crate::shaderc::SHC_STAGE_VERTEX {
        let (m, attribs) = match map {
            Some(map) => {
                super::powervr::abi::with_attrib_map(scratch, &lowered.module, lowered.reflection.attribs, map)
                    .map_err(|e| CompileError::BackEnd(BackEndError::Usc(ChainError::Record(e))))?
            }
            None => (&lowered.module, lowered.reflection.attribs),
        };
        compile_vertex_with(m, &lowered.reflection, attribs, scratch)
    } else {
        compile_fragment_for(&lowered.module, &lowered.reflection, scratch, blend_key)
    };
    let blocks = (lowered.reflection.blocks, lowered.reflection.block_members, [lowered.reflection.uniforms, &[][..]]);
    match r {
        Ok(sh) => Ok((sh, blocks)),
        /* Too many attributes for one location's registers each: the link
         * knows which of them alias, so the layout waits for it. With the
         * link's map this is a real refusal. */
        Err(ChainError::Alloc(AllocError::OutOfRegisters { file: RegFile::VtxIn, .. }))
            if map.is_none() && stage == crate::shaderc::SHC_STAGE_VERTEX =>
        {
            let record = |e| CompileError::BackEnd(BackEndError::Usc(ChainError::Record(e)));
            Ok((deferred_vertex(&lowered.module, lowered.reflection.attribs, keep, record)?, blocks))
        }
        Err(ChainError::OutOfScratch) => Err(CompileError::OutOfMemory { which: Which::Scratch, left: 0 }),
        Err(ChainError::Record(crate::shaderc::online::powervr::abi::UscAbiError::OutOfMemory { left })) => {
            Err(CompileError::OutOfMemory { which: Which::Keep, left })
        }
        Err(e) => Err(CompileError::BackEnd(BackEndError::Usc(e))),
    }
}

/// The back half, end to end, from a hand-written IR.
///
/// These are the M3 acceptance shaders. They run today, before the front end
/// exists, which is the point: if the chain is wrong it is wrong here rather
/// than on a board where the symptom is a picture.
#[cfg(all(test, feature = "gpu-vivante"))]
mod chain_tests {
    use super::super::store;
    use super::*;
    use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX, SHC_VIV_NO_REG};

    fn io(name: &'static str, kind: ir::IoKind, location: u8) -> ir::IoDecl<'static> {
        ir::IoDecl { name, kind, location, components: 4, interp: ir::INTERP_SMOOTH, gl_type: 0, invariant: false }
    }

    /// `gl_Position = a_pos;`
    #[test]
    fn the_passthrough_vertex_shader_becomes_a_real_shcshader() {
        let _s = store::test_keep();
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(1)), ir::Node::Return];
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            inputs: &inputs,
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };

        let sh = store::with_scratch(|a| assemble_from_ir(&m, a, &[])).unwrap().expect("the chain produces a shader");

        assert_eq!(sh.usc_dw, 4, "one 128-bit instruction is four dwords");
        assert_eq!(sh.usc_words().len(), 4);
        /* All zeros: a `nop`. The attribute arrives in the register the
         * position is read from, so the shader has nothing to do, and the
         * reference program for this source is the same four zeros. */
        assert!(sh.usc_words().iter().all(|&w| w == 0), "a shader with no work is a nop");
        assert_eq!(sh.viv.num_in, 1);
        assert_eq!(sh.viv.vs_pos_out_reg, 0);
        assert_eq!(sh.viv.vs_pointsize_out_reg, SHC_VIV_NO_REG, "absent stays absent");
        /* temps comes from the allocator and counts the io registers, because
         * they live in the same file: aPos r0 and the position out of r0 are
         * one register, so one temp. */
        assert!(sh.temps >= 1, "temps must cover the io registers: {}", sh.temps);
        assert_eq!(sh.viv.const_count, 0, "a passthrough reads no constants");
    }

    /// `gl_FragColor = vec4(1, 0, 0, 1);`
    #[test]
    fn the_literal_colour_fragment_shader_becomes_a_real_shcshader() {
        let _s = store::test_keep();
        let red = [0x3f80_0000u32, 0, 0, 0x3f80_0000];
        let instrs = [
            ir::Op::ConstF32(red),
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(1)), ir::Node::Return];
        let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };

        let sh = store::with_scratch(|a| assemble_from_ir(&m, a, &[])).unwrap().expect("the chain produces a shader");

        assert_eq!(sh.usc_dw, 4);
        assert!(sh.usc_words().iter().any(|&w| w != 0));
        /* Never 0: t0 is reserved in the fragment stage, and no real blob in
         * the offline table writes its colour there (0 of 138). */
        assert_eq!(sh.viv.ps_color_out_reg, 1);
        assert_eq!(sh.num_outputs, 0, "zero on this GPU, both stages");
        /* The colour is LITERAL dwords, so nothing in this shader reads the
         * uniform block. It is packed: the constructor's four components hold
         * two distinct values, so two dwords carry them and the other two
         * lanes of the register are UNUSED -- which is what the shipped
         * compiler produces for the same source, and what a shader reads back
         * through the operand swizzle. */
        assert_eq!(sh.viv.const_count, 4, "a register is four dwords");
        let used: Vec<u32> = sh
            .viv
            .const_contents()
            .iter()
            .zip(sh.viv.const_data())
            .filter(|(c, _)| **c == crate::shaderc::SHC_VIV_CONST_LITERAL)
            .map(|(_, d)| *d)
            .collect();
        let mut want: Vec<u32> = red.to_vec();
        /* sort THEN dedup: dedup alone drops only consecutive repeats. */
        want.sort_unstable();
        want.dedup();
        let mut got = used.clone();
        got.sort_unstable();
        assert_eq!(got, want, "every distinct literal is in the file exactly once");
        assert_eq!(sh.uniform_dwords, 0);
        assert_eq!(sh.blend_consts_off, crate::shaderc::SHC_NO_BLEND_CONSTS, "the pixel engine blends here");
    }

    /// **The M3 acceptance test: GLSL text to a `ShcShader`, in process.**
    ///
    /// Not an `ir::Module` by hand -- the application's own bytes, through the
    /// front end, selection, allocation, the encoder and the ABI assembly.
    /// This is the claim the whole milestone is about.
    #[test]
    fn glsl_text_becomes_a_shader_for_both_m3_shaders() {
        let _s = store::test_keep();

        let vs = b"attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
        let sh = store::with_scratch(|a| compile(SHC_STAGE_VERTEX, vs, 0, a, store::keep()))
            .unwrap()
            .expect("the vertex shader compiles from text");
        assert_eq!(sh.usc_dw, 4, "one instruction");
        /* A nop: nothing to do, see the passthrough test above. */
        assert!(sh.usc_words().iter().all(|&w| w == 0));
        assert_eq!(sh.viv.num_in, 1, "one attribute");
        assert_eq!(sh.viv.vs_pos_out_reg, 0);
        assert!(sh.temps >= 1, "temps covers the io registers: {}", sh.temps);

        /* 1.0 and 0.0 are exactly representable, which matters: an immediate
         * carries the top 20 bits of the f32 and `imm_f32` refuses a value it
         * would have to round. A colour that rounded would be a picture that
         * is merely wrong, and the board's pixel read-back would be comparing
         * against the value we meant rather than the one we sent. */
        let fs = b"precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }\n";
        let sh = store::with_scratch(|a| compile(SHC_STAGE_FRAGMENT, fs, 0, a, store::keep()))
            .unwrap()
            .expect("the fragment shader compiles from text");
        assert_eq!(sh.usc_dw, 4);
        assert!(sh.usc_words().iter().any(|&w| w != 0));
        assert_eq!(sh.viv.ps_color_out_reg, 1, "t0 is reserved in the fragment stage");
        assert_eq!(sh.num_outputs, 0, "zero on this GPU, both stages");
        assert_eq!(sh.uniform_dwords, 0, "the colour is a literal, so there is no uniform block");
    }

    /// A shader the front end refuses must come back as a rejection, not as a
    /// compiler defect: the two send a reader to different places.
    #[test]
    fn a_shader_the_front_end_refuses_is_a_rejection() {
        let _s = store::test_keep();
        let bad = b"void main() { this is not glsl }\n";
        let r = store::with_scratch(|a| compile(SHC_STAGE_FRAGMENT, bad, 0, a, store::keep())).unwrap();
        /* `ShcShader` is the C ABI struct and implements no Debug, so the Ok
         * arm cannot be unwrapped for a message; match instead. */
        match r {
            Err(CompileError::Rejected { line, tok, msg }) => {
                assert!(!msg.is_empty(), "a rejection has to say why: line {line}");
                /* And WHICH: the token is the half that was being dropped at
                 * this boundary, and a rejection without it cannot be
                 * located in the source or bucketed in a log. */
                assert!(!tok.is_empty(), "a rejection has to name the construct: {line} {msg}");
                assert!(line > 0, "a rejection has to say where: {tok:?} {msg}");
            }
            Err(other) => panic!("a bad source is a rejection, not {other:?}"),
            Ok(_) => panic!("a source that is not GLSL must not compile"),
        }
    }

    /// The encoded words must decode back to the instruction that was built.
    ///
    /// Without this the chain could produce plausible dwords that are not the
    /// program: the encoder round-trips the whole
    /// offline table, but nothing yet checked that *this* path feeds it what
    /// it thinks it does.
    #[test]
    fn what_the_chain_encodes_decodes_back_to_a_mov() {
        let _s = store::test_keep();
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            /* `.yzwx`, so this is a real permutation and not a copy of the
             * register onto itself -- which would be elided, and would leave
             * this test decoding the nop that a passthrough compiles to. */
            ir::Op::StoreOutput {
                slot: ir::IoSlot::Position,
                src: ir::Src { val: ir::ValueId(0), swizzle: [1, 2, 3, 0] },
            },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(1)), ir::Node::Return];
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            inputs: &inputs,
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let sh = store::with_scratch(|a| assemble_from_ir(&m, a, &[])).unwrap().expect("assembles");
        let w = sh.usc_words();
        let dw: [u32; 4] = [w[0], w[1], w[2], w[3]];
        let back = super::super::vivante::decode::decode_instr(&dw).expect("the words decode");
        /* A permutation is real work, so this one is a `mov` rather than the
         * nop a straight passthrough compiles to. */
        assert_eq!(back.op().name, "mov", "what came out is the instruction that went in");
    }
}

/// The PowerVR build's compile entry: what the GL core gets back.
#[cfg(all(test, feature = "gpu-powervr"))]
mod tests {
    use super::*;
    use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

    fn compile_err(stage: u32, src: &[u8]) -> CompileError {
        let _l = super::super::store::test_keep();
        let r = super::super::store::with_scratch(|s| compile(stage, src, 0, s, super::super::store::keep()))
            .expect("the scratch region is free");
        assert!(r.is_err(), "expected a refusal");
        r.err().unwrap()
    }

    #[test]
    fn a_bad_stage_is_refused_before_anything_else() {
        /* A stage number neither the GL core nor the table knows is a caller
         * defect, and it has to stay distinguishable from "the compiler is not
         * written yet" -- otherwise, the day the compiler lands, the same call
         * starts compiling something for a stage nobody meant. */
        assert_eq!(compile_err(7, b"void main(){}"), CompileError::BadStage(7));
    }

    /// A fragment shader the USC half cannot build yet is refused as a
    /// back-end gap -- `-ENOTSUP`, not a verdict on the source -- naming why.
    /// An output written inside a loop kept as a loop is one.
    #[test]
    fn a_fragment_shader_the_back_end_cannot_build_is_a_named_gap_not_a_rejection() {
        let e = compile_err(
            SHC_STAGE_FRAGMENT,
            b"precision mediump float; uniform int n; void main(){ for (int i = 0; i < 4; i++) { if (i >= n) break; gl_FragColor = vec4(1.0); } }",
        );
        assert!(matches!(e, CompileError::BackEnd(BackEndError::Usc(_))), "{e:?}");
    }

    /// es100_drawstep's fragment shader: the record pco gives it, and pco's
    /// program to the byte (`pco_shc` build-new on the same text, 2026-09-23:
    /// four moves into temp0..3, `pck` u8888 rpt 4 into temp4 carrying the
    /// six-byte alignment block, the pixel output write with `olchk` and
    /// `end` starting on byte 56).
    #[test]
    fn the_drawstep_fragment_shader_is_pcos_to_the_byte() {
        let _l = super::super::store::test_keep();
        let r = super::super::store::with_scratch(|s| {
            compile(
                SHC_STAGE_FRAGMENT,
                b"void main(){ gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }",
                0,
                s,
                super::super::store::keep(),
            )
            .map(|sh| {
                // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
                let code = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }.to_vec();
                (
                    (sh.temps, sh.num_outputs, sh.num_varyings, sh.meta_off, sh.blend_consts_off),
                    (sh.num_attribs, sh.vs_output_size, sh.vs_psize_off, sh.uniform_dwords, sh.num_uniforms),
                    code,
                )
            })
        })
        .expect("the scratch region is free");
        let Ok((a, b, code)) = r else { panic!("the fragment shader did not compile: {:?}", r.err()) };
        /* pco's record: temps 5, num_outputs 1, meta_off 0xffff,
         * blend_consts_off 0xff, and zero for the vertex-only fields. */
        assert_eq!(a, (5, 1, 0, 0xffff, 0xff));
        assert_eq!(b, (0, 0, 0, 0, 0));
        assert_eq!(
            code,
            [
                0x87008235, 0x00000180, 0x82344000, 0x00008700, 0x82344100, 0x00008700, 0x82354200, 0x01808700,
                0x43000000, 0x9c06925a, 0xa0408020, 0x2c001040, 0xfff3ff44, 0xffffffff, 0x87808a34, 0x20000044
            ],
            "the drawstep fragment shader differs from pco's"
        );
    }

    /// es100_drawstep's vertex shader, through the entry the GL core calls:
    /// a record with its attribute, the code, and the output size.
    #[test]
    fn the_drawstep_vertex_shader_compiles_to_a_record() {
        let _l = super::super::store::test_keep();
        let r = super::super::store::with_scratch(|s| {
            compile(
                SHC_STAGE_VERTEX,
                b"attribute vec4 a_pos; void main(){ gl_Position = a_pos; }",
                0,
                s,
                super::super::store::keep(),
            )
            .map(|sh| {
                // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
                let code = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }.to_vec();
                (
                    sh.num_attribs,
                    sh.attribs[0].vi_reg,
                    sh.attribs[0].components,
                    sh.vs_output_size,
                    sh.num_varyings,
                    code,
                )
            })
        })
        .expect("the scratch region is free");
        let Ok((na, vi, comps, out, nv, code)) = r else { panic!("the vertex shader did not compile") };
        assert_eq!((na, vi, comps, out, nv), (1, 0, 4, 4, 0));
        /* One write of vtxin0..3 to dwords 0..3 and the end of task: nothing
         * to compute. */
        let bytes: std::vec::Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let (groups, len, ended) = super::super::powervr::assemble::walk_shader(&bytes).expect("decodes");
        assert_eq!((groups, len, ended), (2, bytes.len(), true));
        /* And it is pco's program to the byte: `pco_shc` (the laptop's
         * build-new, 2026-09-23) on this exact text, `#version 100` prepended
         * as glsl2usc does, gives these six dwords -- `uvsw.write` rpt 4 of
         * vtxin0 at imm_addr 0 carrying the six-byte alignment block, then the
         * end of task starting on byte 16. */
        assert_eq!(
            code,
            [0x0806a058, 0x00048000, 0xfff33000, 0xffffffff, 0x0580a044, 0xff000000],
            "the drawstep vertex shader differs from pco's"
        );
    }

    /// A smooth vec4 varying written straight out: pco's program to the
    /// byte (pco_shc build-new, 2026-09-23) -- fence, `ditrp` of four
    /// components from coefficient 4 into temp0..3, fence, `wdf`, then the
    /// pack straight from temp0 and the pixel output write -- and pco's
    /// record: one varying at vertex-output dword 4, W used.
    #[test]
    fn a_varying_passed_through_is_pcos_program_to_the_byte() {
        let _l = super::super::store::test_keep();
        let r = super::super::store::with_scratch(|s| {
            compile(
                SHC_STAGE_FRAGMENT,
                b"#version 100
precision mediump float;
varying vec4 v_color;
void main(){ gl_FragColor = v_color; }
",
                0,
                s,
                super::super::store::keep(),
            )
            .map(|sh| {
                // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
                let code = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }.to_vec();
                ((sh.temps, sh.num_varyings, sh.uses_w, sh.varyings[0].vtxout_off, sh.varyings[0].components), code)
            })
        })
        .expect("the scratch region is free");
        let Ok((rec, code)) = r else { panic!("the fragment shader did not compile: {:?}", r.err()) };
        assert_eq!(rec, (5, 1, 1, 4, 4));
        assert_eq!(
            code,
            [
                0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000, 0x00080061, 0x80040000,
                0x00050078, 0xc0570040, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104, 0x00000008, 0x00618104,
                0x00000008, 0xff6a8002, 0x9c069258, 0xa0408020, 0x2c001040, 0xfff1ff44, 0x87808a34, 0x20000044
            ],
            "differs from pco's"
        );
    }

    #[test]
    fn every_missing_stage_names_its_milestone_and_every_arena_its_knob() {
        for s in [Stage::FrontEnd, Stage::CodeGen, Stage::Encode, Stage::Abi] {
            let text = s.describe();
            assert!(text.contains("(M"), "{s:?} has to name the milestone that owns it: {text}");
        }
        assert!(Which::Scratch.knob().ends_with("SCRATCH_BYTES"));
        assert!(Which::Keep.knob().ends_with("KEEP_BYTES"));
        assert_eq!((Which::Scratch.name(), Which::Keep.name()), ("scratch", "keep"));
    }
}

/// Does a compiled shader survive the NEXT compile?
///
/// A program is two shaders, and the driver uploads the vertex shader at draw
/// time -- long after the fragment shader was compiled. If any region the
/// `ShcShader` points at lived in the scratch arena, which is reset at the top
/// of every compile, the second compile would overwrite the first's bytes and
/// the program that reaches the GPU would not be the one anything checked.
///
/// The oracle harness cannot see this: it compiles one source and compares it
/// immediately, so it passes by construction however the memory is owned.
/// Only where a code generator exists: on a build without one the compiler
/// refuses with `-ENOTSUP`, correctly, and there is no compiled shader whose
/// memory could outlive anything.
#[cfg(all(test, feature = "gpu-vivante"))]
mod lifetime_tests {
    use super::super::{backend, store};
    use crate::shaderc::{ShcShader, ShcUniform, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
    use core::ffi::c_char;

    const VS: &str = "attribute vec4 a_pos; void main(){ gl_Position = a_pos; }\0";
    const FS: &str = "precision mediump float;\nvoid main(){ gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }\0";

    fn compile(stage: u32, src: &str) -> ShcShader {
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let f = backend::SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, a log of the size given.
        let rc = unsafe { f(stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        // SAFETY: the back end NUL-terminates the log.
        let why = unsafe { crate::util::cstr_bytes(log.as_ptr()) };
        assert_eq!(rc, 0, "compiles; rc {rc}: {}", core::str::from_utf8(why).unwrap_or("<not utf8>"));
        out
    }

    /// Every byte the shader points at, copied out so it can be compared later.
    fn snapshot(sh: &ShcShader) -> (Vec<u32>, Vec<u8>, Vec<u32>, Vec<u8>) {
        // SAFETY: the shader's own lengths describe its own regions.
        let usc = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }.to_vec();
        let (contents, data) = {
            let n = sh.viv.const_count as usize;
            if n == 0 {
                (Vec::new(), Vec::new())
            } else {
                // SAFETY: as above.
                unsafe {
                    (
                        core::slice::from_raw_parts(sh.viv.const_contents, n).to_vec(),
                        core::slice::from_raw_parts(sh.viv.const_data, n).to_vec(),
                    )
                }
            }
        };
        let uni = if sh.num_uniforms == 0 {
            Vec::new()
        } else {
            // SAFETY: as above; compared as raw bytes because ShcUniform is POD.
            unsafe {
                core::slice::from_raw_parts(
                    sh.uniforms as *const u8,
                    sh.num_uniforms as usize * size_of::<ShcUniform>(),
                )
            }
            .to_vec()
        };
        (usc, contents, data, uni)
    }

    #[test]
    fn the_vertex_shader_is_intact_after_the_fragment_shader_is_compiled() {
        let _s = store::test_lock();
        let vs = compile(SHC_STAGE_VERTEX, VS);
        let before = snapshot(&vs);
        assert!(!before.0.is_empty(), "the vertex shader has code to lose");

        let _fs = compile(SHC_STAGE_FRAGMENT, FS);

        let after = snapshot(&vs);
        assert_eq!(after.0, before.0, "the vertex shader's instructions");
        assert_eq!(after.1, before.1, "its constant-file contents");
        assert_eq!(after.2, before.2, "its constant-file data");
        assert_eq!(after.3, before.3, "its uniform rows");
    }

    #[test]
    fn a_shader_from_the_cache_points_at_the_same_bytes() {
        /* The cache hands back a shader compiled earlier still. If its regions
         * were scratch, a hit would return a pointer into memory some later
         * compile had already reused. */
        let _s = store::test_lock();
        let first = compile(SHC_STAGE_VERTEX, VS);
        let snap = snapshot(&first);
        let _other = compile(SHC_STAGE_FRAGMENT, FS);
        let again = compile(SHC_STAGE_VERTEX, VS);
        assert_eq!(again.usc, first.usc, "the same region, not a recompile");
        assert_eq!(snapshot(&again), snap);
        assert_eq!(snapshot(&first), snap, "and the first handle still reads it");
    }
}

/// Does a compile actually reach `builtins::lower`?
///
/// The `native`/`lower` split is invisible from outside the compiler: both
/// produce instructions and a shader that took the wrong path still assembles.
/// So the question "is the lowering library reachable from a compile?" cannot
/// be answered by reading either file -- it was answered by two people in
/// opposite directions on the same afternoon -- and this asks the tree.
#[cfg(all(test, feature = "gpu-vivante"))]
mod lowered_ops {
    use super::*;
    use crate::shaderc::online::vivante::decode::decode_instr;
    use crate::shaderc::SHC_STAGE_FRAGMENT;

    /// The op names of a compiled shader, in order.
    fn opcodes(sh: &crate::shaderc::ShcShader) -> std::vec::Vec<&'static str> {
        /* SAFETY: `usc`/`usc_dw` are what the compiler just wrote into the
         * keep arena, which outlives the test. */
        let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        let (whole, rest) = dw.as_chunks::<4>();
        assert!(rest.is_empty(), "a shader is a whole number of 128-bit instructions");
        whole.iter().map(|w| decode_instr(w).expect("the compiler's own output decodes").op().name).collect()
    }

    /// Compile one fragment shader.
    ///
    /// `store::test_lock()` first: the scratch region is a single global and
    /// `with_scratch` REFUSES a second caller rather than blocking, which is
    /// the contract it exists to enforce. The harness runs tests on several
    /// threads, so without the guard two compiling tests refuse each other at
    /// random -- and the refusal surfaces in the OTHER test, as a compile that
    /// returns -22, which reads like a compiler defect. That is what this cost
    /// before I found the guard that store.rs already provides for exactly
    /// this.
    fn compile_fs(src: &str) -> crate::shaderc::ShcShader {
        let _serial = super::super::store::test_lock();
        super::super::store::with_scratch(|s| {
            compile(SHC_STAGE_FRAGMENT, src.as_bytes(), 0, s, super::super::store::keep())
        })
        .expect("the scratch region is free")
        .expect("compiles")
    }

    #[cfg(not(feature = "report-es2"))]
    fn compile_vs(src: &str) -> crate::shaderc::ShcShader {
        let _serial = super::super::store::test_lock();
        super::super::store::with_scratch(|s| {
            compile(crate::shaderc::SHC_STAGE_VERTEX, src.as_bytes(), 0, s, super::super::store::keep())
        })
        .expect("the scratch region is free")
        .expect("compiles")
    }

    /// The program as listing lines (`yakogl-tools etna-dis` text, without
    /// the index and the words).
    fn listing(sh: &crate::shaderc::ShcShader) -> std::vec::Vec<std::string::String> {
        // SAFETY: as in `opcodes`.
        let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        dw.as_chunks::<4>()
            .0
            .iter()
            .map(|w| {
                let mut s = std::string::String::new();
                super::super::vivante::listing::body_text(&mut s, &decode_instr(w).expect("decodes")).expect("text");
                s
            })
            .collect()
    }

    /// Is component `lane` ('x', 'y', ...) of register `reg` read before any
    /// instruction writes `reg`? A source `t<reg>.<swizzle>` reads, for each
    /// lane the destination writes, the component its swizzle names there;
    /// the destination is the first operand after the opcode.
    #[cfg(not(feature = "report-es2"))]
    fn read_before_written(lines: &[std::string::String], reg: u8, lane: char) -> bool {
        let name = std::format!("t{reg}.");
        for l in lines {
            let mut ops = l.split_once(' ').map_or("", |(_, r)| r).split(',').map(str::trim);
            let dst = ops.next().unwrap_or("");
            /* the lanes written: "t1.x_z_" names them, a bare "t1" is all four */
            let mask: std::vec::Vec<bool> = match dst.split_once('.') {
                Some((_, m)) => m.chars().map(|c| c != '_').collect(),
                None => std::vec![true; 4],
            };
            let reads = ops.any(|o| {
                o.trim_start_matches('-')
                    .strip_prefix(name.as_str())
                    .is_some_and(|swz| swz.chars().zip(&mask).any(|(c, w)| *w && c == lane))
            });
            if reads {
                return true;
            }
            if dst.starts_with(&name) {
                return false;
            }
        }
        false
    }

    /// gl_test's `es300/vid.vert` (cases 43, 44, 45, 49, 57, 98).
    #[cfg(not(feature = "report-es2"))]
    const VID_VS: &str = "#version 300 es\nlayout(location = 0) in vec4 pos;\nout vec4 v_col;\nvoid main()\n{\n    \
        int i = gl_VertexID % 4;\n    v_col = vec4(i == 0 || i == 3 ? 1.0 : 0.0, i == 1 || i == 3 ? 1.0 : 0.0, \
        i == 2 || i == 3 ? 1.0 : 0.0, 1.0);\n    gl_Position = pos;\n}\n";

    /// `gl_VertexID` is the `.x` of the register after the attributes, as
    /// Mesa's etnaviv puts it and as the reference compiler did for this
    /// very source: `vs_id_in_reg` 1 after `pos` in register 0, and the
    /// program reads `t1.xxxx` before anything writes t1 (the reference's
    /// first instruction is `iabs.s32 t1._y__, void, void, t1.xxxx`). The
    /// online compiler refused it until this ("this built-in input is not
    /// selected yet", GitHub issue #11).
    /* ES 3.00 sources: a report-es2 build refuses them in the front end. */
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn gl_vertex_id_is_x_of_the_register_after_the_attributes() {
        let _k = super::super::store::test_keep();
        let sh = compile_vs(VID_VS);
        assert_eq!(sh.viv.vs_id_in_reg, 1, "the reference's register: one past `pos` in t0");
        assert_eq!(sh.viv.num_in, 1, "the id register is not an io row: the driver programs it by ID_CONFIG");
        assert_eq!(sh.viv.in_[0].reg, 0, "pos stays in t0");
        assert!(sh.temps > 1, "temps covers the id register: {}", sh.temps);
        let lines = listing(&sh);
        assert!(read_before_written(&lines, 1, 'x'), "t1.x is read as the vertex id before t1 is written: {lines:#?}");
        /* The instrument tells the lanes apart: the instance id is not read. */
        assert!(!read_before_written(&lines, 1, 'y'), "nothing reads t1.y here: {lines:#?}");
    }

    /// `gl_InstanceID` is the `.y` of the same register (etnaviv_compiler_nir_ra.c
    /// pins load_instance_id to REG_TYPE_VIRT_SCALAR_Y, and the driver writes
    /// the instance id to `reg * 4 + 1`).
    /* ES 3.00 sources: a report-es2 build refuses them in the front end. */
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn gl_instance_id_is_y_of_the_same_register() {
        let _k = super::super::store::test_keep();
        let sh = compile_vs(
            "#version 300 es\nlayout(location = 0) in vec4 pos;\nvoid main() { gl_Position = pos + vec4(float(gl_InstanceID)); }\n",
        );
        assert_eq!(sh.viv.vs_id_in_reg, 1);
        let lines = listing(&sh);
        assert!(read_before_written(&lines, 1, 'y'), "t1.y is read as the instance id: {lines:#?}");
        assert!(!read_before_written(&lines, 1, 'x'), "and not the vertex id in t1.x: {lines:#?}");
    }

    /// "After the attributes" counts locations, every column of a matrix
    /// attribute included: a vertex element feeds each column's register, so
    /// an id register below the last one would be overwritten by the fetch.
    /* ES 3.00 sources: a report-es2 build refuses them in the front end. */
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn the_id_register_clears_every_attribute_column() {
        let _k = super::super::store::test_keep();
        let sh = compile_vs(
            "#version 300 es\nlayout(location = 0) in vec4 pos;\nlayout(location = 1) in mat2 m;\n\
             void main() { gl_Position = pos + vec4(m[0], m[1]) * float(gl_VertexID); }\n",
        );
        assert_eq!(sh.viv.vs_id_in_reg, 3, "pos in t0, m's columns in t1 and t2, the ids in t3");
        let lines = listing(&sh);
        assert!(read_before_written(&lines, 3, 'x'), "t3.x is the vertex id: {lines:#?}");
        /* A shader that reads neither has no id register, so the driver
         * turns ID_CONFIG off (vivante/shader.rs writes it on every draw). */
        let plain =
            compile_vs("#version 300 es\nlayout(location = 0) in vec4 pos;\nvoid main() { gl_Position = pos; }\n");
        assert_eq!(plain.viv.vs_id_in_reg, crate::shaderc::SHC_VIV_NO_REG, "absent stays absent");
    }

    #[cfg(not(feature = "report-es2"))]
    const ARRAY_FS: &str = include_str!("../../../tools/shaders/es300/array.frag");
    #[cfg(not(feature = "report-es2"))]
    const VOL_FS: &str = include_str!("../../../tools/shaders/es300/vol.frag");
    #[cfg(not(feature = "report-es2"))]
    const TEXSIZE_FS: &str = include_str!("../../../tools/shaders/es300/texsize.frag");

    /// The fields of `ours` that differ from `reference`, leaving out the ones
    /// that name registers and lanes (the allocator's choice, not the
    /// encoding's).
    #[cfg(not(feature = "report-es2"))]
    fn differing_fields(ours: &[u32; 4], reference: &[u32; 4]) -> std::vec::Vec<&'static str> {
        differing_fields_but(ours, reference, &["DST", "TEX_SWIZ", "SRC0_REG", "SRC0"])
    }

    /// [`differing_fields`] leaving out `skip`.
    fn differing_fields_but(ours: &[u32; 4], reference: &[u32; 4], skip: &[&str]) -> std::vec::Vec<&'static str> {
        let (a, b) =
            (decode_instr(ours).expect("ours decodes"), decode_instr(reference).expect("the reference decodes"));
        if a.op().name != b.op().name {
            return std::vec!["the operation"];
        }
        a.op()
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !skip.contains(&f.name))
            .filter(|(k, _)| a.vals[*k] != b.vals[*k])
            .map(|(_, f)| f.name)
            .collect()
    }

    fn words(sh: &crate::shaderc::ShcShader) -> std::vec::Vec<[u32; 4]> {
        // SAFETY: as in `opcodes`.
        let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        dw.as_chunks::<4>().0.to_vec()
    }

    /// `texelFetch` is the reference's `txf` in every field but the
    /// registers: the level 0 as the f20 immediate 0.0 in SRC1 and 0x1100 in
    /// SRC2, as Mesa's etnaviv emits it. The reference words are the GC7000
    /// table's for gl_test's `es300/array.frag` (case 42): `txf t2.___w,
    /// tex0.xxxw, t1.xyzw, 0.0:f20, 4352:s20`. Before this the online
    /// compiler refused the shader ("textureSize is not selected yet").
    /* ES 3.00 sources: a report-es2 build refuses them in the front end. */
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn texel_fetch_is_the_reference_txf() {
        let _k = super::super::store::test_keep();
        const REF_TXF: [u32; 4] = [0x0402_1009, 0x3900_1e00, 0x0001_0040, 0x7402_100f];
        const REF_TEXLD: [u32; 4] = [0x0382_1018, 0x3900_2920, 0x0000_0000, 0x0000_0000];
        for (name, src) in [("array.frag", ARRAY_FS), ("vol.frag", VOL_FS)] {
            let sh = compile_fs(src);
            let ws = words(&sh);
            let txf: std::vec::Vec<&[u32; 4]> =
                ws.iter().filter(|w| decode_instr(w).expect("decodes").op().name == "txf").collect();
            assert_eq!(txf.len(), 1, "{name}: one texelFetch, one txf: {:#?}", listing(&sh));
            assert_eq!(differing_fields(txf[0], &REF_TXF), std::vec::Vec::<&str>::new(), "{name}: {:#?}", listing(&sh));
            /* The comparison can fail: a txf is not the reference's texld. */
            assert!(!differing_fields(txf[0], &REF_TEXLD).is_empty());
        }
    }

    /// A 3D or 2D array look-up is the 2D instruction with a three-lane
    /// coordinate: `texture()` a `texld`, `textureLod()` a `texldl`, as the
    /// reference compiled gl_test's `es300/vol.frag` (case 48) and
    /// `es300/array.frag` (case 42).
    /* ES 3.00 sources: a report-es2 build refuses them in the front end. */
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn volume_and_array_lookups_are_the_2d_instructions() {
        let _k = super::super::store::test_keep();
        let vol = opcodes(&compile_fs(VOL_FS));
        for want in ["texld", "texldl", "txf"] {
            assert_eq!(vol.iter().filter(|o| **o == want).count(), 1, "vol.frag: one {want}: {vol:?}");
        }
        let array = opcodes(&compile_fs(ARRAY_FS));
        assert_eq!(array.iter().filter(|o| **o == "texld").count(), 1, "array.frag: {array:?}");
    }

    /// `textureSize(s, 0)` reads the width, height and depth the driver puts
    /// in the constant file for the sampler's unit (contents 5, 6, 7, data =
    /// the unit), as the reference did for gl_test's `es300/texsize.frag`
    /// (case 46): u0 for `tex` on unit 0, u1 for `arr` on unit 1. Another
    /// level is refused by name.
    /* ES 3.00 sources: a report-es2 build refuses them in the front end. */
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn texture_size_reads_the_driver_constants_of_its_unit() {
        let _k = super::super::store::test_keep();
        let sh = compile_fs(TEXSIZE_FS);
        let mut got: std::vec::Vec<(u8, u32)> = sh
            .viv
            .const_contents()
            .iter()
            .zip(sh.viv.const_data())
            .filter(|(c, _)| (5..=7).contains(*c))
            .map(|(c, d)| (*c, *d))
            .collect();
        got.sort_unstable();
        assert_eq!(got, [(5, 0), (5, 1), (6, 0), (6, 1), (7, 0), (7, 1)], "{:#?}", listing(&sh));
        let level1 = "#version 300 es\nprecision mediump float;\nuniform sampler2D tex;\nout vec4 c;\n\
                      void main() { c = vec4(float(textureSize(tex, 1).x)); }\n";
        let err = {
            let _serial = super::super::store::test_lock();
            super::super::store::with_scratch(|s| {
                compile(SHC_STAGE_FRAGMENT, level1.as_bytes(), 0, s, super::super::store::keep())
            })
            .expect("the scratch region is free")
            .err()
            .expect("a level other than 0 is refused")
        };
        assert!(
            std::format!("{err:?}").contains("textureSize at a level other than a literal 0 is not selected yet"),
            "{err:?}"
        );
    }

    /// `dFdx` / `dFdy` are `dsx` / `dsy` with the source in SRC0 and SRC2
    /// alike, as Mesa's etnaviv emits them (`.src[0] = src, .src[1] = src`)
    /// and as the reference did in the GC7000 table (e.g. `dsx t8.xyz_,
    /// t1.xyzw, void, t1.xyzw`, words below), and `dFdy`'s source is first
    /// multiplied by the per-draw y scale `gl_FragCoord` takes (contents
    /// 12), as nir_lower_wpos_ytransform's lower_ddy does (the reference,
    /// etna_shc, runs no GL state tracker and has no such step). gl_test's
    /// `es100/deriv.frag` (case 138) was refused with
    /// Alloc(WrongSourceSlot(2)) before this, as were 14 Qt derivative
    /// shaders.
    #[test]
    fn derivatives_fill_both_slots_and_dfdy_takes_the_y_scale() {
        let _k = super::super::store::test_keep();
        let sh = compile_fs(include_str!("../../../tools/shaders/es100/deriv.frag"));
        let lines = listing(&sh);
        let ds: std::vec::Vec<&std::string::String> =
            lines.iter().filter(|l| l.starts_with("dsx") || l.starts_with("dsy")).collect();
        assert_eq!(ds.len(), 4, "dFdx twice, dFdy twice: {lines:#?}");
        for l in &ds {
            let ops: std::vec::Vec<&str> = l.split_once(' ').map_or("", |(_, r)| r).split(',').map(str::trim).collect();
            assert_eq!(ops.len(), 4, "{l}");
            assert_eq!(ops[2], "void", "SRC1 stays empty: {l}");
            assert_eq!(ops[1], ops[3], "the source in SRC0 and SRC2: {l}");
        }
        /* Decode-back: every field but the registers and lanes equals the
         * reference's dsx and dsy (and a dsx is not a dsy). */
        const REF_DSX: [u32; 4] = [0x0388_1007, 0x3900_1804, 0x0000_0000, 0x0039_0018];
        const REF_DSY: [u32; 4] = [0x0381_1008, 0x3900_1804, 0x0000_0000, 0x0039_0018];
        let regs = ["DST", "SRC0_REG", "SRC0", "SRC2_REG", "SRC2"];
        let ws = words(&sh);
        for (name, reference, other) in [("dsx", REF_DSX, REF_DSY), ("dsy", REF_DSY, REF_DSX)] {
            let ours = ws.iter().find(|w| decode_instr(w).expect("decodes").op().name == name).expect("emitted");
            assert_eq!(differing_fields_but(ours, &reference, &regs), std::vec::Vec::<&str>::new(), "{name}");
            assert!(!differing_fields_but(ours, &other, &regs).is_empty(), "{name} is not the other one");
        }
        /* the y scale is in the constant file, and a mul reads it */
        let scale = sh.viv.const_contents().iter().position(|c| *c == crate::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_SCALE);
        let scale = scale.expect("dFdy reads the y scale the driver fills");
        let name = std::format!("u{}.{}", scale / 4, ['x', 'y', 'z', 'w'][scale % 4]);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("mul") && l.contains(&std::format!("{name}{0}{0}{0}", &name[name.len() - 1..]))),
            "a mul by {name}: {lines:#?}"
        );
    }

    /// `gl_PointSize` is clamped to the reported range: two `select`s (the
    /// max against 1, the min against 64) before the output register. The
    /// GC7000 draws a larger point than GL_ALIASED_POINT_SIZE_RANGE says
    /// without them (dEQP rasterization.limits.points).
    #[test]
    fn a_point_size_is_clamped_to_the_reported_range() {
        let _k = super::super::store::test_keep();
        let sh = {
            let _serial = super::super::store::test_lock();
            super::super::store::with_scratch(|s| {
                compile(
                    crate::shaderc::SHC_STAGE_VERTEX,
                    b"attribute vec4 a_pos;\nuniform float u_size;\nvoid main() { gl_Position = a_pos; gl_PointSize = u_size; }",
                    0,
                    s,
                    super::super::store::keep(),
                )
            })
            .expect("the scratch region is free")
            .expect("compiles")
        };
        let ops = opcodes(&sh);
        assert_eq!(ops.iter().filter(|o| **o == "select").count(), 2, "max then min; got {ops:?}");
        assert_ne!(sh.viv.vs_pointsize_out_reg, crate::shaderc::SHC_VIV_NO_REG);
    }

    /// A fragment shader that reads `gl_FrontFacing` says so, and compiles a
    /// second time under `SHC_VIV_KEY_FRONT_FLIPPED` with the other sense:
    /// without either, the draw never asks for the variant and both windings
    /// read the hardware value one way (dEQP builtin_variable.frontfacing on
    /// the default framebuffer, i.MX8MP, 2362422d).
    #[test]
    fn a_front_facing_shader_has_a_flipped_variant() {
        let _k = super::super::store::test_keep();
        let one = |key: u32, src: &str| {
            let _serial = super::super::store::test_lock();
            super::super::store::with_scratch(|s| {
                compile(SHC_STAGE_FRAGMENT, src.as_bytes(), key, s, super::super::store::keep())
            })
            .expect("the scratch region is free")
        };
        let facing = "precision mediump float;\nvoid main() { gl_FragColor = gl_FrontFacing ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(0.0, 0.0, 1.0, 1.0); }";
        let base = one(0, facing).expect("compiles");
        let flipped = one(crate::shaderc::SHC_VIV_KEY_FRONT_FLIPPED, facing).expect("the variant compiles");
        assert_eq!(base.reads_front_facing, 1);
        assert_eq!(flipped.reads_front_facing, 1);
        let words = |sh: &crate::shaderc::ShcShader| {
            // SAFETY: as in `opcodes`.
            unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }.to_vec()
        };
        assert_ne!(words(&base), words(&flipped), "the two senses are two binaries");
        let plain = one(0, "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }").expect("compiles");
        assert_eq!(plain.reads_front_facing, 0, "only a shader that reads it asks for the variant");
        assert!(one(2, facing).is_err(), "no other key exists on this GPU");
    }

    /// A texture look-up reaches the hardware as a `texld`.
    ///
    /// The whole path in one assertion: the GLSL name becomes a `TexKind` in
    /// `builtin_call.rs`, the IR carries `Op::Tex`, `select.rs` hands it to
    /// `builtins::texture`, and the allocator encodes `TEX_ID` and `TEX_SWIZ`.
    /// Every one of those is separately tested; this is the one that fails if
    /// any pair of them stops being wired to each other.
    #[test]
    fn a_shader_that_samples_a_texture_compiles_to_a_texld() {
        /* compile_fs's shaders live in keep: held for the whole test */
        let _k = super::super::store::test_keep();
        let sh = compile_fs(
            "precision mediump float;\nuniform sampler2D s;\nvarying vec2 v_uv;\nvoid main() { gl_FragColor = texture2D(s, v_uv); }",
        );
        let ops = opcodes(&sh);
        assert!(ops.contains(&"texld"), "texture2D must reach a texld; got {ops:?}");
    }

    /// `min` is NOT native on this machine -- there is no `min` opcode, and
    /// `builtins::lower` expands it to one `select` carrying `COND = Gt` -- so
    /// a `select` in the output is proof the lowering library ran. `sqrt` IS
    /// native, so a shader built on it has no reason to contain one.
    ///
    /// Both halves are in ONE test because the scratch region is a single
    /// global: two tests calling `with_scratch` run concurrently under the
    /// default harness and the second finds it taken, which fails as "the
    /// scratch region is free" and reads like a compiler defect rather than a
    /// test that shares a resource.
    #[test]
    fn a_compile_reaches_the_lowering_library_and_only_when_it_must() {
        /* compile_fs's shaders live in keep: held for the whole test */
        let _k = super::super::store::test_keep();
        let lowered = opcodes(&compile_fs(
            "precision mediump float;\nuniform vec4 a;\nuniform vec4 b;\nvoid main() { gl_FragColor = min(a, b); }",
        ));
        assert!(lowered.contains(&"select"), "min must lower to a select; got {lowered:?}");

        let native =
            opcodes(&compile_fs("precision mediump float;\nuniform vec4 a;\nvoid main() { gl_FragColor = sqrt(a); }"));
        assert!(native.contains(&"sqrt"), "the native opcode is the one used; got {native:?}");
        assert!(!native.contains(&"select"), "nothing here needs a select; got {native:?}");
    }
}

/// What the compiler emits for a TWO-SAMPLER fragment shader.
///
/// Written for #52(f) case 2, where four checks -- every two-texture-unit draw
/// -- come back black on the board while all 25 single-unit checks pass. The
/// three candidates were the second sampler's TEX_ID, its reflection row, and
/// the descriptor base. This probe answers the first two, and refutes both:
///
///     instr 0: texld TEX_ID=0 TEX_SWIZ=228 (0xE4, identity)
///     instr 2: texld TEX_ID=1 TEX_SWIZ=228
///     temps=5 ps_color_out_reg=2 num_in=1
///     uniform rows: 2 -- dword_off=65535 (NO_BLOCK) binding=0 gl_type=0x8b5e
///                       dword_off=65535              binding=1 gl_type=0x8b5e
///
/// Two look-ups, distinct and correct sampler ids, both sampler rows present
/// with the right bindings and in order. Nothing texture-shaped is wrong.
///
/// `#[ignore]`d because it asserts nothing: it prints. A probe that is also a
/// gate would have to decide what "right" is, and the point of this one was to
/// find out. Run it by name when the next texture question comes up.
#[cfg(all(test, feature = "gpu-vivante"))]
mod tex_probe {
    use super::*;
    use crate::shaderc::online::vivante::decode::decode_instr;
    use crate::shaderc::SHC_STAGE_FRAGMENT;

    #[test]
    #[ignore = "a probe, run by name"]
    fn what_tex_id_does_each_lookup_carry() {
        let src = "#version 100\nprecision mediump float;\nvarying vec2 v_uv;\n\
                   uniform sampler2D u_tex0;\nuniform sampler2D u_tex1;\n\
                   void main() { gl_FragColor = vec4(texture2D(u_tex0, v_uv).rgb * 0.5 \
                   + texture2D(u_tex1, v_uv).rgb * 0.25, 1.0); }";
        let _serial = super::super::store::test_keep();
        let sh = super::super::store::with_scratch(|s| {
            compile(SHC_STAGE_FRAGMENT, src.as_bytes(), 0, s, super::super::store::keep())
        })
        .expect("scratch")
        .expect("compiles");

        // SAFETY: the compiler just wrote these into the keep arena.
        let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        let (whole, _) = dw.as_chunks::<4>();
        for (i, w) in whole.iter().enumerate() {
            let ins = decode_instr(w).expect("decodes");
            if ins.op().meta == "tex" {
                std::eprintln!(
                    "  instr {i}: {} TEX_ID={:?} TEX_SWIZ={:?}",
                    ins.op().name,
                    ins.get("TEX_ID"),
                    ins.get("TEX_SWIZ")
                );
            }
        }
        std::eprintln!("  temps={} ps_color_out_reg={} num_in={}", sh.temps, sh.viv.ps_color_out_reg, sh.viv.num_in);
        std::eprintln!("  uniform rows: {}", sh.num_uniforms);
        // SAFETY: the compiler wrote num_uniforms rows into the keep arena.
        let us = unsafe { core::slice::from_raw_parts(sh.uniforms, sh.num_uniforms as usize) };
        for u in us {
            std::eprintln!(
                "    dword_off={} dwords={} binding={} gl_type={:#x}",
                u.dword_off,
                u.dwords,
                u.binding,
                u.gl_type
            );
        }
    }
}
