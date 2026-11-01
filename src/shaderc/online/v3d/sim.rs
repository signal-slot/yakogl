// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (qpu/, compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! A test-only interpreter of ENCODED V3D 4.2 QPU programs: one QPU, 16
//! lanes, run through [`super::decode`].
//!
//! It exists so that code can be judged without a board: Mesa's code first
//! (to validate this interpreter), then ours (`docs/online-shaderc/
//! v3d-backend.md`, "How it is checked without a board"). It vouches only for
//! what it models. Anything else stops the run with an `Err` naming it
//! ("unmodelled"), never with a value: an operation, a signal, a magic
//! register, a flag or a timing it does not know, a read of a register that
//! holds nothing defined, a NaN or a denormal (whose hardware treatment is
//! not written down anywhere we may read), two writes landing on one lane of
//! one register in the same cycle.
//!
//! # Where each semantic comes from
//!
//! Every behaviour below is taken from Mesa 26.2.1's v3d compiler
//! (`src/broadcom/compiler`, MIT; Copyright (c) Broadcom) -- what it emits
//! for which NIR operation, and what its scheduler and validator require --
//! never from a GPL source, and never from memory. Where Mesa does not settle
//! a point, the point is unmodelled or is marked ASSUMPTION here.
//!
//! * **Arithmetic.** Each op computes what the NIR op Mesa maps to it means
//!   (`nir_to_vir.c` `ntq_emit_alu`): FADD/FSUB/FMUL/FMIN/FMAX the f32 ops,
//!   ADD/SUB/NEG/AND/OR/XOR/NOT/MIN/MAX/UMIN/UMAX/SHL/SHR/ASR/ROR the i32
//!   ones (shift counts mod 32: Mesa's own `sin` lowering shifts by -1 to
//!   mean 31, `ntq_fsincos`), FFLOOR/FCEIL/FTRUNC, FROUND (`fround_even`),
//!   FTOIZ (`f2i32`), FTOUZ (`f2u32`), FTOIN (`f2i32` of `fround_even`,
//!   `nir_to_vir.c:1452-1458`), ITOF/UTOF, VFPACK (`pack_half_2x16_split`:
//!   a to the low half, b to the high, `nir_to_vir.c:1807`), MOV/FMOV,
//!   UMUL24/SMUL24 (the low 24 bits of each operand; `qpu_instr.h`'s
//!   UMUL24_RTOP0 comment: UMUL24 adds an rtop register only MULTOP writes).
//!   ASSUMPTION: IEEE-754 binary32, round to nearest even, as `eval.rs` and
//!   Mesa's own constant folding compute; f32 -> f16 rounds to nearest even
//!   (NIR's `pack_half_2x16_split`). Not measured on silicon. A denormal
//!   operand or result, or a NaN, is unmodelled.
//! * **SFU** (RECIP, RSQRT, EXP = exp2, LOG = log2, SIN): `ntq_emit_alu`'s
//!   `frcp`/`frsq`/`fexp2`/`flog2`; SIN takes its argument in units of pi,
//!   reduced to [-0.5, 0.5] (`ntq_fsincos`, `nir_to_vir.c:992-1003`). The
//!   hardware's precision is not known: the interpreter computes the exact
//!   f32 function and lists every SFU op it ran in
//!   [`Outcome::approximate`], so the caller can compare with a tolerance.
//!   Timing: a register-file destination read by the very next
//!   instruction STALLS until the result is there (`read_stalls`,
//!   `qpu_schedule.c:779-786`: the "stallable SFU" register; Mesa counts
//!   these as `sfu-stalls`), so the value is visible from the next
//!   instruction on. An accumulator destination has no such rule in Mesa
//!   (its scheduler's SFU latency is 2, `qpu_schedule.c:1866`): its result
//!   lands two instructions later, and a read before then is unmodelled.
//! * **Flags.** Per lane, A and B. `pushz/pushn/pushc` move A to B and set A.
//!   What each op sets is read off how Mesa uses it
//!   (`ntq_emit_comparison`, `ntq_fsign`, `ntq_emit_bool_to_cond`): `feq` =
//!   FCMP.pushz + ifa, `flt` = FCMP.pushn + ifa, `fge(a, b)` =
//!   FCMP(b, a).pushc + ifa, `ilt(a, b)` = MIN(b, a).pushc + ifa, `ult(a, b)`
//!   = SUB(a, b).pushc + ifa, `ieq` = XOR.pushz, sign = FMOV.pushz then
//!   FMOV.pushn, a bool = MOV.pushz. So FCMP(x, y) sets Z = x == y, N = x <
//!   y, C = x <= y; MIN(x, y) sets C = x > y (signed); SUB(x, y) sets C = x
//!   < y (unsigned); FMOV sets Z = x == 0.0, N = x < 0.0; MOV, XOR and MSF
//!   set Z = bits == 0. Any other op's flag is unmodelled. `and<X>`/`nor<X>`
//!   set A = A & X / !A & !X (Mesa's comments on `vote_all`/`vote_any`,
//!   `nir_to_vir.c:4163-4194`); what they do to B is not said, so B becomes
//!   undefined. IFA/IFNA/IFB/IFNB write only the lanes they select.
//! * **Undefined lanes.** A lane holds no value until something writes it,
//!   and Mesa computes on such lanes freely where their results are never
//!   used (divergent control flow: `mov.ifa rf3, 0` writes the active lanes,
//!   then `itof r1, rf3` runs in all 16). So an undefined operand lane is
//!   not refused where it is read: the op computes on a stand-in (1.0, which
//!   no op refuses) and its result, its flags and a write under an
//!   undefined condition flag are undefined in that lane. The refusal is at
//!   the USE, every sink: a TLB write carries the lane as undefined (the
//!   caller refuses it where the pixel is kept), a VPM store or a VPM row,
//!   a TMU coordinate, address or data, UNIFA (lane 0), SETMSF and its
//!   condition, and a branch condition.
//! * **Signals.** LDUNIF writes r5 one instruction later, LDVARY writes r5
//!   (the C coefficient) two instructions later (`qpu_validate.c:220`);
//!   LDVARY's own destination and LDUNIFRF's land at the next instruction,
//!   as every ALU write does. LDVARY takes the next varying component in
//!   the FS's input order (`emit_fragment_varying` appends one input slot
//!   per LDVARY). THRSW: two delay slots; the thread switch happens after
//!   them and invalidates the accumulators (`vir_register_allocate.c:1340`).
//!   The flags survive it ("In V3D 4.2+ flags are preserved across thread
//!   switches", `vir_opt_redundant_flags.c:109-112`), and a flag write in
//!   a delay slot is an ordinary write in the old thread: the delay slots
//!   run before the switch ("the cycle at which the actual thread switch
//!   ... happens ... would be after the 2 delay slots", qpu_schedule.c
//!   around 2098-2110). Mesa puts one there when it moves a THRSW up past
//!   an instruction that came before it (`qpu_inst_before_thrsw_valid_in_
//!   delay_slot` has no flag rule; the flag rule at `qpu_schedule.c:2203`
//!   is for instructions from after the THRSW); the corpus has three
//!   (environmentmapprefilter_300 fs 223, lightmapdilate_300 fs 187 and
//!   229). Two THRSW
//!   in a row mark the last thread section, and the next THRSW ends the
//!   program after its two delay slots (`qpu_validate.c:350-369`).
//!   ASSUMPTION: a program with no such pair ends at its first THRSW; the
//!   validator decides this from the compiler's `c->last_thrsw`
//!   (`qpu_validate.c:464-469`), which the binary does not carry.
//! * **Uniforms.** LDUNIF/LDUNIFRF, a TLBU write (its configuration) and a
//!   branch each take the next value of the stream
//!   (`vir_emit_tlb_color_write`, `vir_branch_inst`). A taken branch moves
//!   the stream to its own uniform + 1 + value/4 (`qpu_set_branch_targets`,
//!   `qpu_schedule.c:2968-2982`).
//! * **Branches.** REL only: the target is the branch + 4 + offset/8; three
//!   delay slots (`qpu_schedule.c:2968`, `emit_branch`). ALWAYS, ALLA, ANYA,
//!   ALLNA, ANYNA over the lanes; msfign P ignores lanes whose multisample
//!   flags are 0 (`nir_to_vir.c:4332-4380`). A0/NA0, msfign Q and a uniform
//!   read in a delay slot (Mesa never emits one,
//!   `qpu_inst_valid_in_branch_delay_slot`) are unmodelled.
//! * **Uniforms by address** (indexed uniforms). A write to the magic UNIFA
//!   register (an ALU write, or LDUNIFRF straight into it) sets the address
//!   of a second stream, taken from lane 0 ("the address that is used to
//!   write to the unifa register is taken from the first lane",
//!   `ntq_emit_load_unifa`, nir_to_vir.c:3262-3270). LDUNIFA (to r5, one
//!   instruction later, like LDUNIF: qpu_validate.c:220 pairs them) and
//!   LDUNIFARF (to its signal address) read the 32-bit word there and add 4
//!   to the address ("Every ldunifa auto-increments the unifa address by 4
//!   bytes", nir_to_vir.c:3251). A read within 3 instructions of the UNIFA
//!   write (qpu_schedule.c:1437-1442), a UNIFA write within 3 instructions
//!   of a thread switch or end (qpu_schedule.c:2092-2105), or an address outside the
//!   caller's [`Env::memory`] is unmodelled.
//! * **Fragment I/O.** rf0 = W, rf1 = centroid W, rf2 = Z at entry
//!   (`nir_to_vir.c:4851-4858`); FXCD/FYCD give the fragment's x and y
//!   (`emit_fragcoord_input`); LDVARY gives (vary, C) per lane, which the
//!   caller supplies. TLB writes are recorded with the configuration a TLBU
//!   write read. MSF/SETMSF read and write the per-lane multisample flags
//!   (ASSUMPTION: 1 at entry; any non-zero value serves, since Mesa only
//!   compares them with their start value or with zero).
//! * **Vertex I/O.** LDVPMV_IN reads VPM input row `a` of the lane's vertex,
//!   STVPMV writes `b` to output row `a` (`ntq_emit_load_input`,
//!   `vir_VPM_WRITE_indirect`); the row must be the same in every lane.
//!   VPMWT and TMUWT wait, and do nothing here (TMUWT's destination is
//!   not written: Mesa's "is not really read", vir_register_allocate.c:
//!   532-533).
//!
//! * **Texture reads (TMU)**, the regular 2D and cube lookups, as Mesa emits them
//!   (`v3d_vir_emit_tex`, v3d_tex.c:222-393): magic writes to TMUT/TMUR/
//!   TMUI/TMUB/TMUDREF/TMUOFF give the coordinates, WRTMUC signals take the
//!   configuration words P0 (then P1, P2) from the uniform stream, and the
//!   write to TMUS or TMUSLOD (S, "triggers the lookup", v3d_tex.c:58)
//!   retires the request. P0's bits 0..3 say which result words come back,
//!   its bits 24..31 which texture: the hardware reads a texture state
//!   address there (V3D42 "TMU Config Parameter 0", v3d_packet.xml), which
//!   the driver writes over the compiler's unit placeholder (v3d_tex.c:
//!   303-306); the interpreter takes the unit, so the caller passes the
//!   compiler's value unrelocated. The texture state's swizzle is ASSUMED
//!   to be the identity. P1's bit 0
//!   whether they are 32-bit (else f16 pairs: r|g<<16, b|a<<16); the field
//!   layouts are V3D42_TMU_CONFIG_PARAMETER_0/1/2 in Mesa's generated
//!   v3d_packet_v42_pack.h. LDTMU takes the next returned word, to its
//!   signal address one instruction later. The caller describes each
//!   texture ([`Texture`]: 2D or cube, its levels, its minification
//!   filter) and, by the sampler index P1's bits 24..31 carry, the
//!   sampler state ([`Env::samplers`]: the magnification and minification
//!   filters and the wrap modes; a lookup whose index names no entry
//!   samples NEAREST with the texture's own minification filter and
//!   CLAMP_TO_EDGE, as every program did before the sampler state was
//!   modelled); the coordinates are normalised (the texture and sampler
//!   states are the driver's, not the program's). The level and the cube
//!   face are dEQP's, through the model the IR evaluator shares
//!   ([`crate::shaderc::online::texref`], pinned bit for bit against
//!   v3d-tex's C ports): the level of detail is TMUB's when the lookup is
//!   retired by TMUSLOD or carries P2's disable_autolod (v3d_tex.c:137-153,
//!   370-379), else the derivatives' (and TMUB a bias), which is not
//!   modelled and so read only on a texture of one level; a cube lookup
//!   is S by TMUSCM with T and R, the face chosen from the direction, and
//!   filtered SEAMLESSLY whatever the wrap modes, as the V3D 4.2 filters
//!   every cube (docs/v3d-noseamless-cube.md); a 2D-array lookup is S by
//!   TMUS or TMUSLOD with T and the layer in TMUI (Mesa's `is_array` write,
//!   v3d_tex.c), which on a cube texture reads one face as an independent
//!   2D texture with the sampler's wrap modes (the non-seamless emulation's
//!   path; a 2D texture has layer 0 only, and the layer is clamped as GL
//!   clamps it). TMUI is ASSUMED to take a float, like the other
//!   coordinates of a sampled lookup, rounded to the nearest layer (GL ES
//!   3.0 3.8.9: floor(layer + 0.5)); a lookup whose layer is not finite is
//!   refused. LINEAR is dEQP's bilinear rule in f32 (`texref`); the TMU's
//!   fixed-point weights were measured on the board instead (gl_test case
//!   109, docs/online-shaderc/v3d-backend.md "LINEAR on the board"), so a
//!   comparison with hardware allows dEQP's LINEAR tolerance. A texel fetch
//!   (S by TMUSF, v3d_tex.c:372-374) reads the texel at S and T, integers,
//!   of the integer level in TMUB, no filter and no wrap; a level or a
//!   texel outside the texture (undefined in GL) is refused. P2's "Offset
//!   S" / "Offset T" nibbles (a constant texel offset, v3d_tex.c:161-173)
//!   shift the texel read by that many texels of the level read (GLSL ES
//!   3.00 8.8); under a mipmap-LINEAR blend the offset is not modelled.
//!   Refused: a NEAREST coordinate within 1e-3 of a texel edge (the
//!   hardware's rounding there is not known), a level of detail within
//!   dEQP's LOD precision of a level boundary, a cube direction within 1e-3
//!   of a face edge, R without TMUSCM (3D), I with TMUSCM (a cube array), I
//!   with TMUSF (a 2D-array fetch), DREF/OFF (shadow, a non-constant
//!   offset), a P2 that is not a regular lookup (an R offset included), a
//!   cube lookup with an offset, unnormalised coordinates, and conditional
//!   texture writes.
//! * **General memory through the TMU**, as Mesa's spills and fills and
//!   its scratch arrays use it (`vir_register_allocate.c:426-611`): TMUD
//!   gives a lane's word, and the write to TMUA or TMUAU (which takes its
//!   configuration from the stream) retires the access: a write when TMUD
//!   came first, else a read whose word LDTMU takes. Only the two
//!   configurations Mesa uses for them (0xffffffff, per-pixel; 0xffffff7f,
//!   per-quad) and one 32-bit word per lane, at 4-byte-aligned addresses;
//!   a read of an address nothing wrote is refused. TIDX reads
//!   [`Env::thread`]; EIDX the lane number.
//!
//! Unmodelled: derivatives (unless [`Env::quads`] assumes a quad layout),
//! TMUL/TMUC/TMUH*, the magic VPM/SFU/sync
//! writes, RSQRT2 (Mesa 26.2.1 defines it, `VIR_SFU(RSQRT2)`, but nothing
//! calls it and nothing states how it differs from RSQRT; the corpus has
//! none), and the 7.x operations.

use super::decode::decode;
use super::isa::*;
use std::format;
use std::string::String;
use std::vec::Vec;

/// Lanes per QPU instruction.
pub const LANES: usize = 16;

/// One value per lane; `None` is "nothing defined was ever written here".
pub type Lanes = [Option<u32>; LANES];

/// One varying component as the hardware hands it to LDVARY: the
/// interpolated value before the W multiply, and the C coefficient.
#[derive(Clone, Debug)]
pub struct Varying {
    pub vary: [u32; LANES],
    pub c: [u32; LANES],
}

/// A fragment shader's per-lane inputs.
#[derive(Clone, Debug)]
pub struct FsInputs {
    /// rf0 at entry.
    pub w: [u32; LANES],
    /// rf1 at entry.
    pub w_centroid: [u32; LANES],
    /// rf2 at entry.
    pub z: [u32; LANES],
    /// What FXCD and FYCD return.
    pub x: [u32; LANES],
    pub y: [u32; LANES],
    /// What REVF returns: 1 where the primitive is back-facing.
    pub back_facing: [bool; LANES],
    /// In LDVARY order (the FS's input slots).
    pub varyings: Vec<Varying>,
}

/// A vertex or coordinate shader's inputs: the VPM input rows, one value
/// per lane (vertex) each.
#[derive(Clone, Debug)]
pub struct VsInputs {
    pub vpm: Vec<[u32; LANES]>,
}

#[derive(Clone, Debug)]
pub enum Inputs {
    /// Boxed: the per-lane payload is far larger than the vertex variant.
    Fragment(std::boxed::Box<FsInputs>),
    Vertex(VsInputs),
}

/// A sampler's state ([`Env::samplers`]).
pub use crate::shaderc::online::texref::Sampler;
/// A texture the TMU reads: 2D or cube, its levels and its minification
/// filter ([`crate::shaderc::online::texref`], the model the IR evaluator
/// shares). Values should be exactly representable as f16 when the program
/// takes 16-bit results, or the comparison is with the f16 the hardware
/// returns.
pub use crate::shaderc::online::texref::Texture;

/// What a run may read besides its inputs: LDUNIFA's memory and the
/// textures, by the index P0 carries, and the samplers, by the index P1
/// carries (a texture with no sampler entry samples NEAREST, its own
/// minification filter, CLAMP_TO_EDGE: `Sampler::of`); and the QPU thread
/// it runs as, which TIDX reads (the spill base,
/// vir_register_allocate.c:443-446).
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub memory: Vec<Region>,
    pub textures: Vec<Texture>,
    pub samplers: Vec<Sampler>,
    pub thread: u32,
    /// The program's thread count (1, 2 or 4): the TMU output FIFO holds
    /// 16 / threads words per thread. 0 = not given, and a TMU look-up then
    /// stops the run rather than assume one.
    pub threads: u32,
    /// FDX and FDY run on an ASSUMED quad layout: lanes 4q .. 4q + 3 form
    /// quad q, lane 4q + i at column i & 1 and row i >> 1; FDX is the
    /// right pixel's value less the left one's in the lane's row, FDY the
    /// second row's less the first row's in its column. No public source
    /// states the V3D's layout, so this is off by default (a derivative is
    /// then unmodelled); a test that turns it on builds its inputs from the
    /// same layout, and a result that depends only on |d/dx| and |d/dy|
    /// together (a level of detail) does not depend on which of the two
    /// directions is which, nor on the signs.
    pub quads: bool,
    /// The textures (bit = index into `textures`) of an integer type
    /// (RGBA8UI .. RGBA32I, v3d_packet.xml 96-114): their texels are
    /// integers held as floats, which the TMU returns raw, a 32-bit word a
    /// channel at the 32-bit output, else 16-bit halves (the integer's low
    /// 16 bits, two channels a word: Mesa's nir_lower_tex_packing_16 reads
    /// them back with a 16-bit sign or zero extension). Sampled NEAREST
    /// only (a LINEAR filter on one is refused: GL leaves the texture
    /// incomplete).
    pub integer_textures: u32,
}

/// A piece of memory LDUNIFA can read: 32-bit words from `base` on (a byte
/// address), `None` where nothing defined is stored.
#[derive(Clone, Debug)]
pub struct Region {
    pub base: u32,
    pub words: Vec<Option<u32>>,
}

/// One TLB write: the configuration a TLBU write read from the uniform
/// stream (`None` for TLB), and the value of each lane. A lane is `None`
/// only for a MOV of an undefined register straight to the TLB: Mesa's
/// "Z from FEP" write is `mov tlbu, <nop reg>` with the depth specifier
/// TLB_V42_DEPTH_TYPE_INVARIANT, "Unmodified sideband input used"
/// (`emit_frag_end`, nir_to_vir.c:2083-2095), whose value the hardware
/// ignores; the caller decides, from the specifier, whether a `None` lane
/// is allowed.
#[derive(Clone, Debug)]
pub struct TlbWrite {
    pub config: Option<u32>,
    pub value: Lanes,
}

/// What a finished run produced.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub tlb: Vec<TlbWrite>,
    /// VPM output rows, as written (`None` = never written).
    pub vpm_out: Vec<Option<[u32; LANES]>>,
    /// The multisample flags at the end; a lane at 0 was discarded.
    pub msf: [u32; LANES],
    /// The SFU operations executed, whose precision only the hardware knows.
    pub approximate: Vec<&'static str>,
    pub instructions: usize,
    pub uniforms_read: usize,
    /// TMU look-ups performed (S writes that retired a request).
    pub tmu_lookups: usize,
    /// The cube faces texture look-ups read (bit n: `Face::ALL[n]`), and the
    /// levels they read (bit k: level k): what a run exercised of the cube
    /// and mipmap model, for a harness to count.
    pub cube_faces: u8,
    pub levels: u32,
    /// LDTMUs executed, and of those the ones that took a word returned by
    /// a request made with no thread switch since. In a program of more
    /// than one thread that is refused (see [`NO_SWITCH_HANG`]); the count
    /// can then only be non-zero with one thread, where Mesa emits no
    /// switch at all (vir_emit_thrsw, nir_to_vir.c:152-160) and the board
    /// has not been measured.
    pub ldtmus: usize,
    pub ldtmus_without_switch: usize,
}

type R<T> = Result<T, String>;

/// Why an LDTMU with no thread switch since its request is refused in a
/// program of more than one thread. Not a rule Mesa states (its LDTMU may
/// stall, qpu_schedule.c:1561-1576; it always switches first when
/// threaded, ntq_flush_tmu), but measured on silicon: gl_test case 2, one
/// variable apart. Only the thread counts above one are refused: one thread
/// is Mesa's own no-switch path and was not measured.
pub const NO_SWITCH_HANG: &str = "an LDTMU reads a TMU word requested with no thread switch since, \
in a threaded program: this hangs the V3D 4.2 QPU (Pi 4, gl_test case 2, 2026-09-24: \
f8e20da5 without the switch, render job never completes, 12 timeouts; \
e1165634 with it, PASS, 0 timeouts)";

/// Whether `alu` writes the register file (a non-magic ALU destination, or
/// a signal writing a non-magic address), as qpu_validate.c:373-417 asks.
fn rf_write(alu: &Alu) -> bool {
    (alu.add.op != AddOp::Nop && !alu.add.magic)
        || (alu.mul.op != MulOp::Nop && !alu.mul.magic)
        || (alu.sig.writes_address() && !alu.sig_magic)
}

/// The peripherals of Mesa's `qpu_peripherals` (qpu_schedule.c:840-893),
/// one bit each, in its order.
const P_VPM_READ: u32 = 1 << 0;
const P_VPM_WRITE: u32 = 1 << 1;
const P_VPM_WAIT: u32 = 1 << 2;
const P_SFU: u32 = 1 << 3;
const P_TMU_WRITE: u32 = 1 << 4;
const P_TMU_READ: u32 = 1 << 5;
const P_TMU_WAIT: u32 = 1 << 6;
const P_WRTMUC: u32 = 1 << 7;
const P_TSY: u32 = 1 << 8;
const P_TLB_READ: u32 = 1 << 9;
const P_TLB_WRITE: u32 = 1 << 10;
const P_NAMES: [&str; 11] = [
    "VPM read",
    "VPM write",
    "VPM wait",
    "SFU",
    "TMU write",
    "LDTMU",
    "TMUWT",
    "WRTMUC",
    "TSY",
    "TLB read",
    "TLB write",
];

fn peripheral_names(p: u32) -> std::string::String {
    let n: Vec<&str> = (0..P_NAMES.len()).filter(|k| p & (1 << k) != 0).map(|k| P_NAMES[k]).collect();
    n.join(" + ")
}

/// Mesa's `qpu_peripherals` for a V3D 4.2 ALU instruction, transcribed with
/// its predicates (qpu_instr.c: v3d_qpu_reads_vpm, writes_vpm, waits_vpm,
/// writes_tmu with magic_waddr_is_tmu for ver >= 40, uses_sfu and
/// instr_is_legacy_sfu, reads_tlb, writes_tlb, magic_waddr_is_tsy). The SFU
/// add ops of V3D 7.x (BALLOT, ROT, ...) do not exist on 4.2.
fn peripherals(alu: &Alu) -> u32 {
    let magic = |op_nop: bool, magic: bool, waddr: u8, f: &dyn Fn(u8) -> bool| !op_nop && magic && f(waddr);
    let either = |f: &dyn Fn(u8) -> bool| {
        magic(alu.add.op == AddOp::Nop, alu.add.magic, alu.add.waddr, f)
            || magic(alu.mul.op == MulOp::Nop, alu.mul.magic, alu.mul.waddr, f)
    };
    let is_tmu = |w: u8| (11..=13).contains(&w) || (32..=46).contains(&w);
    let is_sfu = |w: u8| (19..=24).contains(&w);
    let mut p = 0;
    use AddOp::*;
    if matches!(alu.add.op, Vpmsetup | LdvpmvIn | LdvpmvOut | LdvpmdIn | LdvpmdOut | Ldvpmp | LdvpmgIn | LdvpmgOut) {
        p |= P_VPM_READ;
    }
    if matches!(alu.add.op, Vpmsetup | Stvpmv | Stvpmd | Stvpmp) || either(&|w| w == 14 || w == 15) {
        p |= P_VPM_WRITE;
    }
    if alu.add.op == Vpmwt {
        p |= P_VPM_WAIT;
    }
    if either(&is_tmu) {
        p |= P_TMU_WRITE;
    }
    if alu.sig.ldtmu {
        p |= P_TMU_READ;
    }
    if alu.sig.wrtmuc {
        p |= P_WRTMUC;
    }
    if matches!(alu.add.op, Recip | Rsqrt | Exp | Log | Sin | Rsqrt2) || either(&is_sfu) {
        p |= P_SFU;
    }
    if alu.sig.ldtlb || alu.sig.ldtlbu {
        p |= P_TLB_READ;
    }
    if either(&|w| w == 7 || w == 8) {
        p |= P_TLB_WRITE;
    }
    if magic(alu.add.op == AddOp::Nop, alu.add.magic, alu.add.waddr, &|w| (16..=18).contains(&w)) {
        p |= P_TSY;
    }
    if alu.add.op == Tmuwt {
        p |= P_TMU_WAIT;
    }
    p
}

/// Mesa's `qpu_compatible_peripheral_access` for V3D 4.2
/// (qpu_schedule.c:895-930), as it decides whether one instruction may hold
/// these peripherals: one at most, or a WRTMUC with a TMU write other than
/// TMUC (v3d_qpu_writes_tmu_not_tmuc), or an LDTMU with a VPM read or a VPM
/// write.
fn peripherals_compatible(p: u32, alu: &Alu) -> bool {
    if p.count_ones() <= 1 {
        return true;
    }
    if p == P_WRTMUC | P_TMU_WRITE {
        let tmuc = |op_nop: bool, magic: bool, waddr: u8| !op_nop && magic && waddr == 32;
        return !(tmuc(alu.add.op == AddOp::Nop, alu.add.magic, alu.add.waddr)
            || tmuc(alu.mul.op == MulOp::Nop, alu.mul.magic, alu.mul.waddr));
    }
    p == P_TMU_READ | P_VPM_READ || p == P_TMU_READ | P_VPM_WRITE
}

/// What an undefined operand lane computes on (1.0f32): see
/// [`Machine::operand`].
const UNDEF_STAND_IN: u32 = 0x3f80_0000;

fn unmodelled<T>(why: impl Into<String>) -> R<T> {
    Err(why.into())
}

/// A register a write can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reg {
    Acc(u8),
    Rf(u8),
}

struct Pending {
    at: usize,
    reg: Reg,
    vals: [u32; LANES],
    /// Lanes this write touches.
    mask: [bool; LANES],
    /// Touched lanes whose value is undefined (see [`Machine::undef_read`]).
    undef: [bool; LANES],
    sfu: bool,
}

/// f16 bits to f32 bits (exact).
fn f16_to_f32(h: u16) -> u32 {
    let s = ((h as u32) & 0x8000) << 16;
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as u32;
    if e == 0 {
        if m == 0 {
            return s;
        }
        // Subnormal: m * 2^-24, exact in f32.
        return s | (m as f32 * (1.0 / 16_777_216.0)).to_bits();
    }
    if e == 31 {
        return s | 0x7f80_0000 | (m << 13);
    }
    s | (((e - 15 + 127) as u32) << 23) | (m << 13)
}

/// f32 bits to f16 bits, rounding to nearest even. NaN is refused before
/// this is reached.
fn f32_to_f16(x: u32) -> u16 {
    let s = ((x >> 16) & 0x8000) as u16;
    let e = ((x >> 23) & 0xff) as i32;
    let m = x & 0x7f_ffff;
    if e == 0xff {
        return s | 0x7c00;
    }
    let ue = e - 127 + 15;
    if ue >= 31 {
        return s | 0x7c00;
    }
    if ue <= 0 {
        if ue < -10 {
            return s;
        }
        let mm = m | 0x80_0000;
        let shift = (14 - ue) as u32;
        let mut h = mm >> shift;
        let rem = mm & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && h & 1 == 1) {
            h += 1;
        }
        return s | h as u16;
    }
    let mut h = ((ue as u32) << 10) | (m >> 13);
    let rem = m & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && h & 1 == 1) {
        // A carry out of the mantissa rounds up into the exponent (and to
        // infinity at the top), which is what round-to-nearest gives.
        h += 1;
    }
    s | h as u16
}

/// Public for the tests that decode the TLB's f16 words.
pub fn half_to_f32(h: u16) -> f32 {
    f32::from_bits(f16_to_f32(h))
}

/// Public for the tests that predict the TLB's f16 words.
pub fn f32_to_half(x: f32) -> u16 {
    f32_to_f16(x.to_bits())
}

fn is_denormal(x: u32) -> bool {
    x & 0x7f80_0000 == 0 && x & 0x7f_ffff != 0
}

fn is_nan(x: u32) -> bool {
    x & 0x7f80_0000 == 0x7f80_0000 && x & 0x7f_ffff != 0
}

fn fl(x: u32) -> f32 {
    f32::from_bits(x)
}

/// A float operand after its input unpack.
fn f32_unpack(v: u32, u: Unpack, what: &str) -> R<u32> {
    let r = match u {
        Unpack::None => v,
        Unpack::Abs => v & 0x7fff_ffff,
        Unpack::L => f16_to_f32(v as u16),
        Unpack::H => f16_to_f32((v >> 16) as u16),
        other => return unmodelled(format!("{what}: input unpack {other:?}")),
    };
    if is_denormal(r) {
        return unmodelled(format!("{what}: a denormal operand"));
    }
    if is_nan(r) {
        return unmodelled(format!("{what}: a NaN operand"));
    }
    Ok(r)
}

/// A float result: refuse what the hardware's treatment is unknown for.
fn f32_result(r: f32, what: &str) -> R<u32> {
    let b = r.to_bits();
    if is_nan(b) {
        return unmodelled(format!("{what}: a NaN result"));
    }
    if is_denormal(b) {
        return unmodelled(format!("{what}: a denormal result"));
    }
    Ok(b)
}

/// The three flags an op can push, per lane; `None` = not described for it.
#[derive(Clone, Copy, Default)]
struct FlagVals {
    z: Option<bool>,
    n: Option<bool>,
    c: Option<bool>,
}

struct Machine<'c> {
    uniforms: &'c [u32],
    uni: usize,
    acc: [Lanes; 6],
    rf: [Lanes; 64],
    flag_a: [Option<bool>; LANES],
    flag_b: [Option<bool>; LANES],
    msf: [u32; LANES],
    pending: Vec<Pending>,
    /// Instructions executed so far (the clock).
    tick: usize,
    out: Outcome,
    fs: Option<FsInputs>,
    vs: Option<VsInputs>,
    next_varying: usize,
    vpm_written: Vec<bool>,
    env: &'c Env,
    /// The TMU request being set up: coordinates by magic address, and the
    /// configuration words WRTMUC took.
    tmu_coords: Vec<(u8, [u32; LANES])>,
    tmu_config: Vec<u32>,
    /// Returned words not yet taken by LDTMU, oldest first, each with the
    /// number of thread switches taken before its request.
    tmu_fifo: std::collections::VecDeque<([u32; LANES], usize)>,
    /// Thread switches taken so far (not the thread end).
    switches: usize,
    /// The TMUD data of a general memory write not yet addressed, with the
    /// lanes that wrote it.
    tmu_data: Option<([u32; LANES], [bool; LANES])>,
    /// Memory the program wrote through the TMU (spills), by byte address.
    scratch: std::collections::BTreeMap<u32, u32>,
    /// The UNIFA address and the tick it was written at.
    unifa: Option<(usize, u32)>,
    /// What MULTOP left in rtop for the next UMUL24.
    rtop: Rtop,
    /// A general TMU write not yet waited for by a TMUWT.
    tmu_write_unwaited: bool,
    /// The tick of the THRSW that ends the program, once it has run.
    thrend: Option<usize>,
    /// The lanes in which an operand read since the last reset was
    /// undefined: such a lane computes on a stand-in and its result is
    /// undefined, which is refused only where it is used (the module notes).
    undef_read: core::cell::Cell<[bool; LANES]>,
}

/// The rtop register between a MULTOP and the UMUL24 that reads it. Mesa
/// only ever emits the pair on the same operands, as a full 32-bit multiply
/// (v3d_compiler.h vir_UMUL, used for nir imul), and says UMUL24 reads rtop
/// and resets it to 0 (qpu_schedule.c:437-441); what rtop holds in between
/// is not stated, so the interpreter records the operands and models only
/// that pair: the low 32 bits of the product.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rtop {
    /// 0: UMUL24 is the 24x24 product.
    Zero,
    /// Set by a MULTOP on these operands.
    Set([u32; LANES], [u32; LANES]),
    /// Set, then a thread switch: Mesa's validator calls reading it a
    /// hazard ("cleared by a previous THRSW", qpu_validate.c:182).
    Lost,
}

impl Machine<'_> {
    fn next_uniform(&mut self, what: &str) -> R<u32> {
        let Some(&v) = self.uniforms.get(self.uni) else {
            return unmodelled(format!("{what}: the uniform stream has no value {}", self.uni));
        };
        self.uni += 1;
        self.out.uniforms_read += 1;
        Ok(v)
    }

    fn reg(&self, r: Reg) -> &Lanes {
        match r {
            Reg::Acc(n) => &self.acc[n as usize],
            Reg::Rf(n) => &self.rf[n as usize],
        }
    }

    /// One operand, every lane, before its unpack.
    fn operand(&self, alu: &Alu, mux: Mux, what: &str) -> R<[u32; LANES]> {
        let reg = match mux {
            Mux::A => Reg::Rf(alu.raddr_a),
            Mux::B if alu.sig.small_imm_b => {
                let Some(v) = alu.small_imm() else {
                    return unmodelled(format!("{what}: small immediate {} past the table", alu.raddr_b));
                };
                return Ok([v; LANES]);
            }
            Mux::B => Reg::Rf(alu.raddr_b),
            m => Reg::Acc(m as u8),
        };
        if self.pending.iter().any(|p| p.sfu && p.reg == reg && p.at > self.tick) {
            return unmodelled(format!("{what}: reads {reg:?} before its SFU result lands"));
        }
        let mut out = [0u32; LANES];
        let mut undef = self.undef_read.get();
        for (l, (o, v)) in out.iter_mut().zip(self.reg(reg).iter()).enumerate() {
            match v {
                Some(x) => *o = *x,
                None => {
                    // A stand-in no operation refuses (1.0: no NaN, no
                    // overflow, no denormal); the lane's result is marked
                    // undefined and never compared or stored as a value.
                    *o = UNDEF_STAND_IN;
                    undef[l] = true;
                }
            }
        }
        self.undef_read.set(undef);
        Ok(out)
    }

    /// One operand as it is, undefined lanes included: only for a MOV
    /// straight to the TLB (see [`TlbWrite`]).
    fn operand_lanes(&self, alu: &Alu, mux: Mux, what: &str) -> R<Lanes> {
        match mux {
            Mux::A => Ok(self.rf[alu.raddr_a as usize]),
            Mux::B if !alu.sig.small_imm_b => Ok(self.rf[alu.raddr_b as usize]),
            Mux::B => self.operand(alu, mux, what).map(|v| v.map(Some)),
            m => Ok(self.acc[m as usize]),
        }
    }

    /// The lanes `cond` selects, from the flags before this instruction.
    fn cond_mask(&self, cond: Cond, what: &str) -> R<[bool; LANES]> {
        let (flag, want) = match cond {
            Cond::None => return Ok([true; LANES]),
            Cond::IfA => (&self.flag_a, true),
            Cond::IfNA => (&self.flag_a, false),
            Cond::IfB => (&self.flag_b, true),
            Cond::IfNB => (&self.flag_b, false),
        };
        let mut m = [false; LANES];
        for (l, (sel, f)) in m.iter_mut().zip(flag.iter()).enumerate() {
            match f {
                Some(f) => *sel = *f == want,
                None => return unmodelled(format!("{what}: {cond:?} on an undefined flag in lane {l}")),
            }
        }
        Ok(m)
    }

    /// The lanes `cond` selects, and the lanes whose flag is undefined (in
    /// which it is unknown whether the write happens, so the destination
    /// lane becomes undefined): for a register write. A sink uses
    /// [`Machine::cond_mask`], which refuses instead.
    fn cond_lanes(&self, cond: Cond) -> ([bool; LANES], [bool; LANES]) {
        let (flag, want) = match cond {
            Cond::None => return ([true; LANES], [false; LANES]),
            Cond::IfA => (&self.flag_a, true),
            Cond::IfNA => (&self.flag_a, false),
            Cond::IfB => (&self.flag_b, true),
            Cond::IfNB => (&self.flag_b, false),
        };
        let mut m = [false; LANES];
        let mut unknown = [false; LANES];
        for l in 0..LANES {
            match flag[l] {
                Some(f) => m[l] = f == want,
                None => unknown[l] = true,
            }
        }
        (m, unknown)
    }

    /// Queue a register write: `mask` the lanes it touches, `undef` those of
    /// them whose value is undefined.
    fn schedule(
        &mut self,
        at: usize,
        reg: Reg,
        vals: [u32; LANES],
        mask: [bool; LANES],
        undef: [bool; LANES],
        sfu: bool,
    ) {
        self.pending.push(Pending { at, reg, vals, mask, undef, sfu });
    }

    /// Land every write due now. Two on one lane of one register at once are
    /// unmodelled.
    fn land(&mut self) -> R<()> {
        let tick = self.tick;
        let (due, later): (Vec<Pending>, Vec<Pending>) = self.pending.drain(..).partition(|p| p.at <= tick);
        self.pending = later;
        for (i, p) in due.iter().enumerate() {
            if due[i + 1..].iter().any(|q| q.reg == p.reg && p.mask.iter().zip(q.mask.iter()).any(|(a, b)| *a && *b)) {
                return unmodelled(format!("two writes land on {:?} in one cycle", p.reg));
            }
            let dst = match p.reg {
                Reg::Acc(n) => &mut self.acc[n as usize],
                Reg::Rf(n) => &mut self.rf[n as usize],
            };
            for (d, ((v, on), u)) in dst.iter_mut().zip(p.vals.iter().zip(p.mask.iter()).zip(p.undef.iter())) {
                if *on {
                    *d = if *u { None } else { Some(*v) };
                }
            }
        }
        Ok(())
    }

    /// One LDUNIFA read: the word at the UNIFA address, which then moves on
    /// by 4 bytes.
    fn ldunifa(&mut self, what: &str) -> R<u32> {
        let Some((at, addr)) = self.unifa else {
            return unmodelled(format!("{what}: LDUNIFA with no UNIFA address written"));
        };
        if self.tick <= at + 3 {
            return unmodelled(format!("{what}: LDUNIFA {} instruction(s) after the UNIFA write", self.tick - at));
        }
        if addr % 4 != 0 {
            return unmodelled(format!("{what}: LDUNIFA at the unaligned address {addr:#x}"));
        }
        let word = self.env.memory.iter().find_map(|r| {
            let off = addr.checked_sub(r.base)? as usize / 4;
            r.words.get(off).copied()
        });
        let Some(Some(v)) = word else {
            return unmodelled(format!("{what}: LDUNIFA at {addr:#x}, where no defined word is"));
        };
        self.unifa = Some((at, addr.wrapping_add(4)));
        Ok(v)
    }

    /// A write to a TMU magic register: a coordinate, or S, which retires
    /// the request and queues its result words.
    fn tmu_write(&mut self, waddr: u8, vals: [u32; LANES], mask: [bool; LANES], what: &str) -> R<()> {
        if !matches!(waddr, 11..=13) && mask.iter().any(|x| !x) {
            return unmodelled(format!("{what}: a conditional TMU write"));
        }
        match waddr {
            // TMUD: the data of a general memory write.
            11 => {
                if self.tmu_data.is_some() {
                    return unmodelled(format!("{what}: a general TMU write of more than one word"));
                }
                self.tmu_data = Some((vals, mask));
                Ok(())
            }
            // TMUA, TMUAU: the address, which retires a general access.
            12 | 13 => self.tmu_general(waddr, vals, mask, what),
            // TMUT, TMUR (a cube's third coordinate), TMUI (a 2D array's
            // layer), TMUB (a bias, or an explicit LOD with TMUSLOD or P2's
            // disable_autolod).
            34..=37 => {
                self.tmu_coords.retain(|(w, _)| *w != waddr);
                self.tmu_coords.push((waddr, vals));
                Ok(())
            }
            // TMUS, TMUSCM, TMUSF, TMUSLOD: S, the lookup (v3d_tex.c:370-381).
            33 | 40 | 41 | 42 => self.tmu_lookup(waddr, vals, what),
            w => unmodelled(format!("{what}: a write to TMU register {}", magic_waddr_name(w).unwrap_or("?"))),
        }
    }

    /// A general memory access, as Mesa's spills and fills make it
    /// (vir_register_allocate.c:498-611): one 32-bit word per lane at the
    /// lane's byte address, a write when TMUD came first and a read (its
    /// word queued for LDTMU) otherwise. The configuration is TMUA's
    /// default (0xffffffff: per-pixel, the regular op, 32-bit) or the
    /// per-quad one TMUAU takes from the stream (0xffffff7f); the field
    /// layout is nir_to_vir.c:43-52 and 640-670. Anything else is refused.
    fn tmu_general(&mut self, waddr: u8, addr: [u32; LANES], mask: [bool; LANES], what: &str) -> R<()> {
        let config = if waddr == 13 { self.next_uniform(what)? } else { 0xffff_ffff };
        if config != 0xffff_ffff && config != 0xffff_ff7f {
            return unmodelled(format!("{what}: general TMU configuration {config:#010x}"));
        }
        if !self.tmu_coords.is_empty() || !self.tmu_config.is_empty() {
            return unmodelled(format!("{what}: a general TMU access inside a texture request"));
        }
        // Per-quad acts on every lane of a quad with one lane active: with
        // every lane active it is the same as per-pixel.
        if config & 0x80 == 0 && mask.iter().any(|x| !x) {
            return unmodelled(format!("{what}: a per-quad general TMU access under a condition"));
        }
        for l in (0..LANES).filter(|&l| mask[l]) {
            if !addr[l].is_multiple_of(4) {
                return unmodelled(format!("{what}: a general TMU address {:#x} not 4-byte aligned", addr[l]));
            }
        }
        match self.tmu_data.take() {
            Some((data, dmask)) => {
                if dmask != mask {
                    return unmodelled(format!("{what}: TMUD and the address written under different conditions"));
                }
                for l in (0..LANES).filter(|&l| mask[l]) {
                    self.scratch.insert(addr[l], data[l]);
                }
                self.tmu_write_unwaited = true;
            }
            None => {
                if mask.iter().any(|x| !x) {
                    return unmodelled(format!("{what}: a conditional general TMU read"));
                }
                let mut word = [0; LANES];
                for l in 0..LANES {
                    word[l] = match self.scratch.get(&addr[l]) {
                        Some(v) => *v,
                        None => {
                            return unmodelled(format!("{what}: a TMU read of {:#x}, which nothing wrote", addr[l]))
                        }
                    };
                }
                self.tmu_fifo.push_back((word, self.switches));
                self.tmu_fifo_bound(what)?;
            }
        }
        Ok(())
    }

    /// The TMU output FIFO may not overflow: it cannot stall, unlike the
    /// input and config FIFOs, and holds 16 / threads words per thread
    /// (nir_to_vir.c:243-258, v3d_compiler.h:44-62).
    fn tmu_fifo_bound(&self, what: &str) -> R<()> {
        if self.env.threads == 0 {
            return unmodelled(format!("{what}: a TMU look-up with no thread count given"));
        }
        let cap = (16 / self.env.threads) as usize;
        if self.tmu_fifo.len() > cap {
            return unmodelled(format!(
                "{what}: {} TMU results outstanding, past the output FIFO's {cap} (16 / {} threads)",
                self.tmu_fifo.len(),
                self.env.threads
            ));
        }
        Ok(())
    }

    fn tmu_lookup(&mut self, retire: u8, s: [u32; LANES], what: &str) -> R<()> {
        let config = core::mem::take(&mut self.tmu_config);
        let coords = core::mem::take(&mut self.tmu_coords);
        let Some(&p0) = config.first() else {
            return unmodelled(format!("{what}: a TMU lookup with no configuration word"));
        };
        let out32 = config.get(1).is_some_and(|p1| p1 & 1 != 0);
        if config.get(1).is_some_and(|p1| p1 & 2 != 0) {
            return unmodelled(format!("{what}: unnormalised texture coordinates"));
        }
        // P2: a regular lookup (op V3D_TMU_OP_REGULAR = 15 in bits 20..23,
        // v3d_packet.xml "TMU Op"; Mesa emits P2 only when it differs from
        // that default, v3d_tex.c:68-70, 303-306), no gather, coefficient
        // mode, sample number, R offset or LOD query; disable_autolod (bit 1)
        // makes TMUB the level of detail (v3d_tex.c:137-153); "Offset S"
        // (bits 8..11) and "Offset T" (bits 12..15) are a constant texel
        // offset, 4-bit two's complement each (v3d_tex.c:161-173, Mesa's
        // packing of a constant nir_tex_src_offset).
        let mut disable_autolod = false;
        let mut offset = [0i32; 2];
        if let Some(&p2) = config.get(2) {
            if p2 & !(0x2 | 0xff00) != 0x00f0_0000 {
                return unmodelled(format!("{what}: TMU P2 {p2:#010x} is not a regular lookup"));
            }
            disable_autolod = p2 & 0x2 != 0;
            let nibble = |shift: u32| ((p2 >> shift) & 0xf) as i32 - if p2 >> (shift + 3) & 1 != 0 { 16 } else { 0 };
            offset = [nibble(8), nibble(12)];
        }
        if config.len() > 3 {
            return unmodelled(format!("{what}: {} TMU configuration words", config.len()));
        }
        let coord = |w: u8| coords.iter().find(|(x, _)| *x == w).map(|(_, v)| *v);
        let Some(t) = coord(34) else {
            return unmodelled(format!("{what}: a 1D texture lookup (no T coordinate)"));
        };
        let cube = retire == 40;
        // TMUSF: a texel fetch (v3d_tex.c:372-374), S and T integer texel
        // coordinates and TMUB the integer level; no filter, no wrap, and no
        // automatic level to disable.
        let fetch = retire == 41;
        if fetch && disable_autolod {
            return unmodelled(format!("{what}: a texel fetch with disable_autolod"));
        }
        if cube && offset != [0; 2] {
            return unmodelled(format!("{what}: a cube lookup with a texel offset"));
        }
        let r = coord(35);
        if cube != r.is_some() {
            return unmodelled(format!(
                "{what}: {}",
                if cube {
                    "a cube lookup with no R coordinate"
                } else {
                    "a 3D lookup (an R coordinate without TMUSCM)"
                }
            ));
        }
        // TMUI: a 2D array's layer (Mesa's `is_array` write); with TMUSCM
        // it would be a cube array, which GL ES 2.0 and 3.0 have not.
        let layer = coord(36);
        if cube && layer.is_some() {
            return unmodelled(format!("{what}: a cube lookup with an array index (TMUI)"));
        }
        // The level of detail: TMUSLOD takes it from TMUB (and implies
        // disable_autolod); so does a cube lookup with disable_autolod
        // (v3d_tex.c:137-153, 375-379). Otherwise it comes from the
        // derivatives, and TMUB is a bias on it.
        let b = coord(37);
        let explicit = retire == 42 || disable_autolod;
        if (explicit || fetch) && b.is_none() {
            return unmodelled(format!("{what}: an explicit level of detail with no TMUB write"));
        }
        if fetch && layer.is_some() {
            return unmodelled(format!("{what}: a texel fetch of a 2D array (TMUI)"));
        }
        let idx = (p0 >> 24) as usize;
        let Some(tex) = self.env.textures.get(idx) else {
            return unmodelled(format!("{what}: texture {idx} not supplied"));
        };
        // A cube texture is also six layers (the V3D record is the same):
        // a cube lookup needs a cube texture, a 2D lookup a 2D one, and a
        // 2D-array lookup reads a face of a cube or layer 0 of a 2D texture.
        if cube != tex.cube && layer.is_none() {
            return unmodelled(format!(
                "{what}: a {} lookup on {} texture {idx}",
                if cube { "cube" } else { "2D" },
                if tex.cube { "cube" } else { "2D" }
            ));
        }
        // The sampler P1 names (bits 24..31), or the texture's own
        // NEAREST / CLAMP_TO_EDGE one.
        let sampler = config
            .get(1)
            .and_then(|p1| self.env.samplers.get((p1 >> 24) as usize))
            .copied()
            .unwrap_or_else(|| Sampler::of(tex));
        let integer = idx < 32 && self.env.integer_textures & (1 << idx) != 0;
        let nearest = sampler.mag == texref::Filter::Nearest
            && matches!(sampler.min, texref::MinFilter::Nearest | texref::MinFilter::NearestMipmapNearest);
        if integer && !nearest && !fetch {
            return unmodelled(format!("{what}: an integer texture {idx} filtered other than NEAREST"));
        }
        let mask = p0 & 0xf;
        let mut words: Vec<[u32; LANES]> = Vec::new();
        for k in 0..4 {
            if mask & (1 << k) != 0 {
                if !out32 && k >= 2 {
                    return unmodelled(format!("{what}: 16-bit TMU results ask for word {k}"));
                }
                words.push([0; LANES]);
            }
        }
        use crate::shaderc::online::texref::{self, Lod};
        for l in 0..LANES {
            let refused = |e: texref::Refusal| format!("{what}: {e}");
            /* A texel fetch (GLSL ES 3.00 8.8 texelFetch): the texel at the
             * integer coordinates plus the offset, of the integer level in
             * TMUB; a level or a texel outside the texture is undefined in
             * GL (no wrap applies) and refused here. */
            if fetch {
                let level = b.map_or(0, |b| b[l] as i32);
                let Some(lv) = usize::try_from(level).ok().and_then(|k| tex.levels.get(k)) else {
                    return unmodelled(format!(
                        "{what}: a texel fetch of level {level} on a texture of {} levels",
                        tex.levels.len()
                    ));
                };
                self.out.levels |= 1 << level;
                let (i, j) = (s[l] as i32 + offset[0], t[l] as i32 + offset[1]);
                if !(0 <= i && i < lv.width as i32 && 0 <= j && j < lv.height as i32) {
                    return unmodelled(format!(
                        "{what}: a texel fetch of ({i}, {j}) outside level {level} ({} x {})",
                        lv.width, lv.height
                    ));
                }
                let texel = lv.faces[0][(j as u32 * lv.width + i as u32) as usize];
                pack_words(&mut words, mask, integer, out32, l, texel);
                continue;
            }
            let lod = match b {
                Some(b) if explicit => Lod::Explicit(fl(b[l])),
                _ => Lod::Implicit,
            };
            let (level, second) = texref::levels(tex, sampler.min, lod).map_err(refused)?;
            self.out.levels |= 1 << level;
            if let Some((l1, _)) = second {
                self.out.levels |= 1 << l1;
            }
            /* The texel offset is in texels of the level read (GLSL ES 3.00
             * 8.8: "added to the (u, v, w) texel coordinates before looking
             * up each texel"): a shift of the normalised coordinate by
             * offset / size of that level. Under a mipmap-LINEAR blend the
             * two levels would need two shifts: not modelled. */
            let (mut s, mut t) = (fl(s[l]), fl(t[l]));
            if offset != [0; 2] {
                if second.is_some() {
                    return unmodelled(format!("{what}: a texel offset under a mipmap-LINEAR blend"));
                }
                let lv = &tex.levels[level];
                s += offset[0] as f32 / lv.width as f32;
                t += offset[1] as f32 / lv.height as f32;
            }
            let texel = match (r, layer) {
                (Some(r), _) => {
                    let (v, face) =
                        texref::sample_cube_filtered(tex, &sampler, s, t, fl(r[l]), lod).map_err(refused)?;
                    self.out.cube_faces |= 1 << face.index();
                    v
                }
                (None, Some(i)) => {
                    let f = fl(i[l]);
                    if !f.is_finite() {
                        return Err(refused(texref::Refusal::LayerNotFinite));
                    }
                    /* floor(layer + 0.5), clamped to the layers there are */
                    let faces = tex.levels.first().map_or(1, |lv| lv.faces.len());
                    let layer = ((f + 0.5).floor().max(0.0) as usize).min(faces - 1);
                    if tex.cube {
                        self.out.cube_faces |= 1 << layer;
                    }
                    texref::sample_layer(tex, &sampler, layer, s, t, lod).map_err(refused)?
                }
                (None, None) => texref::sample_layer(tex, &sampler, 0, s, t, lod).map_err(refused)?,
            };
            pack_words(&mut words, mask, integer, out32, l, texel);
        }
        self.tmu_fifo.extend(words.into_iter().map(|w| (w, self.switches)));
        self.out.tmu_lookups += 1;
        self.tmu_fifo_bound(what)
    }

    /// A thread switch: the accumulators are gone, and a write still in
    /// flight to an accumulator lands in the other thread. The flags stay
    /// (V3D 4.2, see the module notes).
    fn thread_switch(&mut self) {
        self.switches += 1;
        self.acc = [[None; LANES]; 6];
        self.pending.retain(|p| !matches!(p.reg, Reg::Acc(_)));
        if self.rtop != Rtop::Zero {
            self.rtop = Rtop::Lost;
        }
    }
}

/// What an ALU half computes, before its output pack, with its flags.
struct Computed {
    vals: [u32; LANES],
    flags: [FlagVals; LANES],
    sfu: Option<&'static str>,
    float: bool,
}

impl Computed {
    fn new(float: bool) -> Computed {
        Computed { vals: [0; LANES], flags: [FlagVals::default(); LANES], sfu: None, float }
    }
}

/// Lane `l` of a look-up's `texel` into the result words P0's `mask` asked
/// for, in order: a 32-bit word a channel at the 32-bit output (`out32`),
/// else two f16 channels a word (r | g << 16, b | a << 16); an `integer`
/// texture's channels are integers held as floats, returned raw (the low 16
/// bits each at the 16-bit output).
fn pack_words(words: &mut [[u32; LANES]], mask: u32, integer: bool, out32: bool, l: usize, texel: [f32; 4]) {
    let mut n = 0;
    for k in 0..4 {
        if mask & (1 << k) == 0 {
            continue;
        }
        /* an integer held as a float, to its two's complement */
        let int = |v: f32| if v >= 2_147_483_648.0 { v as u32 } else { v as i32 as u32 };
        words[n][l] = if integer && out32 {
            int(texel[k])
        } else if integer {
            int(texel[2 * k]) & 0xffff | int(texel[2 * k + 1]) << 16
        } else if out32 {
            texel[k].to_bits()
        } else {
            let (a, b) = (texel[2 * k], texel[2 * k + 1]);
            f32_to_f16(a.to_bits()) as u32 | (f32_to_f16(b.to_bits()) as u32) << 16
        };
        n += 1;
    }
}

fn no_unpack(ua: Unpack, ub: Unpack, what: &str) -> R<()> {
    if ua != Unpack::None || ub != Unpack::None {
        return unmodelled(format!("{what}: an unpack on an integer op"));
    }
    Ok(())
}

fn uniform_row(row: &[u32; LANES], what: &str) -> R<usize> {
    if row.iter().any(|r| *r != row[0]) {
        return unmodelled(format!("{what}: a VPM row that differs between lanes"));
    }
    Ok(row[0] as usize)
}

/// The add ALU.
fn add_op(m: &mut Machine, alu: &Alu) -> R<Option<Computed>> {
    use AddOp::*;
    let op = alu.add.op;
    let what = format!("add {op:?}");
    let n = num_src(add_op_args(op));
    let a = if n >= 1 { m.operand(alu, alu.add.a.mux, &what)? } else { [0; LANES] };
    let b = if n >= 2 { m.operand(alu, alu.add.b.mux, &what)? } else { [0; LANES] };
    let (ua, ub) = (alu.add.a.unpack, alu.add.b.unpack);
    let mut c;
    match op {
        Nop => return Ok(None),
        Fadd | Faddnf | Fsub | Fmin | Fmax | Fcmp => {
            if op == Fcmp && !(alu.add.magic && alu.add.waddr == WADDR_NOP) {
                return unmodelled("add Fcmp: writes a register (its value is not described)");
            }
            c = Computed::new(true);
            for l in 0..LANES {
                let x = fl(f32_unpack(a[l], ua, &what)?);
                let y = fl(f32_unpack(b[l], ub, &what)?);
                c.vals[l] = match op {
                    Fadd | Faddnf => f32_result(x + y, &what)?,
                    Fsub => f32_result(x - y, &what)?,
                    Fmin => f32_result(x.min(y), &what)?,
                    Fmax => f32_result(x.max(y), &what)?,
                    _ => {
                        c.flags[l] = FlagVals { z: Some(x == y), n: Some(x < y), c: Some(x <= y) };
                        0
                    }
                };
            }
        }
        Vfpack => {
            c = Computed::new(false);
            for l in 0..LANES {
                let x = f32_unpack(a[l], ua, &what)?;
                let y = f32_unpack(b[l], ub, &what)?;
                c.vals[l] = f32_to_f16(x) as u32 | (f32_to_f16(y) as u32) << 16;
            }
        }
        Ffloor | Fceil | Ftrunc | Fround => {
            c = Computed::new(true);
            for (v, x) in c.vals.iter_mut().zip(a.iter()) {
                let x = fl(f32_unpack(*x, ua, &what)?);
                let r = match op {
                    Ffloor => x.floor(),
                    Fceil => x.ceil(),
                    Ftrunc => x.trunc(),
                    _ => x.round_ties_even(),
                };
                *v = f32_result(r, &what)?;
            }
        }
        Ftoiz | Ftoin | Ftouz => {
            c = Computed::new(false);
            for (v, x) in c.vals.iter_mut().zip(a.iter()) {
                let x = fl(f32_unpack(*x, ua, &what)?);
                let r = if op == Ftoin { x.round_ties_even() } else { x.trunc() };
                *v = if op == Ftouz {
                    if !(0.0..4_294_967_296.0).contains(&r) {
                        return unmodelled(format!("{what}: {x} is out of the u32 range"));
                    }
                    r as u32
                } else {
                    if !(-2_147_483_648.0..2_147_483_648.0).contains(&r) {
                        return unmodelled(format!("{what}: {x} is out of the i32 range"));
                    }
                    r as i32 as u32
                };
            }
        }
        Itof | Utof => {
            no_unpack(ua, Unpack::None, &what)?;
            c = Computed::new(true);
            for (v, x) in c.vals.iter_mut().zip(a.iter()) {
                *v = if op == Itof { (*x as i32 as f32).to_bits() } else { (*x as f32).to_bits() };
            }
        }
        Recip | Rsqrt | Exp | Log | Sin => {
            c = Computed::new(true);
            c.sfu = Some(match op {
                Recip => "recip",
                Rsqrt => "rsqrt",
                Exp => "exp",
                Log => "log",
                _ => "sin",
            });
            for (v, x) in c.vals.iter_mut().zip(a.iter()) {
                let x = fl(f32_unpack(*x, ua, &what)?);
                let r = match op {
                    Recip => 1.0 / x,
                    Rsqrt => 1.0 / x.sqrt(),
                    Exp => x.exp2(),
                    Log => x.log2(),
                    _ => {
                        if !(-0.5..=0.5).contains(&x) {
                            return unmodelled(format!("{what}: {x} outside the reduced range [-0.5, 0.5]"));
                        }
                        (x * core::f32::consts::PI).sin()
                    }
                };
                *v = f32_result(r, &what)?;
            }
        }
        Add | Sub | And | Or | Xor | Min | Max | Umin | Umax | Shl | Shr | Asr | Ror => {
            no_unpack(ua, ub, &what)?;
            c = Computed::new(false);
            for l in 0..LANES {
                let (x, y) = (a[l], b[l]);
                c.vals[l] = match op {
                    Add => x.wrapping_add(y),
                    Sub => x.wrapping_sub(y),
                    And => x & y,
                    Or => x | y,
                    Xor => x ^ y,
                    Min => (x as i32).min(y as i32) as u32,
                    Max => (x as i32).max(y as i32) as u32,
                    Umin => x.min(y),
                    Umax => x.max(y),
                    Shl => x.wrapping_shl(y & 31),
                    Shr => x.wrapping_shr(y & 31),
                    Asr => (x as i32).wrapping_shr(y & 31) as u32,
                    _ => x.rotate_right(y & 31),
                };
                c.flags[l] = match op {
                    Xor => FlagVals { z: Some(c.vals[l] == 0), ..Default::default() },
                    Min => FlagVals { c: Some((x as i32) > (y as i32)), ..Default::default() },
                    Sub => FlagVals { c: Some(x < y), ..Default::default() },
                    _ => FlagVals::default(),
                };
            }
        }
        Not | Neg | Clz => {
            no_unpack(ua, Unpack::None, &what)?;
            c = Computed::new(false);
            for (v, x) in c.vals.iter_mut().zip(a.iter()) {
                *v = match op {
                    Not => !*x,
                    Neg => (*x as i32).wrapping_neg() as u32,
                    _ => x.leading_zeros(),
                };
            }
        }
        // 0 for a front-facing primitive, 1 for a back-facing one
        // (nir_to_vir.c:3692-3699).
        Revf => {
            let Some(fs) = &m.fs else { return unmodelled(format!("{what} outside a fragment shader")) };
            c = Computed::new(false);
            c.vals = fs.back_facing.map(u32::from);
        }
        // The lane's element index, 0..15 (the spill base's EIDX << 2,
        // vir_register_allocate.c:449-450).
        Eidx => {
            c = Computed::new(false);
            c.vals = core::array::from_fn(|l| l as u32);
        }
        // The QPU thread (Env::thread), the same in every lane.
        Tidx => {
            c = Computed::new(false);
            c.vals = [m.env.thread; LANES];
        }
        Fxcd | Fycd => {
            let Some(fs) = &m.fs else { return unmodelled(format!("{what} outside a fragment shader")) };
            c = Computed::new(true);
            c.vals = if op == Fxcd { fs.x } else { fs.y };
        }
        Msf => {
            c = Computed::new(false);
            c.vals = m.msf;
            for (f, v) in c.flags.iter_mut().zip(c.vals.iter()) {
                f.z = Some(*v == 0);
            }
        }
        Setmsf => {
            // A sink: the multisample flags decide what is discarded.
            if m.undef_read.get().iter().any(|u| *u) {
                return unmodelled(format!("{what}: an undefined lane reaches the multisample flags"));
            }
            let mask = m.cond_mask(alu.flags.ac, &what)?;
            for ((f, v), on) in m.msf.iter_mut().zip(a.iter()).zip(mask.iter()) {
                if *on {
                    *f = *v;
                }
            }
            return Ok(None);
        }
        LdvpmvIn => {
            if m.undef_read.get().iter().any(|u| *u) {
                return unmodelled(format!("{what}: an undefined lane names a VPM row"));
            }
            let r = uniform_row(&a, &what)?;
            let Some(vs) = &m.vs else { return unmodelled(format!("{what} outside a vertex shader")) };
            if m.vpm_written.get(r).copied().unwrap_or(false) {
                return unmodelled(format!("{what}: reads VPM row {r} after it was written"));
            }
            let Some(v) = vs.vpm.get(r) else { return unmodelled(format!("{what}: VPM input row {r} not supplied")) };
            c = Computed::new(false);
            c.vals = *v;
        }
        Stvpmv => {
            if alu.flags.ac != Cond::None {
                return unmodelled(format!("{what}: a conditional VPM store"));
            }
            // A sink: the VPM row and value are the vertex shader's output.
            if m.undef_read.get().iter().any(|u| *u) {
                return unmodelled(format!("{what}: an undefined lane reaches a VPM store"));
            }
            let r = uniform_row(&a, &what)?;
            if r >= 256 {
                return unmodelled(format!("{what}: VPM row {r}"));
            }
            if m.out.vpm_out.len() <= r {
                m.out.vpm_out.resize(r + 1, None);
                m.vpm_written.resize(r + 1, false);
            }
            if m.out.vpm_out[r].is_some() {
                return unmodelled(format!("{what}: VPM row {r} written twice"));
            }
            m.out.vpm_out[r] = Some(b);
            m.vpm_written[r] = true;
            return Ok(None);
        }
        Vpmwt => return Ok(None),
        Tmuwt => {
            m.tmu_write_unwaited = false;
            return Ok(None);
        }
        /* A difference across a 2x2 quad: which lanes form a quad on the
         * V3D is not modelled (no public source states it, and Mesa's v3d
         * has no quad operation that would show it), so a run that reaches
         * one is unmeasured here; gl_test measures derivatives on the
         * board. */
        Fdx | Fdy if m.env.quads => {
            c = Computed::new(true);
            let mut x = [0f32; LANES];
            for (l, v) in x.iter_mut().enumerate() {
                *v = fl(f32_unpack(a[l], ua, &what)?);
            }
            for l in 0..LANES {
                let (q, i) = (l & !3, l & 3);
                let (hi, lo) = if op == Fdx { (q + (i & 2) + 1, q + (i & 2)) } else { (q + 2 + (i & 1), q + (i & 1)) };
                c.vals[l] = f32_result(x[hi] - x[lo], &what)?;
            }
        }
        Fdx | Fdy => return unmodelled(format!("add {op:?}: the quad lane layout is not modelled (Env::quads)")),
        other => return unmodelled(format!("add {other:?}")),
    }
    Ok(Some(c))
}

/// The mul ALU.
fn mul_op(m: &mut Machine, alu: &Alu) -> R<Option<Computed>> {
    let op = alu.mul.op;
    let what = format!("mul {op:?}");
    let n = num_src(mul_op_args(op));
    let a = if n >= 1 { m.operand(alu, alu.mul.a.mux, &what)? } else { [0; LANES] };
    let b = if n >= 2 { m.operand(alu, alu.mul.b.mux, &what)? } else { [0; LANES] };
    let (ua, ub) = (alu.mul.a.unpack, alu.mul.b.unpack);
    let mut c;
    match op {
        MulOp::Nop => return Ok(None),
        MulOp::Fmul => {
            c = Computed::new(true);
            for l in 0..LANES {
                let x = fl(f32_unpack(a[l], ua, &what)?);
                let y = fl(f32_unpack(b[l], ub, &what)?);
                c.vals[l] = f32_result(x * y, &what)?;
            }
        }
        MulOp::Fmov => {
            c = Computed::new(true);
            for ((v, f), x) in c.vals.iter_mut().zip(c.flags.iter_mut()).zip(a.iter()) {
                let x = f32_unpack(*x, ua, &what)?;
                *v = x;
                *f = FlagVals { z: Some(fl(x) == 0.0), n: Some(fl(x) < 0.0), c: None };
            }
        }
        MulOp::Mov => {
            c = Computed::new(false);
            for ((v, f), x) in c.vals.iter_mut().zip(c.flags.iter_mut()).zip(a.iter()) {
                *v = *x;
                f.z = Some(*x == 0);
            }
        }
        MulOp::Multop => {
            no_unpack(ua, ub, &what)?;
            if alu.flags.mc != Cond::None || alu.flags.mpf != Pf::None || alu.flags.muf != Uf::None {
                return unmodelled(format!("{what}: a conditional MULTOP or one that sets flags"));
            }
            if m.rtop != Rtop::Zero {
                return unmodelled(format!("{what}: a MULTOP while rtop is still set"));
            }
            m.rtop = Rtop::Set(a, b);
            return Ok(None);
        }
        MulOp::Umul24 if m.rtop != Rtop::Zero => {
            no_unpack(ua, ub, &what)?;
            let Rtop::Set(ra, rb) = m.rtop else {
                return unmodelled(format!("{what}: UMUL24 reads rtop after a thread switch"));
            };
            if (ra, rb) != (a, b) || alu.flags.mc != Cond::None {
                return unmodelled(format!("{what}: UMUL24 after a MULTOP on other operands, or conditional"));
            }
            m.rtop = Rtop::Zero;
            c = Computed::new(false);
            for l in 0..LANES {
                c.vals[l] = a[l].wrapping_mul(b[l]);
            }
        }
        MulOp::Add | MulOp::Sub | MulOp::Umul24 | MulOp::Smul24 => {
            no_unpack(ua, ub, &what)?;
            c = Computed::new(false);
            for l in 0..LANES {
                let (x, y) = (a[l], b[l]);
                c.vals[l] = match op {
                    MulOp::Add => x.wrapping_add(y),
                    MulOp::Sub => x.wrapping_sub(y),
                    MulOp::Umul24 => (x & 0xff_ffff).wrapping_mul(y & 0xff_ffff),
                    _ => ((((x << 8) as i32) >> 8).wrapping_mul(((y << 8) as i32) >> 8)) as u32,
                };
                if op == MulOp::Sub {
                    c.flags[l].c = Some(x < y);
                }
            }
        }
        other => return unmodelled(format!("mul {other:?}")),
    }
    Ok(Some(c))
}

/// An output pack: to f16 into one half of the destination, keeping the
/// other half's current value.
/// A lane whose other half is undefined becomes undefined in `undef`.
fn output_pack(m: &Machine, pack: OutPack, reg: Reg, vals: &mut [u32; LANES], undef: &mut [bool; LANES]) {
    if pack == OutPack::None {
        return;
    }
    for (l, (v, o)) in vals.iter_mut().zip(m.reg(reg).iter()).enumerate() {
        let Some(o) = o else {
            undef[l] = true;
            continue;
        };
        let h = f32_to_f16(*v) as u32;
        *v = if pack == OutPack::L { (o & 0xffff_0000) | h } else { (o & 0xffff) | (h << 16) };
    }
}

/// One flag of `f` by the letter of a push or update.
fn pick(f: &FlagVals, which: char, what: &str) -> R<bool> {
    let v = match which {
        'z' => f.z,
        'n' => f.n,
        _ => f.c,
    };
    v.ok_or_else(|| format!("{what}: its {} flag is not described", which.to_ascii_uppercase()))
}

/// `undef`: the lanes whose result is undefined; their flags become
/// undefined too, refused only where a branch or a sink reads them.
fn apply_flags(m: &mut Machine, pf: Pf, uf: Uf, res: Option<&Computed>, undef: [bool; LANES], what: &str) -> R<()> {
    if pf == Pf::None && uf == Uf::None {
        return Ok(());
    }
    let Some(res) = res else { return unmodelled(format!("{what}: a flag push from an op with no result")) };
    if pf != Pf::None {
        let which = match pf {
            Pf::PushZ => 'z',
            Pf::PushN => 'n',
            _ => 'c',
        };
        for (l, u) in undef.iter().enumerate() {
            let v = if *u { None } else { Some(pick(&res.flags[l], which, what)?) };
            m.flag_b[l] = m.flag_a[l];
            m.flag_a[l] = v;
        }
        return Ok(());
    }
    let (and, which, neg) = match uf {
        Uf::AndZ => (true, 'z', false),
        Uf::AndNZ => (true, 'z', true),
        Uf::NorZ => (false, 'z', false),
        Uf::NorNZ => (false, 'z', true),
        Uf::AndN => (true, 'n', false),
        Uf::AndNN => (true, 'n', true),
        Uf::NorN => (false, 'n', false),
        Uf::NorNN => (false, 'n', true),
        Uf::AndC => (true, 'c', false),
        Uf::AndNC => (true, 'c', true),
        Uf::NorC => (false, 'c', false),
        Uf::NorNC => (false, 'c', true),
        Uf::None => return Ok(()),
    };
    for (l, u) in undef.iter().enumerate() {
        let x = if *u { None } else { Some(pick(&res.flags[l], which, what)? != neg) };
        // Three-valued, exactly: A & X is false when either is known false,
        // !A & !X when either is known true, whatever the other is. Mesa's
        // divergent code relies on it: `mov.andz -, rf11` clears A in an
        // inactive lane (rf11 != 0) whatever the undefined condition was.
        m.flag_a[l] = match (m.flag_a[l], x, and) {
            (Some(false), _, true) | (_, Some(false), true) => Some(false),
            (Some(true), Some(true), true) => Some(true),
            (Some(true), _, false) | (_, Some(true), false) => Some(false),
            (Some(false), Some(false), false) => Some(true),
            _ => None,
        };
        m.flag_b[l] = None;
    }
    Ok(())
}

/// Where one ALU half's write goes.
enum Dest {
    Reg(Reg),
    Nop,
    Tlb,
    Tlbu,
    Unifa,
    Tmu(u8),
}

fn dest(waddr: u8, magic: bool, what: &str) -> R<Dest> {
    if !magic {
        return Ok(Dest::Reg(Reg::Rf(waddr)));
    }
    match waddr {
        0..=5 => Ok(Dest::Reg(Reg::Acc(waddr))),
        6 => Ok(Dest::Nop),
        7 => Ok(Dest::Tlb),
        8 => Ok(Dest::Tlbu),
        9 => Ok(Dest::Unifa),
        10..=13 | 32..=46 => Ok(Dest::Tmu(waddr)),
        w => unmodelled(format!("{what}: a write to magic register {}", magic_waddr_name(w).unwrap_or("?"))),
    }
}

/// The register a signal with an address writes.
fn sig_dest(alu: &Alu, what: &str) -> R<Reg> {
    if !alu.sig_magic {
        return Ok(Reg::Rf(alu.sig_addr));
    }
    match alu.sig_addr {
        0..=5 => Ok(Reg::Acc(alu.sig_addr)),
        a => unmodelled(format!("{what}: a signal writing magic register {}", magic_waddr_name(a).unwrap_or("?"))),
    }
}

/// Something that happens after a THRSW's or a branch's delay slots.
enum Event {
    Switch,
    End,
    Jump { pc: usize, uniform: Option<usize> },
}

/// Run `code` with `uniforms` (already resolved to values), `inputs`, and
/// what the program may read besides them (`env`).
pub fn run(code: &[u64], uniforms: &[u32], inputs: &Inputs, env: &Env) -> Result<Outcome, String> {
    let mut m = Machine {
        uniforms,
        uni: 0,
        acc: [[None; LANES]; 6],
        rf: [[None; LANES]; 64],
        flag_a: [None; LANES],
        flag_b: [None; LANES],
        msf: [1; LANES],
        pending: Vec::new(),
        tick: 0,
        out: Outcome::default(),
        fs: None,
        vs: None,
        next_varying: 0,
        vpm_written: Vec::new(),
        env,
        unifa: None,
        rtop: Rtop::Zero,
        tmu_write_unwaited: false,
        thrend: None,
        undef_read: core::cell::Cell::new([false; LANES]),
        tmu_coords: Vec::new(),
        tmu_config: Vec::new(),
        tmu_fifo: std::collections::VecDeque::new(),
        switches: 0,
        tmu_data: None,
        scratch: std::collections::BTreeMap::new(),
    };
    match inputs {
        Inputs::Fragment(f) => {
            m.rf[0] = f.w.map(Some);
            m.rf[1] = f.w_centroid.map(Some);
            m.rf[2] = f.z.map(Some);
            m.fs = Some((**f).clone());
        }
        Inputs::Vertex(v) => m.vs = Some(v.clone()),
    }
    let thrsw: Vec<bool> = code.iter().map(|&w| matches!(decode(w), Ok(Instr::Alu(a)) if a.sig.thrsw)).collect();
    let mut last_section = !thrsw.windows(2).any(|p| p[0] && p[1]);
    let mut prev_thrsw_pc: Option<usize> = None;
    let mut events: Vec<(usize, Event)> = Vec::new();
    // No uniform reads before this tick: a branch's delay slots.
    let mut no_uniform_until = 0usize;
    let mut pc = 0usize;
    loop {
        if m.tick > 1_000_000 {
            return unmodelled("did not end in 1,000,000 instructions");
        }
        m.land()?;
        let mut i = 0;
        while i < events.len() {
            if events[i].0 != m.tick {
                i += 1;
                continue;
            }
            match events.remove(i).1 {
                Event::Switch => {
                    if m.unifa.is_some_and(|(at, _)| m.tick <= at + 3) {
                        return unmodelled("a UNIFA write within 3 instructions of a thread switch");
                    }
                    m.thread_switch()
                }
                Event::End => {
                    if m.unifa.is_some_and(|(at, _)| m.tick <= at + 3) {
                        return unmodelled("a UNIFA write within 3 instructions of the thread end");
                    }
                    // Mesa never ends a program with TMU work outstanding: it
                    // flushes every operation with its LDTMUs or a TMUWT first
                    // (ntq_flush_tmu, nir_to_vir.c:262-292).
                    if !m.tmu_fifo.is_empty() {
                        return unmodelled(format!(
                            "{} TMU results never read by an LDTMU at the thread end",
                            m.tmu_fifo.len()
                        ));
                    }
                    if !m.tmu_coords.is_empty() || !m.tmu_config.is_empty() || m.tmu_data.is_some() {
                        return unmodelled("a TMU request not retired at the thread end");
                    }
                    if m.tmu_write_unwaited {
                        return unmodelled("a general TMU write not waited for by a TMUWT at the thread end");
                    }
                    m.out.msf = m.msf;
                    m.out.instructions = m.tick;
                    return Ok(m.out);
                }
                Event::Jump { pc: to, uniform } => {
                    pc = to;
                    if let Some(u) = uniform {
                        m.uni = u;
                    }
                }
            }
        }
        let Some(&w) = code.get(pc) else {
            return unmodelled(format!("ran off the end of the program at {pc} without a thread end"));
        };
        let instr = decode(w).map_err(|e| format!("instruction {pc} ({w:#018x}) does not decode: {e:?}"))?;
        let this_pc = pc;
        pc += 1;
        let tick = m.tick;
        let what = format!("instruction {this_pc}");
        match instr {
            Instr::Branch(br) => {
                if events.iter().any(|(_, e)| matches!(e, Event::Jump { .. })) || tick < no_uniform_until {
                    return unmodelled(format!("{what}: a branch in a branch's delay slots"));
                }
                if br.bdi != BranchDest::Rel || (br.ub && br.bdu != BranchDest::Rel) {
                    return unmodelled(format!("{what}: branch destination {:?}/{:?}", br.bdi, br.bdu));
                }
                if br.msfign == Msfign::Q {
                    return unmodelled(format!("{what}: branch msfign Q"));
                }
                let branch_uniform = m.uni;
                let uval = if br.ub { Some(m.next_uniform(&what)?) } else { None };
                let mut flags = Vec::new();
                for l in 0..LANES {
                    if br.msfign == Msfign::P && m.msf[l] == 0 {
                        continue;
                    }
                    match m.flag_a[l] {
                        Some(f) => flags.push(f),
                        None if br.cond == BranchCond::Always => {}
                        None => return unmodelled(format!("{what}: a branch on an undefined A in lane {l}")),
                    }
                }
                let taken = match br.cond {
                    BranchCond::Always => true,
                    BranchCond::AllA => flags.iter().all(|x| *x),
                    BranchCond::AnyA => flags.iter().any(|x| *x),
                    BranchCond::AllNA => flags.iter().all(|x| !*x),
                    BranchCond::AnyNA => flags.iter().any(|x| !*x),
                    c => return unmodelled(format!("{what}: branch condition {c:?}")),
                };
                no_uniform_until = tick + 4;
                if taken {
                    let to = this_pc as i64 + 4 + (br.offset as i32 as i64) / 8;
                    if to < 0 || to as usize >= code.len() {
                        return unmodelled(format!("{what}: branch target {to} outside the program"));
                    }
                    let uniform = match uval {
                        Some(v) => {
                            let off = v as i32;
                            let u = branch_uniform as i64 + 1 + off as i64 / 4;
                            if off % 4 != 0 || u < 0 {
                                return unmodelled(format!("{what}: uniform-stream branch offset {off}"));
                            }
                            Some(u as usize)
                        }
                        None => None,
                    };
                    events.push((tick + 4, Event::Jump { pc: to as usize, uniform }));
                }
                prev_thrsw_pc = None;
            }
            Instr::Alu(alu) => {
                let s = alu.sig;
                if s.ldtlb || s.ldtlbu || s.ucb || s.rotate {
                    return unmodelled(format!("{what}: signal {s:?}"));
                }
                // Only one of [TMU, SFU, TSY, TLB read, VPM] per instruction,
                // LDTMU counted on 4.2 (qpu_validate.c:249-343): the magic
                // write addresses of both halves, and the signals.
                let unit_uses = |op_nop: bool, magic: bool, waddr: u8| -> usize {
                    if op_nop || !magic {
                        return 0;
                    }
                    let n = magic_waddr_name(waddr).unwrap_or("");
                    usize::from(
                        n.starts_with("tmu")
                            || matches!(n, "recip" | "rsqrt" | "exp" | "log" | "sin" | "rsqrt2")
                            || matches!(n, "vpm" | "vpmu" | "tlb" | "tlbu" | "sync" | "syncu" | "syncb"),
                    )
                };
                let units = unit_uses(alu.add.op == AddOp::Nop, alu.add.magic, alu.add.waddr)
                    + unit_uses(alu.mul.op == MulOp::Nop, alu.mul.magic, alu.mul.waddr)
                    + usize::from(s.ldtmu)
                    + usize::from(s.ldtlb)
                    + usize::from(s.ldtlbu); // (LDVPM is V3D 3.x: not on 4.2)
                if units > 1 {
                    return unmodelled(format!("{what}: {units} of TMU, SFU, TSY, TLB, VPM, LDTMU in one instruction"));
                }
                // And the peripherals, as Mesa's scheduler pairs them: WRTMUC
                // and an SFU or VPM op count too (see `peripherals`). A WRTMUC
                // sharing its word with an SFU op or a VPM read made the Pi 4's
                // L2T fault while the validator's count above let it through
                // (v3d-m5, 2026-09-25).
                let p = peripherals(&alu);
                if !peripherals_compatible(p, &alu) {
                    return unmodelled(format!(
                        "{what}: the peripherals {} in one instruction (V3D 4.2 allows one, or WRTMUC with a TMU \
                         write other than TMUC, or LDTMU with a VPM read or write)",
                        peripheral_names(p)
                    ));
                }
                // The thread end's THRSW and its two delay slots write no
                // register file, and the last instruction is no TMUWT
                // (qpu_validate.c:373-427; GFXH-1625).
                if m.thrend.is_some_and(|t| tick <= t + 2) {
                    if rf_write(&alu) {
                        return unmodelled(format!("{what}: a register-file write after the thread end"));
                    }
                    if m.thrend == Some(tick - 2) && alu.add.op == AddOp::Tmuwt {
                        return unmodelled(format!("{what}: a TMUWT in the last instruction"));
                    }
                }
                // A write to TLBU or TMUAU takes a uniform too
                // (v3d_qpu_magic_waddr_loads_unif; VPMU and SYNCU are not
                // modelled). One instruction reads at most one.
                let loads_unif = |magic: bool, waddr: u8| magic && matches!(waddr, 8 | 13);
                let uniform_reads = [
                    s.wrtmuc,
                    s.ldunif,
                    s.ldunifrf,
                    alu.add.op != AddOp::Nop && loads_unif(alu.add.magic, alu.add.waddr),
                    alu.mul.op != MulOp::Nop && loads_unif(alu.mul.magic, alu.mul.waddr),
                ]
                .iter()
                .filter(|x| **x)
                .count();
                if uniform_reads > 1 {
                    return unmodelled(format!("{what}: {uniform_reads} uniform reads in one instruction"));
                }
                let reads_uniform = uniform_reads == 1;
                if reads_uniform && tick < no_uniform_until {
                    return unmodelled(format!("{what}: a uniform read in a branch delay slot"));
                }
                if (alu.flags.apf != Pf::None || alu.flags.auf != Uf::None)
                    && (alu.flags.mpf != Pf::None || alu.flags.muf != Uf::None)
                {
                    return unmodelled(format!("{what}: both ALUs update the flags"));
                }
                let (add_mask, add_unknown) = m.cond_lanes(alu.flags.ac);
                let (mul_mask, mul_unknown) = m.cond_lanes(alu.flags.mc);
                // WRTMUC's word belongs to the request this instruction's
                // own TMU write may retire (Mesa pairs the last config word
                // with the S write: `mov tmus, r3 ; wrtmuc`), so it is taken
                // before the ALU writes. An instruction reads at most one
                // uniform (above), so the stream order does not change.
                if s.wrtmuc {
                    let v = m.next_uniform(&what)?;
                    m.tmu_config.push(v);
                }
                m.undef_read.set([false; LANES]);
                let add_res = add_op(&mut m, &alu).map_err(|e| format!("{what}: {e}"))?;
                let add_undef = m.undef_read.get();
                // A MOV straight to the TLB may carry an undefined value
                // (see TlbWrite); it is recorded as it is.
                let tlb_mov = alu.mul.op == MulOp::Mov
                    && alu.mul.magic
                    && matches!(alu.mul.waddr, 7 | 8)
                    && alu.mul.a.unpack == Unpack::None
                    && alu.flags.mpf == Pf::None
                    && alu.flags.muf == Uf::None;
                let tlb_mov_vals = if tlb_mov { Some(m.operand_lanes(&alu, alu.mul.a.mux, &what)?) } else { None };
                m.undef_read.set([false; LANES]);
                let mul_res = if tlb_mov { None } else { mul_op(&mut m, &alu).map_err(|e| format!("{what}: {e}"))? };
                let mul_undef = m.undef_read.get();
                for (res, waddr, magic, pack, (mask, unknown, undef), half) in [
                    (&add_res, alu.add.waddr, alu.add.magic, alu.add.pack, (add_mask, add_unknown, add_undef), "add"),
                    (&mul_res, alu.mul.waddr, alu.mul.magic, alu.mul.pack, (mul_mask, mul_unknown, mul_undef), "mul"),
                ] {
                    let Some(r) = res else { continue };
                    // A lane with an unknown condition may or may not be
                    // written: for a register it becomes undefined; a sink
                    // refuses it, as it refuses an undefined value.
                    let sink_undef = (0..LANES).any(|l| unknown[l] || (mask[l] && undef[l]));
                    let what = format!("{what} {half}");
                    if !r.float && pack != OutPack::None {
                        return unmodelled(format!("{what}: an output pack on a non-float result"));
                    }
                    if let Some(op) = r.sfu {
                        m.out.approximate.push(op);
                    }
                    let mut vals = r.vals;
                    match dest(waddr, magic, &what)? {
                        Dest::Nop if pack == OutPack::None => {}
                        Dest::Nop => return unmodelled(format!("{what}: an output pack to NOP")),
                        Dest::Tlb | Dest::Tlbu => {
                            if mask.iter().any(|x| !x) || unknown.iter().any(|x| *x) || pack != OutPack::None {
                                return unmodelled(format!("{what}: a conditional or packed TLB write"));
                            }
                            let config = if waddr == 8 { Some(m.next_uniform(&what)?) } else { None };
                            // An undefined lane goes to the TLB as undefined:
                            // the caller refuses it where the pixel is kept.
                            let value = core::array::from_fn(|l| if undef[l] { None } else { Some(vals[l]) });
                            m.out.tlb.push(TlbWrite { config, value });
                        }
                        Dest::Tmu(w) => {
                            if pack != OutPack::None {
                                return unmodelled(format!("{what}: a packed TMU write"));
                            }
                            if sink_undef {
                                return unmodelled(format!("{what}: an undefined lane reaches a TMU write"));
                            }
                            m.tmu_write(w, vals, mask, &what)?;
                        }
                        Dest::Unifa => {
                            if mask.iter().any(|x| !x) || pack != OutPack::None {
                                return unmodelled(format!("{what}: a conditional or packed UNIFA write"));
                            }
                            if tick < no_uniform_until {
                                return unmodelled(format!("{what}: a UNIFA write in a branch delay slot"));
                            }
                            // The address comes from lane 0 (see LDUNIFA).
                            if undef[0] || unknown[0] {
                                return unmodelled(format!("{what}: an undefined lane 0 reaches UNIFA"));
                            }
                            m.unifa = Some((tick, vals[0]));
                        }
                        Dest::Reg(reg) => {
                            let mut undef = undef;
                            output_pack(&m, pack, reg, &mut vals, &mut undef);
                            // An SFU result in the register file stalls a read
                            // until it is there (read_stalls); in an
                            // accumulator it lands two instructions later.
                            let acc_sfu = r.sfu.is_some() && matches!(reg, Reg::Acc(_));
                            let at = if acc_sfu { tick + 2 } else { tick + 1 };
                            let touched = core::array::from_fn(|l| mask[l] || unknown[l]);
                            let undef = core::array::from_fn(|l| undef[l] || unknown[l]);
                            m.schedule(at, reg, vals, touched, undef, acc_sfu);
                        }
                    }
                }
                if let Some(value) = tlb_mov_vals {
                    if mul_mask.iter().any(|x| !x) || mul_unknown.iter().any(|x| *x) {
                        return unmodelled(format!("{what} mul: a conditional TLB write"));
                    }
                    let config = if alu.mul.waddr == 8 { Some(m.next_uniform(&what)?) } else { None };
                    m.out.tlb.push(TlbWrite { config, value });
                }
                apply_flags(&mut m, alu.flags.apf, alu.flags.auf, add_res.as_ref(), add_undef, &format!("{what} add"))?;
                apply_flags(&mut m, alu.flags.mpf, alu.flags.muf, mul_res.as_ref(), mul_undef, &format!("{what} mul"))?;

                if s.ldunif {
                    let v = m.next_uniform(&what)?;
                    m.schedule(tick + 1, Reg::Acc(5), [v; LANES], [true; LANES], [false; LANES], false);
                }
                if s.ldunifrf {
                    let v = m.next_uniform(&what)?;
                    if alu.sig_magic && alu.sig_addr == 9 {
                        // Straight into UNIFA (nir_to_vir.c:3348-3360).
                        m.unifa = Some((tick, v));
                    } else {
                        let r = sig_dest(&alu, &what)?;
                        m.schedule(tick + 1, r, [v; LANES], [true; LANES], [false; LANES], false);
                    }
                }
                if s.ldtmu {
                    let Some((v, switches)) = m.tmu_fifo.pop_front() else {
                        return unmodelled(format!("{what}: LDTMU with no TMU result waiting"));
                    };
                    m.out.ldtmus += 1;
                    if switches == m.switches {
                        if m.env.threads > 1 {
                            return unmodelled(format!("{what} ({} threads): {NO_SWITCH_HANG}", m.env.threads));
                        }
                        m.out.ldtmus_without_switch += 1;
                    }
                    let r = sig_dest(&alu, &what)?;
                    m.schedule(tick + 1, r, v, [true; LANES], [false; LANES], false);
                }
                if s.ldunifa {
                    let v = m.ldunifa(&what)?;
                    m.schedule(tick + 1, Reg::Acc(5), [v; LANES], [true; LANES], [false; LANES], false);
                }
                if s.ldunifarf {
                    let v = m.ldunifa(&what)?;
                    let r = sig_dest(&alu, &what)?;
                    m.schedule(tick + 1, r, [v; LANES], [true; LANES], [false; LANES], false);
                }
                if s.ldvary {
                    let Some(fs) = &m.fs else {
                        return unmodelled(format!("{what}: LDVARY outside a fragment shader"));
                    };
                    let Some(v) = fs.varyings.get(m.next_varying).cloned() else {
                        return unmodelled(format!("{what}: LDVARY number {} has no varying supplied", m.next_varying));
                    };
                    m.next_varying += 1;
                    let r = sig_dest(&alu, &what)?;
                    m.schedule(tick + 1, r, v.vary, [true; LANES], [false; LANES], false);
                    m.schedule(tick + 2, Reg::Acc(5), v.c, [true; LANES], [false; LANES], false);
                }
                if s.thrsw {
                    if prev_thrsw_pc.is_some_and(|p| p + 1 == this_pc) {
                        // The second of a pair marks the last section.
                        last_section = true;
                        prev_thrsw_pc = None;
                    } else {
                        events.push((tick + 3, if last_section { Event::End } else { Event::Switch }));
                        if last_section {
                            if rf_write(&alu) {
                                return unmodelled(format!("{what}: a register-file write at the thread end"));
                            }
                            m.thrend = Some(tick);
                        }
                        prev_thrsw_pc = Some(this_pc);
                    }
                } else {
                    prev_thrsw_pc = None;
                }
            }
        }
        m.tick += 1;
    }
}

#[cfg(test)]
mod undefined_lane_tests {
    //! An undefined lane is computed on and carried, and refused at every
    //! sink the module notes list; small programs built with the encoder.
    use super::*;
    use crate::shaderc::online::v3d::encode::encode;

    const RF_NEVER_WRITTEN: u8 = 50;

    fn mov_mul(dst: u8, magic: bool, src: Mux) -> Alu {
        let mut a = Alu::nop();
        a.mul.op = MulOp::Mov;
        a.mul.a = Input { mux: src, unpack: Unpack::None };
        a.mul.waddr = dst;
        a.mul.magic = magic;
        a.raddr_a = RF_NEVER_WRITTEN;
        a
    }

    fn thrsw(mut a: Alu) -> Alu {
        a.sig.thrsw = true;
        a
    }

    /// A fragment program: the last-section THRSW pair, `body`, then the
    /// program end (a THRSW and its two delay slots).
    fn fs(body: &[Instr], uniforms: &[u32]) -> Result<Outcome, String> {
        let mut code = vec![Instr::Alu(thrsw(Alu::nop())), Instr::Alu(thrsw(Alu::nop()))];
        code.extend_from_slice(body);
        code.extend([Instr::Alu(thrsw(Alu::nop())), Instr::Alu(Alu::nop()), Instr::Alu(Alu::nop())]);
        let words: Vec<u64> = code.iter().map(|i| encode(i).expect("encodes")).collect();
        let one = [1.0f32.to_bits(); LANES];
        let inputs = FsInputs {
            w: one,
            w_centroid: one,
            z: one,
            x: one,
            y: one,
            back_facing: [false; LANES],
            varyings: Vec::new(),
        };
        run(&words, uniforms, &Inputs::Fragment(Box::new(inputs)), &Env::default())
    }

    /// Computing on an undefined lane is not refused; the result reaches
    /// the TLB as undefined, never as the stand-in's value (the caller
    /// refuses a kept pixel's undefined colour).
    #[test]
    fn an_undefined_operand_computes_and_reaches_the_tlb_undefined() {
        let mut add = Alu::nop();
        add.add.op = AddOp::Fadd;
        add.add.a = Input { mux: Mux::A, unpack: Unpack::None };
        add.add.b = Input { mux: Mux::A, unpack: Unpack::None };
        add.add.waddr = 0; // r0
        add.raddr_a = RF_NEVER_WRITTEN;
        let o = fs(&[Instr::Alu(add), Instr::Alu(mov_mul(7, true, Mux::R0))], &[]).expect("runs");
        assert_eq!(o.tlb.len(), 1);
        assert!(o.tlb[0].value.iter().all(Option::is_none), "{:?}", o.tlb[0].value);
    }

    #[test]
    fn a_sink_refuses_an_undefined_lane() {
        // A TMU coordinate.
        let e = fs(&[Instr::Alu(mov_mul(34, true, Mux::A))], &[]).unwrap_err();
        assert!(e.contains("an undefined lane reaches a TMU write"), "{e}");
        // UNIFA.
        let e = fs(&[Instr::Alu(mov_mul(9, true, Mux::A))], &[]).unwrap_err();
        assert!(e.contains("an undefined lane 0 reaches UNIFA"), "{e}");
        // SETMSF.
        let mut msf = Alu::nop();
        msf.add.op = AddOp::Setmsf;
        msf.add.a = Input { mux: Mux::A, unpack: Unpack::None };
        msf.raddr_a = RF_NEVER_WRITTEN;
        let e = fs(&[Instr::Alu(msf)], &[]).unwrap_err();
        assert!(e.contains("an undefined lane reaches the multisample flags"), "{e}");
        // A branch on a flag pushed from an undefined lane.
        let mut cmp = Alu::nop();
        cmp.add.op = AddOp::Fcmp;
        cmp.add.a = Input { mux: Mux::A, unpack: Unpack::None };
        cmp.add.b = Input { mux: Mux::A, unpack: Unpack::None };
        cmp.raddr_a = RF_NEVER_WRITTEN;
        cmp.flags.apf = Pf::PushZ;
        let br = Branch {
            cond: BranchCond::AnyA,
            msfign: Msfign::None,
            bdi: BranchDest::Rel,
            ub: false,
            bdu: BranchDest::Abs,
            raddr_a: 0,
            offset: 0,
        };
        let nop = Instr::Alu(Alu::nop());
        let e = fs(&[Instr::Alu(cmp), Instr::Branch(br), nop, nop, nop], &[]).unwrap_err();
        assert!(e.contains("a branch on an undefined A"), "{e}");
    }

    /// `body` as a fragment program run with a texture bound at index 0
    /// and `threads` threads.
    fn fs_tex(body: &[Instr], uniforms: &[u32], threads: u32) -> Result<Outcome, String> {
        let mut code = vec![Instr::Alu(thrsw(Alu::nop())), Instr::Alu(thrsw(Alu::nop()))];
        code.extend_from_slice(body);
        code.extend([Instr::Alu(thrsw(Alu::nop())), Instr::Alu(Alu::nop()), Instr::Alu(Alu::nop())]);
        let words: Vec<u64> = code.iter().map(|i| encode(i).expect("encodes")).collect();
        let half = [0.3f32.to_bits(); LANES];
        let inputs = FsInputs {
            w: half,
            w_centroid: half,
            z: half,
            x: half,
            y: half,
            back_facing: [false; LANES],
            varyings: Vec::new(),
        };
        let tex = Texture::flat(1, 1, vec![[0.5, 0.25, 0.125, 1.0]]);
        let env = Env { textures: vec![tex], threads, ..Env::default() };
        run(&words, uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
    }

    /// `body` as a fragment program with `tex` at index 0, one thread.
    fn fs_with(body: &[Alu], uniforms: &[u32], tex: Texture) -> Result<Outcome, String> {
        fs_with_sampler(body, uniforms, tex, None)
    }

    /// [`fs_with`] with sampler 0's state given (`None`: the texture's
    /// NEAREST / CLAMP_TO_EDGE default).
    fn fs_with_sampler(
        body: &[Alu],
        uniforms: &[u32],
        tex: Texture,
        sampler: Option<Sampler>,
    ) -> Result<Outcome, String> {
        let mut code = vec![thrsw(Alu::nop()), thrsw(Alu::nop())];
        code.extend_from_slice(body);
        code.extend([thrsw(Alu::nop()), Alu::nop(), Alu::nop()]);
        let words: Vec<u64> = code.iter().map(|i| encode(&Instr::Alu(*i)).expect("encodes")).collect();
        let half = [0.3f32.to_bits(); LANES];
        let inputs = FsInputs {
            w: half,
            w_centroid: half,
            z: half,
            x: half,
            y: half,
            back_facing: [false; LANES],
            varyings: Vec::new(),
        };
        let env = Env { textures: vec![tex], samplers: sampler.into_iter().collect(), threads: 1, ..Env::default() };
        run(&words, uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
    }

    /// A look-up as Mesa's v3d_vir_emit_tex orders it, every value from
    /// the uniform stream: P0, P1 (32-bit), T, then R when `r`, TMUB (with
    /// P2 = disable_autolod when `p2`) when `b`, S by `retire`; the one
    /// result word (R) to the TLB. Returns the program and its uniforms.
    fn tex_program(retire: u8, t: f32, r: Option<f32>, b: Option<f32>, p2: bool, s: f32) -> (Vec<Alu>, Vec<u32>) {
        tex_program_layer(retire, t, r, None, b, p2, s)
    }

    /// [`tex_program`] with a 2D array's layer written to TMUI when `i`
    /// (after R, before TMUB: Mesa's coordinate order).
    fn tex_program_layer(
        retire: u8,
        t: f32,
        r: Option<f32>,
        i: Option<f32>,
        b: Option<f32>,
        p2: bool,
        s: f32,
    ) -> (Vec<Alu>, Vec<u32>) {
        let bits = |x: Option<f32>| x.map(f32::to_bits);
        /* P2: op regular (15), disable_autolod */
        tex_program_bits(retire, t.to_bits(), bits(r), bits(i), bits(b), p2.then_some(0x00f0_0002), s.to_bits())
    }

    /// [`tex_program_layer`] on raw words (a texel fetch's integer
    /// coordinates and level), with any P2 word: P0, P1 (32-bit), P2 when
    /// given (a third `wrtmuc`, before the coordinates as Mesa orders the
    /// configuration words), T, R, I, TMUB, then S by `retire`.
    fn tex_program_bits(
        retire: u8,
        t: u32,
        r: Option<u32>,
        i: Option<u32>,
        b: Option<u32>,
        p2: Option<u32>,
        s: u32,
    ) -> (Vec<Alu>, Vec<u32>) {
        let mut ld = Alu::nop();
        ld.sig.ldunif = true; // r5
        let mut wrtmuc = Alu::nop();
        wrtmuc.sig.wrtmuc = true;
        let mut code = vec![Alu::nop(), ld, wrtmuc];
        let mut unif = vec![t, 0x1, 0x1]; // T, P0 (unit 0, word 0), P1 (32-bit)
        let mut c1 = mov_mul(34, true, Mux::R5); // TMUT
        c1.sig.wrtmuc = true; // P1
        code.push(c1);
        if let Some(p2) = p2 {
            code.push(wrtmuc);
            unif.push(p2);
        }
        if let Some(r) = r {
            code.extend([ld, mov_mul(35, true, Mux::R5)]); // TMUR
            unif.push(r);
        }
        if let Some(i) = i {
            code.extend([ld, mov_mul(36, true, Mux::R5)]); // TMUI
            unif.push(i);
        }
        if let Some(b) = b {
            code.extend([ld, mov_mul(37, true, Mux::R5)]); // TMUB
            unif.push(b);
        }
        code.extend([ld, mov_mul(retire, true, Mux::R5)]);
        unif.push(s);
        let mut ldtmu = Alu::nop();
        ldtmu.sig.ldtmu = true;
        ldtmu.sig_addr = 0;
        ldtmu.sig_magic = true; // r0
        code.extend([ldtmu, mov_mul(7, true, Mux::R0)]); // TLB
        (code, unif)
    }

    /// Every channel of level k's every texel of face f is 10k + f.
    fn mip_texture(cube: bool) -> Texture {
        use crate::shaderc::online::texref::{Level, MinFilter};
        let lv = |k: u32| {
            let w = 4 >> k;
            let faces =
                (0..if cube { 6 } else { 1 }).map(|f| vec![[(10 * k + f) as f32; 4]; (w * w) as usize]).collect();
            Level { width: w, height: w, faces }
        };
        Texture { cube, min: MinFilter::NearestMipmapNearest, levels: (0..3).map(lv).collect() }
    }

    /// A 2D texture of three levels (4x4, 2x2, 1x1), NEAREST_MIPMAP_NEAREST:
    /// every channel of texel (i, j) of level k is 100k + 10j + i (exact in
    /// f16).
    fn indexed_texture() -> Texture {
        use crate::shaderc::online::texref::{Level, MinFilter};
        let lv = |k: u32| {
            let w = 4 >> k;
            let texels = (0..w * w).map(|n| [(100 * k + 10 * (n / w) + n % w) as f32; 4]).collect();
            Level { width: w, height: w, faces: vec![texels] }
        };
        Texture { cube: false, min: MinFilter::NearestMipmapNearest, levels: (0..3).map(lv).collect() }
    }

    fn tlb_r(o: &Outcome) -> f32 {
        f32::from_bits(o.tlb[0].value[0].expect("a defined value"))
    }

    /// A texel fetch (S by TMUSF, v3d_tex.c:372-374): S and T are integer
    /// texel coordinates and TMUB the integer level, and the texel read is
    /// that one of that level, whatever the sampler (no filter, no wrap).
    /// P2's S/T offset nibbles move it by texels. Refused: a level or a
    /// texel outside the texture (undefined in GL), no TMUB, disable_autolod
    /// on a fetch (Mesa's assert, v3d_tex.c:141-150), a cube texture, an
    /// array layer (TMUI) with TMUSF, and an R offset (not a regular P2).
    #[test]
    fn a_texel_fetch_reads_the_texel_of_its_level() {
        let fetch = |x: i32, y: i32, level: i32, p2: Option<u32>| {
            let (code, unif) = tex_program_bits(41, y as u32, None, None, Some(level as u32), p2, x as u32);
            fs_with(&code, &unif, indexed_texture())
        };
        for (x, y, level, want) in
            [(2, 1, 0, 12.0f32), (3, 3, 0, 33.0), (1, 0, 1, 101.0), (0, 1, 1, 110.0), (0, 0, 2, 200.0)]
        {
            let o = fetch(x, y, level, None).expect("runs");
            assert_eq!((tlb_r(&o), o.levels, o.tmu_lookups), (want, 1 << level, 1), "({x}, {y}) of level {level}");
        }
        /* P2 offsets, 4-bit two's complement nibbles: (+1, -1) from (1, 1)
         * is texel (2, 0); (-8, +7) is outside, refused */
        let p2 = super::super::select::tmu_p2;
        assert_eq!(p2(false, [1, -1]), 0x00f0_f100, "S nibble at bit 8, T at 12 (v3d_packet.xml)");
        let o = fetch(1, 1, 0, Some(p2(false, [1, -1]))).expect("runs");
        assert_eq!(tlb_r(&o), 2.0);
        let o = fetch(1, 1, 1, Some(p2(false, [0, -1]))).expect("runs");
        assert_eq!(tlb_r(&o), 101.0, "the offset is in texels of the level fetched");
        let e = fetch(1, 1, 0, Some(p2(false, [-8, 7]))).unwrap_err();
        assert!(e.contains("a texel fetch of (-7, 8) outside level 0 (4 x 4)"), "{e}");
        /* outside, by coordinate or by level */
        let e = fetch(4, 0, 0, None).unwrap_err();
        assert!(e.contains("a texel fetch of (4, 0) outside level 0 (4 x 4)"), "{e}");
        let e = fetch(0, -1, 1, None).unwrap_err();
        assert!(e.contains("a texel fetch of (0, -1) outside level 1 (2 x 2)"), "{e}");
        let e = fetch(0, 0, 3, None).unwrap_err();
        assert!(e.contains("a texel fetch of level 3 on a texture of 3 levels"), "{e}");
        let e = fetch(0, 0, -1, None).unwrap_err();
        assert!(e.contains("a texel fetch of level -1 on a texture of 3 levels"), "{e}");
        /* no level, disable_autolod, a cube texture, an array layer, an R offset */
        let (code, unif) = tex_program_bits(41, 0, None, None, None, None, 0);
        let e = fs_with(&code, &unif, indexed_texture()).unwrap_err();
        assert!(e.contains("an explicit level of detail with no TMUB write"), "{e}");
        let e = fetch(0, 0, 0, Some(p2(true, [0, 0]))).unwrap_err();
        assert!(e.contains("a texel fetch with disable_autolod"), "{e}");
        let (code, unif) = tex_program_bits(41, 0, None, None, Some(0), None, 0);
        let e = fs_with(&code, &unif, mip_texture(true)).unwrap_err();
        assert!(e.contains("a 2D lookup on cube texture 0"), "{e}");
        let (code, unif) = tex_program_bits(41, 0, None, Some(0), Some(0), None, 0);
        let e = fs_with(&code, &unif, indexed_texture()).unwrap_err();
        assert!(e.contains("a texel fetch of a 2D array (TMUI)"), "{e}");
        let e = fetch(0, 0, 0, Some(0x00f1_0000)).unwrap_err();
        assert!(e.contains("TMU P2 0x00f10000 is not a regular lookup"), "{e}");
        /* an integer texture is fetched raw, whatever the filter */
        let (code, unif) = tex_program_bits(41, 1, None, None, Some(0), None, 2);
        let tex = indexed_texture();
        let env = Env { textures: vec![tex], threads: 1, integer_textures: 1, ..Env::default() };
        let mut body = vec![thrsw(Alu::nop()), thrsw(Alu::nop())];
        body.extend_from_slice(&code);
        body.extend([thrsw(Alu::nop()), Alu::nop(), Alu::nop()]);
        let words: Vec<u64> = body.iter().map(|i| encode(&Instr::Alu(*i)).expect("encodes")).collect();
        let half = [0.3f32.to_bits(); LANES];
        let inputs = FsInputs {
            w: half,
            w_centroid: half,
            z: half,
            x: half,
            y: half,
            back_facing: [false; LANES],
            varyings: Vec::new(),
        };
        let o = run(&words, &unif, &Inputs::Fragment(Box::new(inputs)), &env).expect("runs");
        assert_eq!(o.tlb[0].value[0], Some(12), "the integer 12, raw");
    }

    /// A sampled look-up with P2's texel offset reads the texel `offset`
    /// away, in texels of the level read, under the sampler's wrap mode:
    /// GLSL ES 3.00 8.8's textureOffset. A cube with an offset and an offset
    /// under a mipmap-LINEAR blend are refused.
    #[test]
    fn a_texel_offset_moves_a_sampled_look_up_by_texels_of_its_level() {
        let p2 = super::super::select::tmu_p2;
        /* NEAREST at (0.3, 0.3): u = v = 1.2, texel (1, 1) of level 0; the
         * offset (1, -1) makes it (2, 0); (-8, 7) clamps to (0, 3); at level
         * 1 (lod 1.0, 2x2) u = 0.6, texel (0, 0), and (1, 0) is one texel
         * of THAT level */
        let look = |lod: f32, off: Option<[i8; 2]>| {
            let (code, unif) = tex_program(42, 0.3, None, Some(lod), false, 0.3);
            let (code, unif) = match off {
                Some(off) => {
                    tex_program_bits(42, unif[0], None, None, Some(lod.to_bits()), Some(p2(false, off)), unif[0])
                }
                None => (code, unif),
            };
            fs_with(&code, &unif, indexed_texture())
        };
        assert_eq!(tlb_r(&look(0.0, None).expect("runs")), 11.0);
        assert_eq!(tlb_r(&look(0.0, Some([1, -1])).expect("runs")), 2.0);
        assert_eq!(tlb_r(&look(0.0, Some([-8, 7])).expect("runs")), 30.0, "CLAMP_TO_EDGE");
        assert_eq!(tlb_r(&look(1.0, None).expect("runs")), 100.0);
        assert_eq!(tlb_r(&look(1.0, Some([1, 0])).expect("runs")), 101.0, "one texel of level 1");
        /* REPEAT: texel 1 less 2 wraps to 3 */
        use crate::shaderc::online::texref::{Filter, MinFilter, Wrap};
        let repeat =
            Sampler { mag: Filter::Nearest, min: MinFilter::Nearest, wrap_s: Wrap::Repeat, wrap_t: Wrap::Repeat };
        let (code, unif) =
            tex_program_bits(33, 0.3f32.to_bits(), None, None, None, Some(p2(false, [-2, 0])), 0.3f32.to_bits());
        let flat = Texture::flat(4, 4, (0..16).map(|n| [(10 * (n / 4) + n % 4) as f32; 4]).collect());
        let o = fs_with_sampler(&code, &unif, flat, Some(repeat)).expect("runs");
        assert_eq!(tlb_r(&o), 13.0);
        /* refused: a cube with an offset; an offset under a LINEAR blend of
         * two levels */
        let (code, unif) = tex_program_bits(
            40,
            0.3f32.to_bits(),
            Some(0.2f32.to_bits()),
            None,
            Some(0),
            Some(p2(true, [1, 0])),
            1.0f32.to_bits(),
        );
        let e = fs_with(&code, &unif, mip_texture(true)).unwrap_err();
        assert!(e.contains("a cube lookup with a texel offset"), "{e}");
        let blend = Sampler {
            mag: Filter::Nearest,
            min: MinFilter::NearestMipmapLinear,
            wrap_s: Wrap::ClampToEdge,
            wrap_t: Wrap::ClampToEdge,
        };
        let (code, unif) = tex_program_bits(
            42,
            0.3f32.to_bits(),
            None,
            None,
            Some(0.5f32.to_bits()),
            Some(p2(false, [1, 0])),
            0.3f32.to_bits(),
        );
        let e = fs_with_sampler(&code, &unif, indexed_texture(), Some(blend)).unwrap_err();
        assert!(e.contains("a texel offset under a mipmap-LINEAR blend"), "{e}");
    }

    /// An explicit level of detail (TMUB, retired by TMUSLOD) reads the
    /// level dEQP picks; the same look-up retired by TMUS takes its level
    /// from the derivatives, which a texture of three levels cannot answer.
    #[test]
    fn an_explicit_lod_reads_its_level_and_an_implicit_one_is_refused() {
        for (lod, level) in [(0.2f32, 0.0f32), (0.9, 1.0), (1.8, 2.0), (6.0, 2.0), (-2.0, 0.0)] {
            let (code, unif) = tex_program(42, 0.3, None, Some(lod), false, 0.3);
            let o = fs_with(&code, &unif, mip_texture(false)).expect("runs");
            assert_eq!(tlb_r(&o), 10.0 * level, "LOD {lod}");
            assert_eq!(o.levels, 1 << level as u32);
        }
        let (code, unif) = tex_program(42, 0.3, None, Some(1.5), false, 0.3);
        let e = fs_with(&code, &unif, mip_texture(false)).unwrap_err();
        assert!(e.contains("within dEQP's LOD precision of a level boundary"), "{e}");
        let (code, unif) = tex_program(33, 0.3, None, Some(0.9), false, 0.3);
        let e = fs_with(&code, &unif, mip_texture(false)).unwrap_err();
        assert!(e.contains("an implicit level of detail on a texture of 3 levels"), "{e}");
    }

    /// A cube look-up (T, R, S by TMUSCM) reads the face its direction
    /// selects; T and R swapped select another face and read another value;
    /// P2's disable_autolod makes TMUB its level of detail.
    #[test]
    fn a_cube_lookup_reads_the_face_its_direction_selects() {
        let run = |t: f32, r: f32, s: f32, lod: f32| {
            let (code, unif) = tex_program(40, t, Some(r), Some(lod), true, s);
            fs_with(&code, &unif, mip_texture(true))
        };
        let o = run(-1.0, 0.3, 0.2, 0.0).expect("runs");
        assert_eq!((tlb_r(&o), o.cube_faces), (3.0, 1 << 3), "(0.2, -1, 0.3) is -Y");
        let o = run(0.3, -1.0, 0.2, 0.0).expect("runs");
        assert_eq!((tlb_r(&o), o.cube_faces), (5.0, 1 << 5), "(0.2, 0.3, -1) is -Z");
        let o = run(0.3, 0.2, 1.0, 0.9).expect("runs");
        assert_eq!(tlb_r(&o), 10.0, "(1, 0.3, 0.2) is +X, level 1");
        let e = run(1.0, 0.2, 1.0, 0.0).unwrap_err();
        assert!(e.contains("a cube direction within 1e-3 of a face edge"), "{e}");
        /* P2's op must be REGULAR (15): the same program with op 0 in P2
         * (an atomic, "Write ADD, Read Prefetch") is refused. */
        let (code, mut unif) = tex_program(40, -1.0, Some(0.3), Some(0.0), true, 0.2);
        let p2 = unif.iter().position(|&w| w == 0x00f0_0002).expect("the P2 word");
        unif[p2] = 0x2;
        let e = fs_with(&code, &unif, mip_texture(true)).unwrap_err();
        assert!(e.contains("TMU P2 0x00000002 is not a regular lookup"), "{e}");
        let (code, unif) = tex_program(40, 0.3, Some(0.2), Some(0.0), true, 1.0);
        let e = fs_with(&code, &unif, mip_texture(false)).unwrap_err();
        assert!(e.contains("a cube lookup on 2D texture 0"), "{e}");
        let (code, unif) = tex_program(33, 0.3, Some(0.2), None, false, 1.0);
        let e = fs_with(&code, &unif, mip_texture(true)).unwrap_err();
        assert!(e.contains("a 3D lookup (an R coordinate without TMUSCM)"), "{e}");
    }

    /// A 4x4 cube of one level: face +X's texel (i, j) is (1, i/4, j/4, 1),
    /// face -Z (which adjoins +X's s = 1 edge) is flat (0, 0, 1, 1), the
    /// other faces flat (f/8, f/8, f/8, 1). Every blend of these is exact in
    /// f16.
    fn edge_cube() -> Texture {
        use crate::shaderc::online::texref::{Level, MinFilter};
        let n = 4usize;
        let faces: Vec<Vec<[f32; 4]>> = (0..6)
            .map(|f| match f {
                0 => (0..n * n).map(|k| [1.0, (k % n) as f32 / 4.0, (k / n) as f32 / 4.0, 1.0]).collect(),
                5 => vec![[0.0, 0.0, 1.0, 1.0]; n * n],
                _ => vec![[f as f32 / 8.0; 4]; n * n],
            })
            .collect();
        Texture { cube: true, min: MinFilter::Linear, levels: vec![Level { width: 4, height: 4, faces }] }
    }

    /// The four f16 channels of the first TLB write's lane 0 (two words).
    fn tlb_rgba16(o: &Outcome) -> [f32; 4] {
        let w = |k: usize| o.tlb[k].value[0].expect("a defined value");
        let h = |x: u32| half_to_f32(x as u16);
        [h(w(0) & 0xffff), h(w(0) >> 16), h(w(1) & 0xffff), h(w(1) >> 16)]
    }

    /// A LINEAR look-up near +X's s = 1 edge, (1, 0, -0.875): on the +X face
    /// u = 3.75, so the taps are columns 3 and 4 weighed 3:1, and rows 1 and
    /// 2 equally. By TMUSCM (a cube lookup) the column-4 tap reads -Z's edge
    /// column, whatever the wrap mode: the seamless blend (0.75, 0.5625,
    /// 0.53125, 1). By TMUS with TMUI = 0 (a 2D-array lookup of face 0 on
    /// the same texture) the tap wraps as the sampler says: CLAMP (1, 0.75,
    /// 0.375, 1), REPEAT (1, 0.5625, 0.375, 1), MIRROR as CLAMP. A P1
    /// naming no sampler entry is NEAREST: the texel (3, 2), (1, 0.75, 0.5, 1).
    /// This is the seam the non-seamless emulation removes
    /// (docs/v3d-noseamless-cube.md; tests/v3d_cube_noseamless.rs drives the
    /// compiler's own code through the same model).
    #[test]
    fn a_cube_lookup_is_seamless_and_an_array_lookup_of_its_face_is_not() {
        use crate::shaderc::online::texref::{Filter, MinFilter, Wrap};
        let sampler = |wrap: Wrap| Sampler { mag: Filter::Linear, min: MinFilter::Linear, wrap_s: wrap, wrap_t: wrap };
        /* P0 word mask 0x3: two f16 words, 16-bit output (P1 without bit 0) */
        let rgba16 = |unif: &mut Vec<u32>| {
            unif[1] = 0x3;
            unif[2] = 0x0;
        };
        let (t, s) = (0.0f32, 1.0f32);
        let z = -0.875f32;
        for wrap in [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat] {
            let (mut code, mut unif) = tex_program(40, t, Some(z), Some(0.0), true, s);
            rgba16(&mut unif);
            /* the second word (b, a) to the TLB too */
            code.extend([ldtmu_r1(), mov_mul(7, true, Mux::R1)]);
            let o = fs_with_sampler(&code, &unif, edge_cube(), Some(sampler(wrap))).expect("runs");
            assert_eq!(tlb_rgba16(&o), [0.75, 0.5625, 0.53125, 1.0], "seamless under {wrap:?}");
            assert_eq!(o.cube_faces, 1 << 0, "+X selected");
            /* the same direction's face coordinates, as a 2D-array lookup of
             * layer 0: s = (sc / ma + 1) / 2 with sc = -z, t = 0.5 */
            let (mut code, mut unif) = tex_program_layer(33, 0.5, None, Some(0.0), None, false, (-z + 1.0) / 2.0);
            rgba16(&mut unif);
            code.extend([ldtmu_r1(), mov_mul(7, true, Mux::R1)]);
            let o = fs_with_sampler(&code, &unif, edge_cube(), Some(sampler(wrap))).expect("runs");
            let want = match wrap {
                Wrap::Repeat => [1.0, 0.5625, 0.375, 1.0],
                _ => [1.0, 0.75, 0.375, 1.0],
            };
            assert_eq!(tlb_rgba16(&o), want, "non-seamless under {wrap:?}");
            assert_eq!(o.cube_faces, 1 << 0, "face 0 read as a layer");
        }
        /* layer 5.4 rounds to -Z, flat blue */
        let (code, unif) = tex_program_layer(33, 0.5, None, Some(5.4), None, false, 0.9375);
        let o = fs_with_sampler(&code, &unif, edge_cube(), Some(sampler(Wrap::Repeat))).expect("runs");
        assert_eq!((tlb_r(&o), o.cube_faces), (0.0, 1 << 5), "the 32-bit R word of -Z");
        /* no sampler entry: NEAREST on the texel (3, 2), at t = 0.625 (t =
         * 0.5 is the edge of rows 1 and 2, refused under NEAREST) */
        let (mut code, mut unif) = tex_program_layer(33, 0.625, None, Some(0.0), None, false, 0.9375);
        rgba16(&mut unif);
        code.extend([ldtmu_r1(), mov_mul(7, true, Mux::R1)]);
        let o = fs_with_sampler(&code, &unif, edge_cube(), None).expect("runs");
        assert_eq!(tlb_rgba16(&o), [1.0, 0.75, 0.5, 1.0]);
        /* a cube lookup with a layer is a cube array: refused */
        let (code, unif) = tex_program_layer(40, t, Some(z), Some(0.0), Some(0.0), true, s);
        let e = fs_with(&code, &unif, edge_cube()).unwrap_err();
        assert!(e.contains("a cube lookup with an array index (TMUI)"), "{e}");
        /* a layer that is not finite */
        let (code, unif) = tex_program_layer(33, 0.5, None, Some(f32::NAN), None, false, 0.5);
        let e = fs_with(&code, &unif, edge_cube()).unwrap_err();
        assert!(e.contains("a 2D-array layer that is not finite"), "{e}");
    }

    /// An LDTMU of the next word into r1.
    fn ldtmu_r1() -> Alu {
        let mut ldtmu = Alu::nop();
        ldtmu.sig.ldtmu = true;
        ldtmu.sig_addr = 1;
        ldtmu.sig_magic = true;
        ldtmu
    }

    /// A 2D look-up: WRTMUC takes P0 (then P1), T and S from r5 (0.3),
    /// the S write retires it.
    fn lookup() -> Vec<Alu> {
        let mut ld = Alu::nop();
        ld.sig.ldunif = true; // r5: a coordinate
        let mut c0 = Alu::nop();
        c0.sig.wrtmuc = true; // P0
        let mut c1 = mov_mul(34, true, Mux::R5); // TMUT
        c1.sig.wrtmuc = true; // P1
        let s = mov_mul(33, true, Mux::R5); // TMUS
                                            // The first THRSW's switch clears the accumulators at tick 3, so
                                            // r5 is loaded after it.
        vec![Alu::nop(), ld, c0, c1, s]
    }

    /// Mesa's peripheral pairing (qpu_schedule.c:854-930): a WRTMUC with an
    /// SFU write or a VPM read is refused (the Pi 4's L2T faults, v3d-m5),
    /// with a TMUC write too; a WRTMUC with any other TMU write and an LDTMU
    /// with a VPM read or write are allowed.
    #[test]
    fn a_wrtmuc_shares_its_word_only_with_a_tmu_write() {
        let with = |mut a: Alu, wrtmuc: bool, ldtmu: bool| {
            a.sig.wrtmuc = wrtmuc;
            if ldtmu {
                a.sig.ldtmu = true;
                a.sig_addr = 0;
                a.sig_magic = true;
            }
            a
        };
        let ok = |a: &Alu| peripherals_compatible(peripherals(a), a);
        let mut ldvpm = Alu::nop();
        ldvpm.add.op = AddOp::LdvpmvIn;
        ldvpm.add.waddr = 0;
        ldvpm.add.magic = true;
        assert!(!ok(&with(mov_mul(19, true, Mux::R5), true, false)), "WRTMUC + an SFU write (recip)");
        assert!(!ok(&with(ldvpm, true, false)), "WRTMUC + LDVPMV_IN");
        assert!(!ok(&with(mov_mul(32, true, Mux::R5), true, false)), "WRTMUC + a TMUC write");
        assert!(ok(&with(mov_mul(34, true, Mux::R5), true, false)), "WRTMUC + TMUT: Mesa's own pairing");
        assert!(ok(&with(ldvpm, false, true)), "LDTMU + a VPM read");
        assert!(ok(&with(mov_mul(19, true, Mux::R5), false, false)), "one peripheral");
        assert_eq!(peripheral_names(peripherals(&with(ldvpm, true, false))), "VPM read + WRTMUC");
        /* and the interpreter stops such a word where a program has it */
        let e = fs_tex(&[Instr::Alu(with(mov_mul(19, true, Mux::R5), true, false))], &[0], 1).unwrap_err();
        assert!(e.contains("the peripherals SFU + WRTMUC in one instruction"), "{e}");
    }

    /// Mesa's TMU rules, each refused by name (qpu_validate.c, nir_to_vir.c
    /// ntq_flush_tmu and the output FIFO bound).
    #[test]
    fn the_tmu_rules_are_refused() {
        let unif = |p0: u32, p1: u32| vec![0.3f32.to_bits(), p0, p1];
        let ldtmu = || {
            let mut a = Alu::nop();
            a.sig.ldtmu = true;
            a.sig_addr = 0;
            a.sig_magic = true; // r0
            a
        };
        // Control: a look-up whose one result word is read runs.
        let mut ok = lookup();
        ok.push(ldtmu());
        fs_tex(&ok.iter().copied().map(Instr::Alu).collect::<Vec<_>>(), &unif(0x1, 0x0), 1)
            .expect("a clean look-up runs");
        // A result never read by an LDTMU.
        let e = fs_tex(&lookup().into_iter().map(Instr::Alu).collect::<Vec<_>>(), &unif(0x1, 0x0), 1).unwrap_err();
        assert!(e.contains("TMU results never read by an LDTMU at the thread end"), "{e}");
        // A request never retired (T written, no S).
        let e = fs_tex(&[Instr::Alu(mov_mul(34, true, Mux::R0))], &[], 1).unwrap_err();
        assert!(e.contains("an undefined lane") || e.contains("a TMU request not retired"), "{e}");
        let mut t_only = Vec::from(&lookup()[..4]);
        t_only.push(Alu::nop());
        let e = fs_tex(&t_only.into_iter().map(Instr::Alu).collect::<Vec<_>>(), &unif(0x1, 0x0), 1).unwrap_err();
        assert!(e.contains("a TMU request not retired at the thread end"), "{e}");
        // The output FIFO: 4 threads hold 16 / 4 = 4 words; two 32-bit
        // four-word look-ups make 8.
        let mut two = lookup();
        two.extend(&lookup()[2..]);
        let e =
            fs_tex(&two.into_iter().map(Instr::Alu).collect::<Vec<_>>(), &[0.3f32.to_bits(), 0xf, 0x1, 0xf, 0x1], 4)
                .unwrap_err();
        assert!(e.contains("past the output FIFO's 4 (16 / 4 threads)"), "{e}");
        // No thread count: refused, not assumed.
        let e = fs_tex(&ok.iter().copied().map(Instr::Alu).collect::<Vec<_>>(), &unif(0x1, 0x0), 0).unwrap_err();
        assert!(e.contains("a TMU look-up with no thread count given"), "{e}");
        // Two units in one instruction: a TMU write with an LDTMU.
        let mut both = mov_mul(34, true, Mux::R0);
        both.sig.ldtmu = true;
        both.sig_addr = 1;
        both.sig_magic = true;
        let e = fs_tex(&[Instr::Alu(both)], &[], 1).unwrap_err();
        assert!(e.contains("2 of TMU, SFU, TSY, TLB, VPM, LDTMU in one instruction"), "{e}");
    }

    /// An LDTMU that takes a word requested with no thread switch since is
    /// refused in a threaded program ([`NO_SWITCH_HANG`]) and counted with
    /// one thread. The same program with the switch replaced by a NOP is
    /// the control: the refusal follows the switch, not the distance.
    #[test]
    fn an_ldtmu_without_a_switch_since_its_request_is_refused_when_threaded() {
        let unif = [0.3f32.to_bits(), 0x1, 0x0];
        let mut ld = Alu::nop();
        ld.sig.ldtmu = true;
        ld.sig_addr = 0;
        ld.sig_magic = true; // r0
                             // Read in the next instruction, no switch: what hung the board.
        let mut next = lookup();
        next.push(ld);
        let next: Vec<Instr> = next.into_iter().map(Instr::Alu).collect();
        for threads in [4, 2] {
            let e = fs_tex(&next, &unif, threads).unwrap_err();
            assert!(e.contains(&format!("({threads} threads): {NO_SWITCH_HANG}")), "{e}");
        }
        // One thread: Mesa's own no-switch path, counted, not refused.
        let got = fs_tex(&next, &unif, 1).expect("runs");
        assert_eq!((got.ldtmus, got.ldtmus_without_switch), (1, 1));
        // A switch between the request and the LDTMU (Mesa's ntq_flush_tmu),
        // and the same program with that switch a NOP.
        let program = |mid: Alu| -> Result<Outcome, String> {
            let mut code = vec![thrsw(Alu::nop()), Alu::nop(), Alu::nop()];
            code.extend(lookup());
            code.extend([mid, Alu::nop(), Alu::nop(), ld]);
            code.extend([thrsw(Alu::nop()), thrsw(Alu::nop()), thrsw(Alu::nop()), Alu::nop(), Alu::nop()]);
            let words: Vec<u64> = code.iter().map(|i| encode(&Instr::Alu(*i)).expect("encodes")).collect();
            let half = [0.3f32.to_bits(); LANES];
            let inputs = FsInputs {
                w: half,
                w_centroid: half,
                z: half,
                x: half,
                y: half,
                back_facing: [false; LANES],
                varyings: Vec::new(),
            };
            let tex = Texture::flat(1, 1, vec![[0.5, 0.25, 0.125, 1.0]]);
            let env = Env { textures: vec![tex], threads: 4, ..Env::default() };
            run(&words, &unif, &Inputs::Fragment(Box::new(inputs)), &env)
        };
        let switched = program(thrsw(Alu::nop())).expect("runs");
        assert_eq!((switched.ldtmus, switched.ldtmus_without_switch), (1, 0));
        let e = program(Alu::nop()).unwrap_err();
        assert!(e.contains(NO_SWITCH_HANG), "{e}");
    }

    /// The thread end: no register-file write in its THRSW and two delay
    /// slots, no TMUWT last, no general TMU write left unwaited.
    #[test]
    fn the_thread_end_rules_are_refused() {
        let end = |slot: Alu| -> Result<Outcome, String> {
            let code = [thrsw(Alu::nop()), thrsw(Alu::nop()), thrsw(Alu::nop()), Alu::nop(), slot].map(Instr::Alu);
            let words: Vec<u64> = code.iter().map(|i| encode(i).expect("encodes")).collect();
            let one = [1.0f32.to_bits(); LANES];
            let inputs = FsInputs {
                w: one,
                w_centroid: one,
                z: one,
                x: one,
                y: one,
                back_facing: [false; LANES],
                varyings: Vec::new(),
            };
            run(&words, &[], &Inputs::Fragment(Box::new(inputs)), &Env::default())
        };
        end(Alu::nop()).expect("a clean end runs");
        let e = end(mov_mul(3, false, Mux::R0)).unwrap_err(); // rf3 = r0
        assert!(e.contains("a register-file write after the thread end"), "{e}");
        let mut wt = Alu::nop();
        wt.add.op = AddOp::Tmuwt;
        wt.add.waddr = WADDR_NOP;
        let e = end(wt).unwrap_err();
        assert!(e.contains("a TMUWT in the last instruction"), "{e}");
        // A general write (TMUD then TMUA) with no TMUWT before the end.
        let mut ld = Alu::nop();
        ld.sig.ldunif = true; // r5 = an aligned address
        let d = mov_mul(11, true, Mux::R5); // TMUD
        let a = mov_mul(12, true, Mux::R5); // TMUA
        let e = fs(&[Alu::nop(), ld, Alu::nop(), d, a].map(Instr::Alu), &[0x100]).unwrap_err();
        assert!(e.contains("a general TMU write not waited for by a TMUWT at the thread end"), "{e}");
    }

    /// A VPM store is a sink: the vertex shader's output.
    #[test]
    fn a_vpm_store_refuses_an_undefined_value() {
        let mut ld = Alu::nop();
        ld.sig.ldunif = true; // r5 = row 0
        let mut st = Alu::nop();
        st.add.op = AddOp::Stvpmv;
        st.add.a = Input { mux: Mux::R5, unpack: Unpack::None };
        st.add.b = Input { mux: Mux::A, unpack: Unpack::None };
        st.raddr_a = RF_NEVER_WRITTEN;
        let mut code = vec![Instr::Alu(thrsw(Alu::nop())), Instr::Alu(thrsw(Alu::nop()))];
        code.extend([ld, Alu::nop(), st].map(Instr::Alu));
        code.extend([Instr::Alu(thrsw(Alu::nop())), Instr::Alu(Alu::nop()), Instr::Alu(Alu::nop())]);
        let words: Vec<u64> = code.iter().map(|i| encode(i).expect("encodes")).collect();
        let e = run(&words, &[0], &Inputs::Vertex(VsInputs { vpm: Vec::new() }), &Env::default()).unwrap_err();
        assert!(e.contains("an undefined lane reaches a VPM store"), "{e}");
    }

    /// A & X is false when X is known false, whatever the undefined A:
    /// Mesa's `mov.andz -, rf11` clears an inactive lane's condition this
    /// way, and a write under it then does not happen.
    #[test]
    fn and_with_a_known_false_clears_an_undefined_flag() {
        let mut cmp = Alu::nop();
        cmp.add.op = AddOp::Fcmp;
        cmp.add.a = Input { mux: Mux::A, unpack: Unpack::None };
        cmp.add.b = Input { mux: Mux::A, unpack: Unpack::None };
        cmp.raddr_a = RF_NEVER_WRITTEN;
        cmp.flags.apf = Pf::PushZ; // A undefined in every lane
        let mut ld = Alu::nop();
        ld.sig.ldunif = true; // r5 = 2.0, next instruction
        let mut and = mov_mul(WADDR_NOP, true, Mux::R5);
        and.flags.muf = Uf::AndZ; // A = A & (r5 == 0) = false
        let keep = mov_mul(1, true, Mux::R5); // r1 (magic 1) = 2.0
        let mut cond = mov_mul(1, true, Mux::A); // r1 = undefined, if A
        cond.flags.mc = Cond::IfA;
        let out = mov_mul(7, true, Mux::R1);
        let body = [cmp, ld, and, keep, cond, out].map(Instr::Alu);
        let o = fs(&body, &[2.0f32.to_bits()]).expect("runs");
        assert_eq!(o.tlb[0].value, [Some(2.0f32.to_bits()); LANES], "the write under a false A must not happen");
        // Control: with no AND the condition stays undefined, and so does r1.
        let body = [cmp, ld, keep, cond, out].map(Instr::Alu);
        let o = fs(&body, &[2.0f32.to_bits()]).expect("runs");
        assert!(o.tlb[0].value.iter().all(Option::is_none), "{:?}", o.tlb[0].value);
    }
}
