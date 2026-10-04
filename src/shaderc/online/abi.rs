// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2012-2019 Etnaviv Project
// SPDX-FileCopyrightText: 2019 Zodiac Inflight Innovations
// Ported in part from Mesa 26.2.1 src/gallium/drivers/etnaviv/
// etnaviv_compiler_nir.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The `ShcShader` assembly for the GC7000: what the code generator's output
//! has to become before the driver can program the hardware with it.
//!
//! `docs/online-shaderc/gc7000-abi.md` is the contract and every rule here
//! cites it. Its opening is the reason this file is careful: *"A shader that
//! gets the encodings right and this wrong produces a clean draw and a wrong
//! picture."* Nothing here can be checked by looking at a picture, so
//! everything this file can refuse, it refuses.
//!
//! What makes this GPU's assembly short: there are no instruction groups, no
//! phases and no padding, so a shader is a flat instruction list; and the
//! pixel engine blends, so a fragment shader is compiled once and has **no
//! blend variants** — the single biggest simplification against the PowerVR.
//!
//! What makes it different: the compiler does not report a uniform block size.
//! It hands the driver a **per-dword description of the constant register
//! file** ([`ConstSlot`]), because some of those dwords are not uniforms at
//! all but properties of a bound texture that the driver resolves at draw
//! time. The compiler owns that layout.

use super::store;
use super::vivante::decode::decode_instr;
use crate::shaderc::{
    ShcShader, ShcUniform, ShcViv, ShcVivConst, ShcVivIo, SHC_NO_BLEND_CONSTS, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,
    SHC_VIV_CONST_LITERAL, SHC_VIV_CONST_UNIFORM, SHC_VIV_CONST_UNUSED, SHC_VIV_MAX_IO, SHC_VIV_NO_REG,
};
use crate::util::set_name;

/// One dword of the constant register file: what it holds, and the datum that
/// resolves it (`gc7000-abi.md`, "Uniforms: the constant register file").
///
/// `contents` is a [`ShcVivConst`]: `LITERAL` carries the value itself,
/// `UNIFORM` a dword index into the stage's uniform block, and the texture
/// codes a sampler binding whose bound texture the driver reads at draw time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ConstSlot {
    pub contents: ShcVivConst,
    pub data: u32,
}

impl ConstSlot {
    /// A dword nothing reads; the driver resolves it to zero.
    pub const UNUSED: ConstSlot = ConstSlot { contents: SHC_VIV_CONST_UNUSED, data: 0 };

    /// A constant the compiler folded in, as raw bits.
    pub fn literal(bits: u32) -> ConstSlot {
        ConstSlot { contents: SHC_VIV_CONST_LITERAL, data: bits }
    }

    /// Dword `block_dword` of the stage's uniform block.
    pub fn uniform(block_dword: u32) -> ConstSlot {
        ConstSlot { contents: SHC_VIV_CONST_UNIFORM, data: block_dword }
    }
}

/// One shader input or output register, before it becomes a [`ShcVivIo`].
#[derive(Clone, Copy)]
pub struct IoReg<'a> {
    /// The GC7000 register number.
    pub reg: u8,
    /// The compiler's `gl_varying_slot` / `gl_vert_attrib`.
    pub slot: u8,
    /// 1..4.
    pub components: u8,
    /// Fragment inputs: 0 smooth, 1 noperspective, 2 flat.
    pub interp: u8,
    /// The GLSL name. `link()` pairs the stages on this, so it is not
    /// decoration — see [`AbiError::NameTooLong`].
    pub name: &'a str,
    /// The GL enum (`GL_FLOAT_VEC4`, ...). The front end's, carried through
    /// for [`ShcShader::attribs`], which `glGetActiveAttrib` reports from.
    pub gl_type: u16,
}

/// Everything the code generator produces for one stage, before it is a
/// `ShcShader`.
pub struct VivProgram<'a> {
    /// [`SHC_STAGE_VERTEX`] or [`SHC_STAGE_FRAGMENT`].
    pub stage: u32,
    /// The instruction words: four dwords per 128-bit instruction.
    pub code: &'a [u32],
    /// Temporaries used, **honestly counted**. See [`AbiError::TempsUnset`].
    pub temps: u16,
    /// VS attributes, or FS varyings. A fragment shader's must be registers
    /// `1..=N` in order; see [`AbiError::VaryingRegisters`].
    pub io_in: &'a [IoReg<'a>],
    /// VS varyings and position, or the FS colour.
    pub io_out: &'a [IoReg<'a>],
    /// The constant register file, one entry per dword, in register order.
    pub consts: &'a [ConstSlot],
    /// Instruction indices of branches that are a loop's BACK EDGE, and so
    /// may point backwards. Everything else must go forward -- see
    /// [`every_branch_goes_forward`].
    ///
    /// Selection knows which branch closes a loop and the words do not, so
    /// the permission has to travel with them. An empty slice means "this
    /// program has no loop", which is the common case and the strict one.
    pub back_edges: &'a [usize],
    pub vs_pos_out_reg: Option<u8>,
    pub vs_pointsize_out_reg: Option<u8>,
    pub vs_id_in_reg: Option<u8>,
    pub ps_color_out_reg: Option<u8>,
    pub ps_depth_out_reg: Option<u8>,
}

/// `input_count_unk8`, computed the way Mesa computes it.
///
/// The name still says `unk8` because what the field *means* is unestablished;
/// what it *contains* is not, and those are different questions.
///
/// The fragment value is a constant in etnaviv too -- it computes the vertex
/// formula for every stage and then overwrites it for fragment
/// (`etnaviv_compiler_nir.c:1389`, "XXX what is this"):
///
/// ```text
/// v->input_count_unk8 = DIV_ROUND_UP(v->infile.num_reg + 4, 16);  /* :1157 */
/// if (stage == MESA_SHADER_FRAGMENT) v->input_count_unk8 = 31;    /* :1389 */
/// ```
///
/// This used to be a per-stage constant here, 1 for vertex, "measured over the
/// whole offline table, no exceptions". The measurement was true and the
/// conclusion was not: **the corpus cannot tell the two rules apart.** Its 179
/// vertex blobs run from 1 to 10 inputs, and `DIV_ROUND_UP(n + 4, 16)` is 1 for
/// every `n <= 12`. The first input count at which the formula says anything
/// but 1 is 13, which no blob in the table reaches.
///
/// It is reachable in the product: `MAX_VERTEX_ATTRIBS` is 16
/// (`src/gl/context.rs:77`), the chip's own `stream_count`, so a conformant GL
/// ES 2.0 shader may declare 13..16 attributes. Every one of those got 1 from
/// the constant and 2 from the hardware's vendor driver.
fn input_count_unk8(stage: u32, num_in: usize) -> u16 {
    if stage == SHC_STAGE_FRAGMENT {
        31
    } else {
        /* DIV_ROUND_UP(num_in + 4, 16). num_in is bounded by SHC_VIV_MAX_IO,
         * checked in io_list() before this runs, so the arithmetic cannot
         * overflow a u16. */
        ((num_in + 4).div_ceil(16)) as u16
    }
}

/// `vs_load_balancing`, computed the way Mesa computes it.
///
/// This field steers how the hardware splits work between the vertex and
/// fragment stages, and it is **a function of the vertex output count and the
/// chip's own specs — not of the shader's register use.** Transcribed from
/// `etnaviv_compiler_nir.c:1178-1189` (Mesa, MIT; read and transcribed, never
/// vendored):
///
/// ```text
/// half_out = num_out / 2 + 1
/// b        = (20480 / (vertex_output_buffer_size - 2*half_out*vertex_cache_size) + 9) / 10
/// a        = (b + 256 / (shader_core_count * half_out)) / 2
/// word     = A(min(a,255)) | B(min(b,255)) | C(0x3f) | D(0x0f)
/// ```
///
/// with integer division throughout, and `num_out` counting varyings only —
/// the `+1` in `half_out` is what accounts for the position output.
///
/// **Verified by reproducing every measured band.** Computed against this
/// part's specs (`driver/rust/src/gpu/vivante/hw.rs` `DB`, galcore's
/// `gc7000UL_6200` entry: vertex output buffer 1024, vertex cache 16, two
/// shader cores) it gives, for `num_out` 0..9, exactly the five values the
/// 169 shipped vertex blobs carry — see `the_formula_reproduces_every_shipped_band`.
///
/// # What this replaced, and why the replacement matters
///
/// This was a table of five measured values plus a refusal for anything
/// outside them, and — because the bands correlated with register use across
/// the shipped blobs — a second refusal for a shader whose `temps` exceeded
/// the highest any blob in its band carried. That second refusal was reasoned
/// backwards from a hung GPU, and **the formula refutes it: `temps` appears
/// nowhere in it.** The correlation was an artefact of which shaders happened
/// to be in the table.
///
/// The hardware limit on registers is real and is elsewhere: `NUM_TEMPS` is
/// six bits (`state_3d.xml.h:201` and `:644`) and this part has 64 registers,
/// so more than 63 is refused by the allocator and again at the driver seam.
/// That refusal stands on the field width, which is measured, rather than on
/// a ceiling inferred from a sample.
///
/// The formula also closes the gap the table had at `num_out` 8 — a count GL
/// ES 2.0 requires us to accept — and at everything above 9.
fn vs_load_balancing(stage: u32, num_out: u8, temps: u16) -> Result<u32, AbiError> {
    if stage == SHC_STAGE_FRAGMENT {
        return Ok(0);
    }
    /* THE CEILING IS BACK, and the formula did not entitle me to remove it.
     *
     * I took `temps` not appearing in Mesa's expression as proof that the
     * ceiling was a correlation, and predicted that a shader at temps 44
     * would draw. cts-runner ran it: temps 44 WEDGES, and so does the
     * REFERENCE compiler's temps-40 build of the same shader. Neither crosses
     * the six-bit field, so the truncation is not what hangs them, and the
     * ceiling fits all six arms with no exception -- above it wedges, below
     * it draws, on both compilers.
     *
     * What I missed is that "Mesa computes the word this way" and "the word
     * is safe at this register pressure" are different claims. Mesa's own
     * comment calls it `XXX this is a conservative estimate`, and Mesa's
     * allocator never hands it a shader like ours. The formula is right about
     * the VALUE and says nothing about the RANGE over which that value holds.
     *
     * The bound below is the highest `temps` any shipped blob in each band
     * carries. It is not the hardware's limit: cts-runner's arms put the real
     * one somewhere in 11..39, so this is measured-safe rather than known-safe
     * and a bisect will replace it with a number rather than a sample. */
    /* Band 2|3 WAS 42 and an arm falsified it. `n2_k22`, 27 temporaries,
     * WEDGED on the board at ccb2e81 while this ceiling stood at 42 -- so 42
     * was never measured-safe, it was the largest count that happened to have
     * shipped. Those are different claims and the first is what this guard is
     * for.
     *
     * 26 is the highest count in that band MEASURED TO DRAW (`k = 21`), with
     * `k = 22..24` wedging at 27..29 on file. The criterion therefore changes
     * with the number: the bound is the highest temps a band has been SEEN TO
     * DRAW, not the highest a shipped blob happens to carry.
     *
     * The other bands still rest on the falsified criterion, because no arm
     * has probed them. They are not thereby safe, they are unprobed, and
     * 0|1's 21 and the three 16s may fall the same way. The next ladder should
     * take a band whose ceiling stands above anything it has been seen to
     * draw. */
    let temps_max: u16 = match num_out {
        0 | 1 => 21,
        2 | 3 => 26,
        4 | 5 => 16,
        6 | 7 => 16,
        9 => 18,
        /* No blob measured this band, so there is no ceiling for it. Take the
         * most conservative one that was measured rather than let a band the
         * formula newly enables through unguarded. */
        _ => 16,
    };
    if temps > temps_max {
        return Err(AbiError::LoadBalancingTempsUnmeasured { num_out, temps, temps_max });
    }
    /* This part's specs, from the hwdb row named above. They are constants
     * because this compiler targets this part; when a second Vivante part
     * arrives they come from the identity the driver already reads. */
    const VERTEX_OUTPUT_BUFFER_SIZE: u32 = 1024;
    const VERTEX_CACHE_SIZE: u32 = 16;
    const SHADER_CORE_COUNT: u32 = 2;

    let half_out = num_out as u32 / 2 + 1;
    /* The denominator goes non-positive once the outputs would need more
     * buffer than the chip has. Unreachable for GL ES 2.0 (eight varyings
     * against a limit near sixty), but a division by zero is not a thing to
     * leave to reasoning. */
    let Some(denom) = VERTEX_OUTPUT_BUFFER_SIZE.checked_sub(2 * half_out * VERTEX_CACHE_SIZE) else {
        return Err(AbiError::LoadBalancingUnmeasured { num_out });
    };
    if denom == 0 {
        return Err(AbiError::LoadBalancingUnmeasured { num_out });
    }
    /* Mesa writes this as `(20480 / denom + 9) / 10`, which is the same
     * value; spelled as div_ceil here because that is what it means and
     * because clippy is right that the idiom is the clearer one. */
    let b = (20480 / denom).div_ceil(10);
    let a = (b + 256 / (SHADER_CORE_COUNT * half_out)) / 2;
    Ok(a.min(255) | (b.min(255) << 8) | (0x3f << 16) | (0x0f << 24))
}

/// Why a program could not be assembled into a `ShcShader`.
///
/// Every one of these would otherwise reach the hardware as a clean draw with
/// a wrong picture, which is the failure `gc7000-abi.md` opens by naming.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AbiError {
    /// A vertex shader using more temporaries than any shipped blob that
    /// established this band's `vs_load_balancing` value.
    ///
    /// **Refused because the symptom is a hung GPU, measured on both
    /// compilers.** cts-runner's discriminating run: the same shader at temps
    /// 44 from our back end and temps 40 from the reference back end both
    /// wedge the GC7000; at temps 2, 5 and 10 both draw. Six arms, no
    /// exception, and neither hanging count crosses the six-bit `NUM_TEMPS`
    /// field -- so this is not the truncation, which is a separate defect
    /// refused separately.
    ///
    /// The bound is the highest `temps` a shipped blob in the band carries,
    /// which is measured-safe rather than known-safe: the real limit is
    /// somewhere in 11..39 and a bisect will replace this with a number.
    LoadBalancingTempsUnmeasured { num_out: u8, temps: u16, temps_max: u16 },
    /// A vertex shader emits a number of outputs for which no shipped blob
    /// records a `vs_load_balancing` value, so there is nothing to copy and
    /// the formula is not established. Refused rather than extrapolated: the
    /// field steers how the hardware splits work between the stages, and a
    /// value invented for it fails as a stall or a slow frame rather than as
    /// an error, which is the hardest kind of wrong to find.
    LoadBalancingUnmeasured { num_out: u8 },
    /// A branch whose target is at or before the branch itself, from a
    /// program with no loop in it.
    ///
    /// **This is the guarantee that we never emit a non-terminating shader**,
    /// and it is structural on purpose. A GPU that will not finish a shader
    /// hangs until a watchdog resets it; the Zephyr image resets by
    /// construction, but a reset costs a frame and a visible glitch, which on
    /// a cluster is a safety-relevant event. So the compiler must not be
    /// *able* to emit one, rather than merely not doing so today.
    ///
    /// Only a loop's back edge may point backwards, and this back end has no
    /// loop node yet, so any backward target is a defect -- a mis-resolved
    /// forward branch or a target remapped wrongly after an instruction was
    /// deleted. shaderc-remote's corpus work makes it diagnostic as well as
    /// illegal: targets are absolute in both stages, and only 4 of the 303
    /// branches in the shipped table point backwards at all.
    BranchTargetBackwards { at: usize, target: u16 },
    /// A branch past the end of the program.
    ///
    /// The range is `[0, ninstr]` inclusive -- one past the last instruction
    /// is "the end", which 17 shipped branches rely on -- so anything beyond
    /// that runs whatever follows the shader in the instruction cache.
    BranchTargetPastEnd { at: usize, target: u16, ninstr: usize },
    /// The code is not a whole number of 128-bit instructions. The driver
    /// takes the instruction count as `usc_dw / 4` (`src/vivante/shader.rs`,
    /// :157), so a ragged tail is silently dropped or run as garbage.
    CodeNotWholeInstructions { dwords: usize },
    /// No instructions at all: `ICACHE_COUNT` is `count - 1`, which underflows.
    CodeEmpty,
    /// More inputs or outputs than [`SHC_VIV_MAX_IO`].
    TooManyIo { got: usize, max: usize },
    /// A fragment shader's varyings are not registers `1..=N` in order.
    ///
    /// `gc7000-abi.md`: the fragment shader reads its varyings from registers
    /// 1..N and `num_in` **is** the varying count. The driver builds the
    /// `VS_OUTPUT` map from `fs.vs_reg_of_in[i]` in that order, so a gap, or a
    /// varying at register 0, shifts every one of them in the stream.
    VaryingRegisters { index: usize, reg: u8, expected: u8 },
    /// A vertex shader with no `gl_Position`. `VS_OUTPUT` is built with
    /// position first; without one the map is meaningless.
    NoPositionOutput,
    /// A name the 32-byte field would cut. `link()` pairs the stages by name,
    /// and two names identical in their first 31 bytes would pair the wrong
    /// ones and render something plausible. `shaderc::link` already refuses
    /// such a pair; refusing to *store* one applies the same rule earlier,
    /// while the full name still exists.
    NameTooLong { len: usize, max: usize },
    /// `temps` is 0 while the program has inputs. The driver places unread
    /// vertex elements in temporaries **above** the shader's count
    /// (`shader.rs:51-61`), so an under-count aliases a live temporary with an
    /// attribute nobody reads.
    TempsUnset,
    /// `temps` does not cover an io register the program uses.
    ///
    /// On this machine **vertex attributes and shader outputs live in the same
    /// register file as the temporaries** — `driver/src/vivante/gc_shaders.h`
    /// has the bring-up triangle at `aPos` r0, `aColor` r1, `VS_POS_OUT_REG`
    /// 0, `VS_VARYING0_REG` 1, `NUM_TEMPS` 2. So an io register is not a
    /// separate space the count may ignore: it is a reservation *inside* the
    /// file, and `temps` has to be above the highest one even when no
    /// instruction writes a temporary at all.
    ///
    /// Getting this wrong is invisible here and expensive there: the driver
    /// puts unread vertex elements above `temps`, so a count that excluded the
    /// io registers would have it overwrite a live attribute.
    TempsBelowIo { temps: u16, reg: u8, what: &'static str },
    /// The `keep` arena had no room; `left` bytes remained.
    OutOfMemory { left: usize },
}

/// The longest name a [`ShcVivIo`] stores, not counting the NUL.
const NAME_MAX: usize = 31;

fn store_name(field: &mut [core::ffi::c_char; 32], name: &str) -> Result<(), AbiError> {
    if name.len() > NAME_MAX {
        return Err(AbiError::NameTooLong { len: name.len(), max: NAME_MAX });
    }
    set_name(field, name.as_bytes());
    Ok(())
}

/// What `ShcVivIo::slot` counts from for a vertex attribute.
///
/// The field holds the compiler's `gl_vert_attrib`, not the GL location, and
/// the offline table numbers attributes from 15: measured, the vertex input
/// slot is the attribute's location plus 15 in 559 of 588 attributes. The 29
/// that are not are the zero-component duplicate rows of the known table
/// defect (one source names an attribute twice), which carry location 0 and
/// slot 0 and are not attributes at all.
const VERT_ATTRIB_BASE: u8 = 15;

/// What `ShcVivIo::slot` counts from for a varying: Mesa's `VARYING_SLOT_VAR0`.
///
/// Measured on both sides of the interface, which is the part that matters:
/// vertex output slots run 32..40 and fragment input slots 32..38, both
/// contiguous from 32, so the two stages agree on the numbering.
///
/// [`crate::shaderc::link::by_register`] pairs varyings by NAME and falls back
/// to this only when a name is missing. Our compiler always emits names, so
/// nothing we link today depends on it -- but a program that pairs one of our
/// shaders with one from the offline table would fall back into a comparison
/// between two different numberings, and silently match nothing.
const VARYING_SLOT_VAR0: u8 = 32;

/// What a fragment shader's colour output row's `slot` counts from: Mesa's
/// `FRAG_RESULT_DATA0` (after DEPTH, STENCIL, COLOR and SAMPLE_MASK), so a
/// draw buffer n is slot 4 + n. Only a shader writing more than the first
/// colour has output rows at all (`select`).
const FRAG_RESULT_DATA0: u8 = 4;

fn io_list(src: &[IoReg<'_>], base: u8) -> Result<([ShcVivIo; SHC_VIV_MAX_IO], u8), AbiError> {
    if src.len() > SHC_VIV_MAX_IO {
        return Err(AbiError::TooManyIo { got: src.len(), max: SHC_VIV_MAX_IO });
    }
    let mut out = [ShcVivIo::ZERO; SHC_VIV_MAX_IO];
    for (dst, s) in out.iter_mut().zip(src) {
        dst.reg = s.reg;
        /* `IoReg::slot` is the GL location; `ShcVivIo::slot` is the compiler's
         * numbering, which counts from a different base per kind. Converted
         * here rather than in selection so that `ShcAttrib::location` keeps
         * the GL location `glGetAttribLocation` must return. */
        dst.slot = base.saturating_add(s.slot);
        dst.components = s.components;
        dst.interp = s.interp;
        store_name(&mut dst.name, s.name)?;
    }
    Ok((out, src.len() as u8))
}

/// Assemble one stage into a `ShcShader`, with everything it points at copied
/// into the `keep` arena.
///
/// `uniforms` is the front end's reflection, unchanged: this file does not
/// invent, reorder or filter rows (`ir.md` — row order is load-bearing,
/// because `glGetActiveUniform` indices follow it).
/// Every branch goes forward, and none goes past the end.
///
/// Checked on the FINAL words rather than on the selector's instruction list,
/// because the list is not what the hardware runs: a target is an instruction
/// index, and every pass between selection and here that deletes or reorders
/// an instruction has to move the targets with it. Validating the output is
/// the only way to catch a pass that forgets -- and one already nearly did
/// (`select.rs`'s copy propagation deletes instructions and remaps targets
/// through a survivor map).
///
/// A loop's back edge is the one branch that may point backwards, and it says
/// so by index in `back_edges` -- selection knows which branch closes a loop
/// and the words do not. A program with no loop passes an empty slice, so the
/// permission is granted per branch rather than per program: a second,
/// unintended backward branch in a shader that does have a loop is still
/// refused.
fn every_branch_goes_forward(code: &[u32], back_edges: &[usize]) -> Result<(), AbiError> {
    let ninstr = code.len() / 4;
    for (i, q) in code.as_chunks::<4>().0.iter().enumerate() {
        let Ok(ins) = decode_instr(q) else { continue };
        let Some(t) = ins.get("TARGET") else { continue };
        let t = t as u16;
        if usize::from(t) > ninstr {
            return Err(AbiError::BranchTargetPastEnd { at: i, target: t, ninstr });
        }
        if usize::from(t) <= i && !back_edges.contains(&i) {
            return Err(AbiError::BranchTargetBackwards { at: i, target: t });
        }
    }
    Ok(())
}

pub fn assemble(p: &VivProgram<'_>, uniforms: &[ShcUniform], uniform_dwords: u16) -> Result<ShcShader, AbiError> {
    if p.code.is_empty() {
        return Err(AbiError::CodeEmpty);
    }
    if !p.code.len().is_multiple_of(4) {
        return Err(AbiError::CodeNotWholeInstructions { dwords: p.code.len() });
    }
    if p.temps == 0 && !p.io_in.is_empty() {
        return Err(AbiError::TempsUnset);
    }
    if p.stage == SHC_STAGE_VERTEX && p.vs_pos_out_reg.is_none() {
        return Err(AbiError::NoPositionOutput);
    }
    every_branch_goes_forward(p.code, p.back_edges)?;
    if p.stage == SHC_STAGE_FRAGMENT {
        for (i, io) in p.io_in.iter().enumerate() {
            let expected = (i + 1) as u8;
            if io.reg != expected {
                return Err(AbiError::VaryingRegisters { index: i, reg: io.reg, expected });
            }
        }
    }
    /* Inputs, outputs and temporaries share one register file, so `temps` has
     * to be above every io register the program uses. See
     * `AbiError::TempsBelowIo`.
     *
     * After the varying-order check on purpose: a fragment shader with a
     * varying at the wrong register usually fails both, and "your varyings are
     * not 1..=N" is the cause while "temps does not cover r3" is the
     * consequence. Reporting the consequence would send a reader to the
     * allocator instead of to the selection. */
    for (regs, what) in [(p.io_in, "an input"), (p.io_out, "an output")] {
        for io in regs {
            if u16::from(io.reg) >= p.temps {
                return Err(AbiError::TempsBelowIo { temps: p.temps, reg: io.reg, what });
            }
        }
    }
    for (reg, what) in [
        (p.vs_pos_out_reg, "gl_Position"),
        (p.vs_pointsize_out_reg, "gl_PointSize"),
        (p.ps_color_out_reg, "the fragment colour"),
        (p.ps_depth_out_reg, "gl_FragDepth"),
        (p.vs_id_in_reg, "gl_VertexID/gl_InstanceID"),
    ] {
        if let Some(r) = reg {
            if u16::from(r) >= p.temps {
                return Err(AbiError::TempsBelowIo { temps: p.temps, reg: r, what });
            }
        }
    }

    /* A vertex shader's inputs are attributes and everything else in these
     * two lists is a varying, so the numbering differs by which side of the
     * vertex stage the list is on. */
    let in_base = if p.stage == SHC_STAGE_VERTEX { VERT_ATTRIB_BASE } else { VARYING_SLOT_VAR0 };
    let (in_, num_in) = io_list(p.io_in, in_base)?;
    let out_base = if p.stage == SHC_STAGE_VERTEX { VARYING_SLOT_VAR0 } else { FRAG_RESULT_DATA0 };
    let (out, num_out) = io_list(p.io_out, out_base)?;

    let keep = store::keep();
    let oom = |_| AbiError::OutOfMemory { left: store::keep().left() };
    let code: &'static [u32] = keep.alloc_copy(p.code).map_err(oom)?;
    let uni: &'static [ShcUniform] = keep.alloc_copy(uniforms).map_err(oom)?;

    /* The constant file as two parallel arrays in register order. They are
     * separate because `ShcViv` points at them separately -- a u8 per dword
     * and a u32 per dword -- which is the C ABI's shape, not a choice here. */
    let contents = keep.alloc_filled(p.consts.len(), 0u8).map_err(oom)?;
    let data = keep.alloc_filled(p.consts.len(), 0u32).map_err(oom)?;
    for (i, slot) in p.consts.iter().enumerate() {
        contents[i] = slot.contents;
        data[i] = slot.data;
    }

    let viv = ShcViv {
        in_,
        num_in,
        out,
        num_out,
        /* link() fills this, not the compiler (gc7000-abi.md, the last line of
         * "What an online compiler must produce"). All-NO_REG means an
         * unlinked shader cannot be mistaken for one whose varyings happen to
         * come from register 0. */
        vs_reg_of_in: [SHC_VIV_NO_REG; SHC_VIV_MAX_IO],
        vs_pos_out_reg: p.vs_pos_out_reg.unwrap_or(SHC_VIV_NO_REG),
        vs_pointsize_out_reg: p.vs_pointsize_out_reg.unwrap_or(SHC_VIV_NO_REG),
        vs_id_in_reg: p.vs_id_in_reg.unwrap_or(SHC_VIV_NO_REG),
        ps_color_out_reg: p.ps_color_out_reg.unwrap_or(SHC_VIV_NO_REG),
        ps_depth_out_reg: p.ps_depth_out_reg.unwrap_or(SHC_VIV_NO_REG),
        vs_load_balancing: vs_load_balancing(p.stage, num_out, p.temps)?,
        input_count_unk8: input_count_unk8(p.stage, num_in as usize),
        const_contents: if p.consts.is_empty() { core::ptr::null() } else { contents.as_ptr() },
        const_data: if p.consts.is_empty() { core::ptr::null() } else { data.as_ptr() },
        const_count: p.consts.len() as u16,
    };

    let mut sh = ShcShader::ZERO;
    /* The vertex attributes, which `glGetActiveAttrib`, `glGetAttribLocation`
     * and `GL_ACTIVE_ATTRIBUTES` are answered from, and which the driver reads
     * to route each bound array into the register the shader expects.
     *
     * This was missing, and the way it failed is worth keeping: on the board
     * the program compiled and linked -- the first time our compiler's output
     * did -- and then reported `GL_ACTIVE_ATTRIBUTES = 0` with no GL error at
     * all, so `glGetAttribLocation("a_pos")` returned -1 and the application
     * had nothing to bind. A shader with no attributes is a legal shader, so
     * nothing downstream could tell the difference between "this program has
     * none" and "nobody filled them in".
     *
     * A fragment shader's `io_in` are varyings, not attributes, so only the
     * vertex stage fills this. `num_varyings` stays 0: on the GC7000 the
     * inter-stage wiring is `ShcViv::in_`/`out`, and all 179 vertex blobs in
     * the offline table carry `num_varyings = 0`. */
    if p.stage == SHC_STAGE_VERTEX {
        if p.io_in.len() > sh.attribs.len() {
            return Err(AbiError::TooManyIo { got: p.io_in.len(), max: sh.attribs.len() });
        }
        for (dst, io) in sh.attribs.iter_mut().zip(p.io_in) {
            dst.location = io.slot;
            dst.components = io.components;
            /* The register the code generator actually made the shader read,
             * so the two cannot disagree. The shipped compiler compacts over
             * an unused location -- measured, 21 of 588 attributes have
             * `vi_reg < location` and every one of those shaders skips a
             * location -- which we do not do yet. Ours stays self-consistent
             * either way, because the driver is told the same register the
             * code reads. */
            dst.vi_reg = io.reg;
            /* 0 in all 588 attributes of the offline table. */
            dst.format = 0;
            dst.gl_type = io.gl_type;
            store_name(&mut dst.name, io.name)?;
        }
        sh.num_attribs = p.io_in.len() as u8;
    }
    sh.usc = code.as_ptr();
    sh.usc_dw = code.len() as u32;
    sh.temps = p.temps;
    sh.uniforms = if uni.is_empty() { core::ptr::null() } else { uni.as_ptr() };
    sh.num_uniforms = uni.len() as u8;
    sh.uniform_dwords = uniform_dwords;
    /* Zero on this GPU, both stages. Measured: all 307 blobs in the offline
     * table carry 0, fragment and vertex alike. This used to be 1 for a
     * fragment shader, which is the PowerVR reading of the field -- there
     * `num_outputs` counts what the USC emits -- and on the GC7000 the colour
     * is named by `ps_color_out_reg` instead, so nothing is lost by the 0. */
    sh.num_outputs = 0;
    /* The pixel engine blends on this part, so a fragment shader is compiled
     * once: no blend variant, and no blend-constant window. */
    sh.blend_key = 0;
    sh.blend_consts_off = SHC_NO_BLEND_CONSTS;
    sh.viv = viv;
    Ok(sh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(name: &str, reg: u8) -> IoReg<'_> {
        IoReg { reg, slot: 0, components: 4, interp: 0, name, gl_type: crate::gl::consts::GL_FLOAT_VEC4 as u16 }
    }

    /// The error of an assembly that must fail. `ShcShader` is the C ABI's
    /// `#[repr(C)]` struct with raw pointers in it and implements neither
    /// `Debug` nor `PartialEq`, so a failing case compares the error and
    /// asserts separately that there was one.
    fn err(p: &VivProgram<'_>) -> AbiError {
        match assemble(p, &[], 0) {
            Ok(_) => panic!("this program must not assemble"),
            Err(e) => e,
        }
    }

    /// The passthrough vertex shader's *shape*: the real words are
    /// shaderc-remote's builder's to produce, and this asserts about the
    /// assembly, not about any instruction.
    fn vs<'a>(code: &'a [u32], io_in: &'a [IoReg<'a>], io_out: &'a [IoReg<'a>]) -> VivProgram<'a> {
        VivProgram {
            stage: SHC_STAGE_VERTEX,
            code,
            temps: 1,
            io_in,
            io_out,
            consts: &[],
            back_edges: &[],
            vs_pos_out_reg: Some(0),
            vs_pointsize_out_reg: None,
            vs_id_in_reg: None,
            ps_color_out_reg: None,
            ps_depth_out_reg: None,
        }
    }

    fn fs<'a>(code: &'a [u32], io_in: &'a [IoReg<'a>], consts: &'a [ConstSlot]) -> VivProgram<'a> {
        VivProgram {
            stage: SHC_STAGE_FRAGMENT,
            code,
            /* Above every io register these fixtures use: inputs, outputs and
             * temporaries share one file, so a fixture with a varying at r2
             * and temps = 1 is a program the hardware could not run. The
             * invariant caught four of these when it was added, which is the
             * fixtures having been wrong rather than the check being strict. */
            temps: (io_in.len() + 1) as u16,
            io_in,
            io_out: &[],
            consts,
            back_edges: &[],
            vs_pos_out_reg: None,
            vs_pointsize_out_reg: None,
            vs_id_in_reg: None,
            ps_color_out_reg: Some(0),
            ps_depth_out_reg: None,
        }
    }

    #[test]
    fn a_passthrough_vertex_shader_assembles() {
        let _s = store::test_keep();
        let code = [0u32; 8]; /* two 128-bit instructions */
        let ins = [io("a_pos", 0)];
        let outs = [io("gl_Position", 0)];
        let sh = assemble(&vs(&code, &ins, &outs), &[], 0).expect("assembles");
        assert_eq!(sh.usc_dw, 8);
        assert_eq!(sh.usc_words().len(), 8, "the code is in `keep` and readable through the shader");
        assert_eq!(sh.viv.num_in, 1);
        assert_eq!(sh.viv.vs_pos_out_reg, 0);
        assert_eq!(sh.viv.vs_pointsize_out_reg, SHC_VIV_NO_REG, "absent reads as absent, not as register 0");
    }

    #[test]
    fn code_that_is_not_whole_instructions_is_refused() {
        /* The driver takes the instruction count as usc_dw / 4. A ragged tail
         * is either dropped or run as garbage, and neither shows up as
         * anything but a wrong picture. */
        let _s = store::test_keep();
        let ins = [io("a_pos", 0)];
        let outs = [io("gl_Position", 0)];
        assert_eq!(err(&vs(&[0u32; 6], &ins, &outs)), AbiError::CodeNotWholeInstructions { dwords: 6 });
    }

    #[test]
    fn an_empty_shader_is_refused_because_icache_count_would_underflow() {
        let _s = store::test_keep();
        let ins = [io("a_pos", 0)];
        let outs = [io("gl_Position", 0)];
        assert_eq!(err(&vs(&[], &ins, &outs)), AbiError::CodeEmpty);
    }

    #[test]
    fn a_vertex_shader_without_a_position_is_refused() {
        let _s = store::test_keep();
        let code = [0u32; 4];
        let ins = [io("a_pos", 0)];
        let mut p = vs(&code, &ins, &[]);
        p.vs_pos_out_reg = None;
        assert_eq!(err(&p), AbiError::NoPositionOutput);
    }

    #[test]
    fn fragment_varyings_must_be_registers_one_upward_in_order() {
        /* gc7000-abi.md: the FS reads varyings from registers 1..N and num_in
         * IS the varying count; the driver builds VS_OUTPUT from
         * fs.vs_reg_of_in[i] in that order. A varying at register 0, or a gap,
         * shifts every one of them in the stream -- geometry that draws, and
         * colours that belong to a different varying. */
        let _s = store::test_keep();
        let code = [0u32; 4];

        let good = [io("v_uv", 1), io("v_col", 2)];
        assert!(assemble(&fs(&code, &good, &[]), &[], 0).is_ok(), "1,2 is the layout the driver expects");

        let from_zero = [io("v_uv", 0)];
        assert_eq!(err(&fs(&code, &from_zero, &[])), AbiError::VaryingRegisters { index: 0, reg: 0, expected: 1 });

        let with_gap = [io("v_uv", 1), io("v_col", 3)];
        assert_eq!(err(&fs(&code, &with_gap, &[])), AbiError::VaryingRegisters { index: 1, reg: 3, expected: 2 });
    }

    #[test]
    fn a_name_the_field_would_cut_is_refused_rather_than_stored() {
        /* link() pairs the stages by name. Two names identical in their first
         * 31 bytes pair the wrong ones and draw something plausible;
         * shaderc::link already refuses such a pair, and refusing to store one
         * applies the same rule while the full name still exists. */
        let _s = store::test_keep();
        let code = [0u32; 4];
        let long = "v_this_name_is_thirty_two_chars_";
        assert_eq!(long.len(), 32);
        assert_eq!(err(&fs(&code, &[io(long, 1)], &[])), AbiError::NameTooLong { len: 32, max: 31 });

        let ok = "v_this_name_is_thirty_one_chars";
        assert_eq!(ok.len(), 31, "the longest the field stores");
        assert!(assemble(&fs(&code, &[io(ok, 1)], &[]), &[], 0).is_ok());
    }

    #[test]
    fn a_program_with_inputs_and_no_temps_is_refused() {
        /* The driver puts unread vertex elements in temporaries ABOVE the
         * shader's count, so an under-count aliases a live temporary with an
         * attribute nobody reads. pvrgpu-platform calls this the most
         * dangerous field in the interface. */
        let _s = store::test_keep();
        let code = [0u32; 4];
        let ins = [io("a_pos", 0)];
        let outs = [io("gl_Position", 0)];
        let mut p = vs(&code, &ins, &outs);
        p.temps = 0;
        assert_eq!(err(&p), AbiError::TempsUnset);
    }

    #[test]
    fn temps_must_cover_every_io_register_because_they_share_one_file() {
        /* shaderc-remote's point, and the sharpest risk in the whole
         * interface: on this machine attributes and outputs are reservations
         * INSIDE the temporary file, not a separate space. gc_shaders.h has
         * the bring-up triangle at aPos r0, aColor r1, NUM_TEMPS 2. So a
         * passthrough vertex shader that allocated no temporary at all still
         * needs temps above its io registers -- and an allocator that treated
         * them as external would report 0, which the driver would then write
         * unread vertex elements on top of. */
        let _s = store::test_keep();
        let code = [0u32; 4];
        let ins = [io("a_pos", 0), io("a_color", 1)];
        let outs = [io("gl_Position", 0), io("v_color", 1)];

        let mut p = vs(&code, &ins, &outs);
        p.temps = 2; /* the gc_shaders.h number */
        assert!(assemble(&p, &[], 0).is_ok(), "two io registers need temps = 2");

        let mut p = vs(&code, &ins, &outs);
        p.temps = 1;
        assert_eq!(
            err(&p),
            AbiError::TempsBelowIo { temps: 1, reg: 1, what: "an input" },
            "a count that covers r0 but not r1 is refused, naming the register"
        );

        /* the same for a reported output register, which is equally inside the
         * file and equally easy to forget */
        let mut p = vs(&code, &[], &[]);
        p.temps = 1;
        p.vs_pos_out_reg = Some(3);
        assert_eq!(err(&p), AbiError::TempsBelowIo { temps: 1, reg: 3, what: "gl_Position" });
    }

    #[test]
    fn the_constant_file_is_a_per_dword_table_and_not_a_block_size() {
        /* The difference from the PowerVR: the compiler owns the layout and
         * says what every dword means, including dwords that are properties of
         * a bound texture rather than uniforms at all. */
        let _s = store::test_keep();
        let code = [0u32; 4];
        let consts = [
            ConstSlot::literal(0x3f80_0000), /* 1.0f, folded in */
            ConstSlot::uniform(3),           /* block dword 3 */
            ConstSlot::UNUSED,
            ConstSlot { contents: crate::shaderc::SHC_VIV_CONST_TEXTURE_WIDTH, data: 2 },
        ];
        let sh = assemble(&fs(&code, &[], &consts), &[], 0).expect("assembles");
        assert_eq!(sh.viv.const_count, 4);
        assert_eq!(
            sh.viv.const_contents(),
            [
                SHC_VIV_CONST_LITERAL,
                SHC_VIV_CONST_UNIFORM,
                SHC_VIV_CONST_UNUSED,
                crate::shaderc::SHC_VIV_CONST_TEXTURE_WIDTH,
            ]
        );
        assert_eq!(sh.viv.const_data(), [0x3f80_0000, 3, 0, 2]);
    }

    #[test]
    fn a_literal_colour_needs_no_uniform_path_at_all() {
        /* Why M3's first triangle takes its colour as a literal: it is the
         * shortest path that exercises the constant file, not a way around a
         * driver defect.
         *
         * An earlier version of this comment said a fragment uniform does not
         * reach its shader on this part, citing a 2026-09-21 measurement.
         * **That premise is withdrawn.** cts-runner re-measured it on
         * 2026-09-22 across every path -- offline at HEAD on both pools, the
         * Sep-21 binary, and the remote pair a51616c -- and gl_test case 1
         * passes all ten checks on each. Nothing since has touched
         * emit_uniforms, const_value or take_uniforms. So a fragment uniform
         * does arrive, and anything held back "for that defect" is held back
         * for nothing.
         *
         * Kept as a literal anyway, for its own reason: a LITERAL slot is
         * resolved from const_data alone and needs no uniform plumbing above
         * it, which makes this the smallest thing that can be wrong. The
         * uniform path has its own test beside this one. */
        let _s = store::test_keep();
        let code = [0u32; 4];
        let red = [
            ConstSlot::literal(0x3f80_0000), /* 1.0 */
            ConstSlot::literal(0),           /* 0.0 */
            ConstSlot::literal(0),           /* 0.0 */
            ConstSlot::literal(0x3f80_0000), /* 1.0 */
        ];
        let sh = assemble(&fs(&code, &[], &red), &[], 0).expect("assembles");
        assert!(
            sh.viv.const_contents().iter().all(|&c| c == SHC_VIV_CONST_LITERAL),
            "not one slot reads the uniform block"
        );
        assert_eq!(sh.uniform_dwords, 0, "and the shader declares no uniform block");
    }

    #[test]
    fn a_shader_with_no_constants_reports_no_table_rather_than_an_empty_one() {
        /* A null pointer with count 0 is what the C side reads as "none"; a
         * dangling pointer with count 0 is a pointer something may still
         * follow. */
        let _s = store::test_keep();
        let sh = assemble(&fs(&[0u32; 4], &[], &[]), &[], 0).expect("assembles");
        assert_eq!(sh.viv.const_count, 0);
        assert!(sh.viv.const_contents.is_null() && sh.viv.const_data.is_null());
        assert!(sh.viv.const_contents().is_empty() && sh.viv.const_data().is_empty());
    }

    #[test]
    fn link_fills_vs_reg_of_in_and_the_assembly_leaves_it_absent() {
        let _s = store::test_keep();
        let sh = assemble(&fs(&[0u32; 4], &[io("v_uv", 1)], &[]), &[], 0).expect("assembles");
        assert!(sh.viv.vs_reg_of_in.iter().all(|&r| r == SHC_VIV_NO_REG));
    }

    #[test]
    fn a_fragment_shader_has_no_blend_variant_on_this_gpu() {
        /* The pixel engine blends, so there is no FS-lowered blend and no
         * blend-constant window. blend_consts_off must be its sentinel and not
         * dword 0: an omitted sentinel read as 0 cost this project six cases. */
        let _s = store::test_keep();
        let sh = assemble(&fs(&[0u32; 4], &[], &[]), &[], 0).expect("assembles");
        assert_eq!(sh.blend_key, 0);
        assert_eq!(sh.blend_consts_off, SHC_NO_BLEND_CONSTS);
        /* Zero, both stages: all 307 shipped blobs carry 0 here. The colour
         * is named by ps_color_out_reg on this GPU, not counted. */
        assert_eq!(sh.num_outputs, 0);
    }

    #[test]
    fn the_reflection_rows_are_passed_through_in_order() {
        /* Row order is load-bearing: glGetActiveUniform indices follow it, so
         * this file must not sort, filter or invent rows. */
        let _s = store::test_keep();
        let mut a = ShcUniform::ZERO;
        a.dword_off = 7;
        let mut b = ShcUniform::ZERO;
        b.dword_off = 2;
        let sh = assemble(&fs(&[0u32; 4], &[], &[]), &[a, b], 9).expect("assembles");
        let got = sh.uniform_table();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].dword_off, got[1].dword_off), (7, 2), "declaration order, not sorted");
        assert_eq!(sh.uniform_dwords, 9);
    }

    #[test]
    fn more_io_than_the_abi_holds_is_refused() {
        let _s = store::test_keep();
        let many: std::vec::Vec<IoReg<'_>> = (0..SHC_VIV_MAX_IO + 1).map(|i| io("v", (i + 1) as u8)).collect();
        assert_eq!(
            err(&fs(&[0u32; 4], &many, &[])),
            AbiError::TooManyIo { got: SHC_VIV_MAX_IO + 1, max: SHC_VIV_MAX_IO }
        );
    }

    #[test]
    fn a_vertex_shader_reports_its_attributes() {
        /* The board found this one: the program compiled and linked, and then
         * GL_ACTIVE_ATTRIBUTES was 0 with no GL error, so the application
         * could not find `a_pos` to bind it. A shader with no attributes is
         * legal, so nothing downstream could tell that apart from nobody
         * having filled these in. */
        let _s = store::test_keep();
        let code = [0u32; 4];
        let ins = [io("a_pos", 0), io("a_uv", 1)];
        let outs = [io("gl_Position", 0)];
        let mut p = vs(&code, &ins, &outs);
        p.temps = 3;
        let sh = assemble(&p, &[], 0).expect("assembles");
        assert_eq!(sh.num_attribs, 2);
        assert_eq!(crate::util::name_bytes(&sh.attribs[0].name), b"a_pos");
        assert_eq!(crate::util::name_bytes(&sh.attribs[1].name), b"a_uv");
        /* The register the shader actually reads, so the driver routes the
         * bound array to the same place the code looks. */
        assert_eq!(sh.attribs[0].vi_reg, ins[0].reg);
        assert_eq!(sh.attribs[1].vi_reg, ins[1].reg);
        assert_eq!(sh.attribs[0].location, 0);
        /* 0 in all 588 attributes of the offline table. */
        assert!(sh.attribs.iter().all(|a| a.format == 0));
        /* The GC7000 wires the stages through ShcViv, so this stays empty --
         * all 179 vertex blobs in the table carry num_varyings = 0. */
        assert_eq!(sh.num_varyings, 0);
    }

    #[test]
    fn a_fragment_shader_reports_no_attributes() {
        /* Its io_in are varyings. Reporting them as attributes would make
         * glGetAttribLocation answer for a name the vertex stage never had. */
        let _s = store::test_keep();
        let sh = assemble(&fs(&[0u32; 4], &[io("v_color", 1)], &[]), &[], 0).expect("assembles");
        assert_eq!(sh.num_attribs, 0);
    }

    #[test]
    fn the_two_abi_words_are_measured_values_and_not_zero() {
        /* These used to be handed in by the caller, and the caller handed in
         * 0 -- a value no shipped blob carries for either of them in either
         * stage. Both are derived here now, from the offline table:
         * input_count_unk8 is DIV_ROUND_UP(num_in + 4, 16) for vertex and 31
         * for fragment, and vs_load_balancing is a function of the vertex
         * output count. */
        let _s = store::test_keep();
        let code = [0u32; 4];
        let ins = [io("a_pos", 0)];
        let outs = [io("gl_Position", 0)];
        let sh = assemble(&vs(&code, &ins, &outs), &[], 0).expect("assembles");
        assert_eq!(sh.viv.input_count_unk8, 1, "vertex");
        assert_eq!(sh.viv.vs_load_balancing, 0x0f3f_0241, "one vertex output");

        let fsh = assemble(&fs(&code, &[], &[]), &[], 0).expect("assembles");
        assert_eq!(fsh.viv.input_count_unk8, 31, "fragment");
        assert_eq!(fsh.viv.vs_load_balancing, 0, "a fragment shader balances nothing");
        /* All 307 blobs carry 0 here, both stages; 1 was the PowerVR reading. */
        assert_eq!(fsh.num_outputs, 0);
        assert_eq!(sh.num_outputs, 0);
    }

    #[test]
    fn the_vertex_input_count_is_a_formula_and_not_the_constant_the_corpus_shows() {
        /* The offline table's 179 vertex blobs all carry 1 here, and all have
         * 10 inputs or fewer -- so the table agrees with a constant 1 and with
         * DIV_ROUND_UP(num_in + 4, 16) equally, and cannot choose between
         * them. The formula is etnaviv's (etnaviv_compiler_nir.c:1157). This
         * test pins the half of it the corpus cannot see: the step at 13.
         *
         * 13 is reachable -- MAX_VERTEX_ATTRIBS is 16 -- so the constant was
         * wrong for a shader the product accepts, not merely under-evidenced. */
        for num_in in 0..=12usize {
            assert_eq!(input_count_unk8(SHC_STAGE_VERTEX, num_in), 1, "num_in {num_in}");
        }
        for num_in in 13..=SHC_VIV_MAX_IO {
            assert_eq!(input_count_unk8(SHC_STAGE_VERTEX, num_in), 2, "num_in {num_in}");
        }
        /* The fragment stage really is a constant: etnaviv computes the same
         * formula and then throws it away (:1389). */
        for num_in in 0..=SHC_VIV_MAX_IO {
            assert_eq!(input_count_unk8(SHC_STAGE_FRAGMENT, num_in), 31, "num_in {num_in}");
        }

        /* And it arrives through assemble(), not just the helper. A vertex
         * shader with 13 attributes: legal under GL ES 2.0 here, and the
         * smallest one the old constant got wrong. vs() pins temps = 1, so
         * this builds the program itself -- inputs and temporaries share one
         * register file and 13 inputs need 13. */
        const NAMES: [&str; 13] = ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8", "a9", "a10", "a11", "a12"];
        let _s = store::test_keep();
        let code = [0u32; 4];
        let outs = [io("gl_Position", 0)];
        let ins: [IoReg<'_>; 13] = core::array::from_fn(|i| io(NAMES[i], i as u8));
        let p = VivProgram { temps: 13, ..vs(&code, &ins, &outs) };
        let sh = assemble(&p, &[], 0).expect("assembles");
        assert_eq!(sh.viv.num_in, 13);
        assert_eq!(sh.viv.input_count_unk8, 2, "13 attributes is the step");
    }

    #[test]
    fn each_measured_load_balancing_band_is_pinned() {
        let _s = store::test_keep();
        let code = [0u32; 4];
        let ins = [io("a_pos", 0)];
        for (num_out, want) in
            [(1u8, 0x0f3f_0241u32), (2, 0x0f3f_0321), (5, 0x0f3f_0316), (7, 0x0f3f_0311), (9, 0x0f3f_030e)]
        {
            let outs: Vec<IoReg> = (0..num_out).map(|i| io("gl_Position", i)).collect();
            let mut p = vs(&code, &ins, &outs);
            p.temps = u16::from(num_out) + 1;
            let sh = assemble(&p, &[], 0).expect("assembles");
            assert_eq!(sh.viv.vs_load_balancing, want, "num_out {num_out}");
        }
    }

    /// A backward branch is refused, and a forward one is not.
    ///
    /// Both halves matter. Asserting only that a real program passes would
    /// hold on a validator that never looks at anything, so the backward case
    /// is the control that the check can fail at all -- the same shape
    /// shaderc-remote used for the r0 reservation, where a fragment-stage test
    /// could not have failed because r0 was already reserved there.
    #[test]
    fn a_backward_branch_is_refused_and_a_forward_one_is_not() {
        use super::super::vivante::{build::Builder, encode};

        /* Through the real encoder, so the words are the ones the hardware
         * would see rather than a hand-assembled approximation of them. */
        let build_branch = |target: u16| -> [u32; 4] {
            let i = Builder::new("branch").set("TARGET", u32::from(target)).build().expect("a branch encodes");
            let mut w = [0u32; 4];
            encode::encode(&[i], &mut w).expect("one instruction fits four dwords");
            w
        };
        /* Three instructions; the branch sits at index 1. */
        let mut code = [0u32; 12];
        code[4..8].copy_from_slice(&build_branch(2));
        assert!(every_branch_goes_forward(&code, &[]).is_ok(), "a forward branch is fine");

        code[4..8].copy_from_slice(&build_branch(1));
        assert_eq!(
            every_branch_goes_forward(&code, &[]),
            Err(AbiError::BranchTargetBackwards { at: 1, target: 1 }),
            "a branch to itself is a loop with no loop in the source"
        );

        code[4..8].copy_from_slice(&build_branch(0));
        assert!(matches!(every_branch_goes_forward(&code, &[]), Err(AbiError::BranchTargetBackwards { .. })));

        /* One past the last instruction is "the end" and is legal; beyond it
         * runs whatever follows the shader in the instruction cache. */
        code[4..8].copy_from_slice(&build_branch(3));
        assert!(every_branch_goes_forward(&code, &[]).is_ok(), "a target of ninstr is the end");
        code[4..8].copy_from_slice(&build_branch(4));
        assert!(matches!(every_branch_goes_forward(&code, &[]), Err(AbiError::BranchTargetPastEnd { .. })));

        /* A loop's back edge is the one exception, and it is granted by INDEX:
         * the branch at 1 may go back, a branch anywhere else may not. That is
         * what stops "this shader has a loop" from excusing every backward
         * jump in it. */
        code[4..8].copy_from_slice(&build_branch(0));
        assert!(every_branch_goes_forward(&code, &[1]).is_ok(), "the branch named as a back edge may go back");
        assert!(
            every_branch_goes_forward(&code, &[2]).is_err(),
            "naming a different instruction does not excuse this one"
        );
    }

    /// cts-runner's six measured arms, so the guard cannot be removed again
    /// by the argument that removed it once.
    ///
    /// I reasoned that `temps` is absent from Mesa's expression, therefore the
    /// ceiling was a correlation, therefore the refusal was rejecting correct
    /// programs. The board disagreed on every count that mattered:
    ///
    /// | shader | back end | temps | outcome |
    /// |---|---|---:|---|
    /// | hsl_to_rgb_vertex | ours | 67 | WEDGE |
    /// | hsl_to_rgb_vertex | reference | 10 | draws |
    /// | a_many_temps | ours | 44 | **WEDGE** |
    /// | a_many_temps | reference | 40 | **WEDGE** |
    /// | b_many_instructions | ours | 5 | draws |
    /// | b_many_instructions | reference | 2 | draws |
    ///
    /// Two things follow. The six-bit truncation is not the mechanism -- 44
    /// and 40 do not cross it and hang anyway. And the reference compiler
    /// hangs the same hardware at 40, so this is not a defect in our back end
    /// at all; it is a bound on the load-balancing word that whoever writes
    /// the shader has to respect.
    #[test]
    fn the_arms_that_hung_the_board_are_refused_and_the_ones_that_drew_are_not() {
        /* These probes emit few varyings, so they sit in the first band,
         * whose measured ceiling is 21. */
        const FIRST_BAND: u8 = 1;
        for temps in [67u16, 44, 40] {
            assert!(
                vs_load_balancing(SHC_STAGE_VERTEX, FIRST_BAND, temps).is_err(),
                "temps {temps} wedged the board and must be refused"
            );
        }
        for temps in [10u16, 5, 2] {
            assert!(
                vs_load_balancing(SHC_STAGE_VERTEX, FIRST_BAND, temps).is_ok(),
                "temps {temps} drew and must not be refused"
            );
        }
        /* The real limit is somewhere in 11..39. This bound is 21, which is
         * inside that window and measured from the shipped blobs -- safe, but
         * not the number. A bisect replaces it. */
        assert!((11..=39).contains(&21u16));
    }

    /// The second band's ceiling, against the arm that falsified the first
    /// value for it.
    ///
    /// `n2_k22` allocates 27 temporaries and WEDGED the board at ccb2e81,
    /// while this ceiling stood at 42. So 42 refused nothing that hung and
    /// the guard did not guard. 26 is the neighbouring count that DREW
    /// (`k = 21`), and 27..29 (`k = 22..24`) wedge on file.
    ///
    /// Exactly 26, not 25: cts-runner's control rung is the 26 one, and a
    /// ceiling of 25 would turn the control into an override arm -- the
    /// experiment would then need the escape hatch to run its own baseline,
    /// which is how a control stops being a control.
    #[test]
    fn the_second_band_refuses_the_count_that_wedged_and_admits_the_one_that_drew() {
        const SECOND_BAND: u8 = 2;
        for temps in [27u16, 28, 29] {
            assert!(
                vs_load_balancing(SHC_STAGE_VERTEX, SECOND_BAND, temps).is_err(),
                "temps {temps} wedged the board in this band and must be refused"
            );
        }
        assert!(
            vs_load_balancing(SHC_STAGE_VERTEX, SECOND_BAND, 26).is_ok(),
            "temps 26 drew in this band and is cts-runner's control rung: refusing it \
             would make the control an override arm"
        );
        // Band 3 shares the bound, and a change to one that missed the other
        // would leave half the band unguarded.
        assert!(vs_load_balancing(SHC_STAGE_VERTEX, 3, 27).is_err());
        assert!(vs_load_balancing(SHC_STAGE_VERTEX, 3, 26).is_ok());
    }

    /// The formula must reproduce every value the shipped blobs carry.
    ///
    /// This is the whole justification for replacing a table of measurements
    /// with an expression: five bands, 169 vertex blobs, and the expression
    /// has to hit all five from the chip's specs alone. If a regenerated
    /// table ever disagrees, or someone edits the constants, this fails
    /// rather than the board.
    ///
    /// `num_out` 8 is absent from the list because no blob has it -- that is
    /// the gap the formula closes, and the test below covers it.
    #[test]
    fn the_formula_reproduces_every_shipped_band() {
        const MEASURED: &[(u8, u32)] = &[
            (0, 0x0f3f_0241),
            (1, 0x0f3f_0241),
            (2, 0x0f3f_0321),
            (3, 0x0f3f_0321),
            (4, 0x0f3f_0316),
            (5, 0x0f3f_0316),
            (6, 0x0f3f_0311),
            (7, 0x0f3f_0311),
            (9, 0x0f3f_030e),
        ];
        for &(num_out, want) in MEASURED {
            let got = vs_load_balancing(SHC_STAGE_VERTEX, num_out, 0).unwrap();
            assert_eq!(got, want, "num_out {num_out}: {got:#010x} != {want:#010x}");
        }
        /* A fragment shader does not balance anything, and that is why this
         * field can never explain a difference between the two stages. */
        assert_eq!(vs_load_balancing(SHC_STAGE_FRAGMENT, 4, 99).unwrap(), 0);
    }

    /// The VALUE does not depend on register use. Its ADMISSIBILITY does, and
    /// conflating the two cost a board hang.
    ///
    /// Mesa's expression takes only the output count and the chip specs, so
    /// the same `num_out` yields the same word whatever the shader's register
    /// pressure — that is what this pins. I then reasoned one step too far:
    /// that because the value does not depend on `temps`, no limit on `temps`
    /// could be real, and removed the ceiling. It is real. The word Mesa
    /// computes is only *valid* up to some register pressure, and above it the
    /// GPU hangs for either compiler's output.
    ///
    /// So: this test says the formula is a function of `num_out` alone. It
    /// says nothing about which shaders may use the result, which is
    /// `the_arms_that_hung_the_board_are_refused_and_the_ones_that_drew_are_not`.
    #[test]
    fn the_balancing_value_is_a_function_of_the_output_count_alone() {
        for num_out in 0..=9u8 {
            let w = vs_load_balancing(SHC_STAGE_VERTEX, num_out, 0).unwrap();
            /* Same word at any admissible register count. */
            assert_eq!(w, vs_load_balancing(SHC_STAGE_VERTEX, num_out, 16).unwrap());
            assert_eq!(w >> 16, 0x0f3f, "C and D are constant in every band");
        }
    }

    #[test]
    fn an_output_count_no_blob_records_is_refused_rather_than_extrapolated() {
        /* Eight outputs appear in no shipped blob, so the old table refused
         * them -- a count GL ES 2.0 requires us to accept. The formula
         * computes it. */
        let _s = store::test_keep();
        let code = [0u32; 4];
        let ins = [io("a_pos", 0)];
        let outs: Vec<IoReg> = (0..8).map(|i| io("v", i)).collect();
        let mut p = vs(&code, &ins, &outs);
        p.temps = 9;
        /* The formula closed this gap: eight outputs share a band with nine. */
        assert_eq!(assemble(&p, &[], 0).unwrap().viv.vs_load_balancing, 0x0f3f_030e);
    }
}
