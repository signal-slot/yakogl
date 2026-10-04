// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// Ported from Mesa 26.2.1 src/imagination/pco/pco_map.py (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Groups made of several operations wired through the internal registers:
//! min, max, the boolean compares, the select.
//!
//! Each is a [`Template`] transcribed from Mesa `src/imagination/pco/pco_map.py`
//! (MIT, read and transcribed): the operations with their phases and field
//! values, which kernel operand goes to which of the six source slots, the
//! `is0` mux and the ISS routing. Field values are written by the NAME the
//! generated enum gives them, resolved through the field's own enum, so the
//! template reads against `pco_map.py` and the listing, not against numbers.
//!
//! The wiring, for reading the templates: phase 0 and phase 1 `mbyp` the
//! lower section's `s0` into `ft0` and the upper section's `s3` into `ft1`;
//! `is0` puts one lower slot on the feed-through `fte`; the `tst` compares
//! what `is1` and `is2` select and leaves the answer in `ftt`; `movc` writes
//! the destination with `movw0`'s internal register when `ftt` is set and
//! with what `is4` selects when it is not.
//!
//! What the corpus says about each is checked by the chain gate
//! (`tests/usc_limits.rs`): every group built from a template must have a
//! skeleton pco emitted, and the program must compute what pco's does.

use super::assemble::{close, BuildError, Parts};
use super::decode::{Group, MAX_INSTRS};
use super::instr::instr_region;
use super::isa::{bit_set, BitSetId, Region, PH_0, PH_1, PH_2, PH_2_MOV, PH_2_PCK, PH_2_TST};
use super::isa_tables::ENUMS;
use super::legalise::{choose_dst_variant, choose_src_variant, fill_dsts, fill_srcs};
use super::operands::Operand;
use super::sel::{MachineOp, Mods, SelOp, Trig};
use crate::shaderc::online::ir::AluOp;

/// One operation of a template.
pub struct Step {
    pub op: &'static str,
    pub phase: u8,
    /// (field, value name in the field's enum, or a decimal number).
    pub fields: &'static [(&'static str, &'static str)],
}

/// What a source slot carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arg {
    /// The kernel operation's operand `i`.
    Op(usize),
    /// A special register: a constant the template needs (0 is `pco_zero`,
    /// 143 is `pco_true`, all ones).
    Special(u16),
}

pub struct Template {
    pub name: &'static str,
    /// A bitwise-pipeline group (header `opcnt`, no ISS section) rather
    /// than a main-ALU one (header `oporg`).
    pub bitwise: bool,
    /// Whether the group writes a destination (`w0p` and a dst section).
    pub dst: bool,
    /// Which destination it writes: `w0`, or `w1` (`fred` part A).
    pub dst_w1: bool,
    /// Header fields beyond the phase set and `w0p`, by name.
    pub header: &'static [(&'static str, &'static str)],
    pub steps: &'static [Step],
    /// By true slot, s0..s5.
    pub slots: [Option<Arg>; 6],
    /// The lower section's `is0`, by name; `None` leaves it at `s0`.
    pub mux: Option<&'static str>,
    /// ISS fields that differ from zero, by name.
    pub iss: &'static [(&'static str, &'static str)],
}

const fn mbyp(phase: u8) -> Step {
    Step { op: "mbyp", phase, fields: &[] }
}

/// `group_map(O_MIN)` / `group_map(O_MAX)`: `ftt = ft0 <op> ft1`,
/// `dest = ftt ? ft0 : ft1`.
macro_rules! minmax {
    ($name:literal, $tst:literal) => {
        minmax!($name, $tst, "f32")
    };
    ($name:literal, $tst:literal, $ty:literal) => {
        Template {
            name: $name,
            bitwise: false,
            dst: true,
            dst_w1: false,
            header: &[],
            steps: &[
                mbyp(PH_0),
                mbyp(PH_1),
                Step {
                    op: "tst",
                    phase: PH_2_TST,
                    fields: &[("tst_op", $tst), ("pwen", "0"), ("type", $ty), ("p2end", "1"), ("elem", "0")],
                },
                Step {
                    op: "movc",
                    phase: PH_2_MOV,
                    fields: &[("movw0", "ft0"), ("movw1", "ft0"), ("maskw0", "eall"), ("p2end", "0")],
                },
            ],
            slots: [Some(Arg::Op(0)), None, None, Some(Arg::Op(1)), None, None],
            mux: None,
            iss: &[("is1", "ft0"), ("is2", "ft1"), ("is4", "ft1")],
        }
    };
}

pub const FMIN: Template = minmax!("min", "l");
pub const FMAX: Template = minmax!("max", "g");
/* The integer ones are the same group with the test's type (pco_map.py
 * group_map(O_MIN)/(O_MAX) take the type from the op: imin/imax s32,
 * umin/umax u32). */
pub const IMIN: Template = minmax!("imin", "l", "s32");
pub const IMAX: Template = minmax!("imax", "g", "s32");
pub const UMIN: Template = minmax!("umin", "l", "u32");
pub const UMAX: Template = minmax!("umax", "g", "u32");

/// `group_map(O_BCSEL)`: `ft0 = x`, `ft1 = y`, `ftt = (c == 0)` as u32 on the
/// feed-through, `dest = ftt ? ft1 : ft0` -- that is, `c ? x : y`.
pub const BCSEL: Template = Template {
    name: "bcsel",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        mbyp(PH_0),
        mbyp(PH_1),
        Step {
            op: "tst",
            phase: PH_2_TST,
            fields: &[("tst_op", "z"), ("pwen", "0"), ("type", "u32"), ("p2end", "1"), ("elem", "0")],
        },
        Step {
            op: "movc",
            phase: PH_2_MOV,
            fields: &[("movw0", "ft1"), ("movw1", "ft0"), ("maskw0", "eall"), ("p2end", "0")],
        },
    ],
    slots: [Some(Arg::Op(1)), Some(Arg::Op(0)), None, Some(Arg::Op(2)), None, None],
    mux: Some("s1"),
    iss: &[("is1", "fte"), ("is4", "ft0")],
};

/// `group_map(O_BCMP)`: `ftt = ft0 <op> ft1`, `ft2 = 0` (a `pck` of format
/// `zero`), `dest = ftt ? fte : ft2` with `fte` the all-ones constant on
/// `s1` -- the boolean as this machine and our selection represent it.
macro_rules! bcmp {
    ($name:literal, $tst:literal, $ty:literal) => {
        bcmp!($name, $tst, $ty, 0, 1)
    };
    ($name:literal, $tst:literal, $ty:literal, $a:literal, $b:literal) => {
        Template {
            name: $name,
            bitwise: false,
            dst: true,
            dst_w1: false,
            header: &[],
            steps: &[
                mbyp(PH_0),
                mbyp(PH_1),
                Step {
                    op: "tst",
                    phase: PH_2_TST,
                    fields: &[("tst_op", $tst), ("pwen", "0"), ("type", $ty), ("p2end", "0"), ("elem", "0")],
                },
                Step { op: "pck", phase: PH_2_PCK, fields: &[("pck_format", "zero"), ("scale", "0"), ("rtz", "0")] },
                Step {
                    op: "movc",
                    phase: PH_2_MOV,
                    fields: &[("movw0", "fte"), ("movw1", "ft0"), ("maskw0", "eall"), ("p2end", "0")],
                },
            ],
            slots: [Some(Arg::Op($a)), Some(Arg::Special(143)), None, Some(Arg::Op($b)), None, None],
            mux: Some("s1"),
            iss: &[("is1", "ft0"), ("is2", "ft1"), ("is4", "ft2")],
        }
    };
}

pub const FLT: Template = bcmp!("flt", "l", "f32");
/* NIR has no "less or equal": `a <= b` reaches pco as `b >= a`, and no
 * compare in the corpus uses `le`. So the operands swap and the test is
 * `ge` -- exact for every float, NaN included (both answer false). */
pub const FLE: Template = bcmp!("fle", "ge", "f32", 1, 0);
pub const FEQ: Template = bcmp!("feq", "e", "f32");
pub const FNE: Template = bcmp!("fne", "ne", "f32");
pub const ILT: Template = bcmp!("ilt", "l", "s32");
pub const ILE: Template = bcmp!("ile", "ge", "s32", 1, 0);
pub const IEQ: Template = bcmp!("ieq", "e", "s32");
pub const INE: Template = bcmp!("ine", "ne", "s32");
/* ult/ule: the signed compares' group with the test's type u32 */
pub const ULT: Template = bcmp!("ult", "l", "u32");
pub const ULE: Template = bcmp!("ule", "ge", "u32", 1, 0);

/// `group_map(O_IADD32)`: `imadd32(a, 1, b)`, pco's constant one riding
/// `s1` (special register 1); signed off, as `pco_iadd32` leaves it.
pub const IADD: Template = Template {
    name: "iadd",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "imadd32", phase: PH_0, fields: &[("s", "0"), ("s2neg", "0")] }],
    slots: [Some(Arg::Op(0)), Some(Arg::Special(1)), Some(Arg::Op(1)), None, None, None],
    mux: None,
    iss: &[],
};

/// `group_map(O_IMUL32)`: `imadd32(a, b, 0)`, the zero on `s2`.
pub const IMUL: Template = Template {
    name: "imul",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "imadd32", phase: PH_0, fields: &[("s", "0"), ("s2neg", "0")] }],
    slots: [Some(Arg::Op(0)), Some(Arg::Op(1)), Some(Arg::Special(0)), None, None, None],
    mux: None,
    iss: &[],
};

/// `trans_conv` for `i2f32`: `unpck` of format s32 (element 0, no scale).
pub const I2F: Template = Template {
    name: "i2f",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "unpck", phase: PH_0, fields: &[("elem", "e0"), ("scale_rtz", "0"), ("pck_format", "s32")] }],
    slots: [Some(Arg::Op(0)), None, None, None, None, None],
    mux: None,
    iss: &[],
};

/// `trans_conv` for `f2i32`: a `pck` of format s32 with round-to-zero (GLSL
/// `int(x)` truncates), wired as every phase-2 pack is -- `is0 = s2` onto the
/// feed-through, `is3 = fte` into the packer, `is4 = ft2` out -- with pco's
/// placeholder zeros in `s0` and `s1`.
pub const F2I: Template = Template {
    name: "f2i",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step {
        op: "pck",
        phase: PH_2_PCK,
        fields: &[("pck_format", "s32"), ("rtz", "1"), ("scale", "0"), ("prog", "0")],
    }],
    slots: [Some(Arg::Special(0)), Some(Arg::Special(0)), Some(Arg::Op(0)), None, None, None],
    mux: Some("s2"),
    iss: &[("is3", "fte"), ("is4", "ft2")],
};

/// `trans_nir`'s `ineg`: `iadd32(-a, 0)` signed, which group_map(O_IADD32)
/// makes `imadd32(-a, 1, 0)`.
pub const INEG: Template = Template {
    name: "ineg",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "imadd32", phase: PH_0, fields: &[("s", "1"), ("s0neg", "1"), ("s2neg", "0")] }],
    slots: [Some(Arg::Op(0)), Some(Arg::Special(1)), Some(Arg::Special(0)), None, None, None],
    mux: None,
    iss: &[],
};

/// `fsat`: `fadd(x, 0.0)` with the result saturated to [0, 1], as pco
/// clamps a blend's source colour (its blend variants' first groups:
/// `fadd sat=1`, s1 special 0) and builds NIR's `fsat` (`clamp(x, 0.0,
/// 1.0)` in a shader: `MachineOp::Fsat`).
pub const FSAT: Template = Template {
    name: "fsat",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "fadd", phase: PH_0, fields: &[("sat", "1")] }],
    slots: [Some(Arg::Op(0)), Some(Arg::Special(0)), None, None, None, None],
    mux: None,
    iss: &[],
};

/// `trans_nir`'s `iabs`: `iadd32(|a|, 0)` signed -- `imadd32(|a|, 1, 0)`
/// with `s0abs`, as [`INEG`] is with `s0neg`. The groups below are pco's
/// program for `int / int` (pco_shc build-new on an ES 1.00 source,
/// 2026-09-23: `nir_lower_idiv`, then `trans_nir`): this one at 0x20.
pub const IABS: Template = Template {
    name: "iabs",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "imadd32", phase: PH_0, fields: &[("s", "1"), ("s0abs", "1"), ("s2neg", "0")] }],
    slots: [Some(Arg::Op(0)), Some(Arg::Special(1)), Some(Arg::Special(0)), None, None, None],
    mux: None,
    iss: &[],
};

/// `trans_conv` for `u2f32`: [`I2F`]'s `unpck` of format u32 (0x2c).
pub const U2F: Template = Template {
    name: "u2f",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "unpck", phase: PH_0, fields: &[("elem", "e0"), ("scale_rtz", "0"), ("pck_format", "u32")] }],
    slots: [Some(Arg::Op(0)), None, None, None, None, None],
    mux: None,
    iss: &[],
};

/// `trans_conv` for `f2u32`: [`F2I`]'s round-to-zero `pck` of format u32
/// (0x54).
pub const F2U: Template = Template {
    name: "f2u",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step {
        op: "pck",
        phase: PH_2_PCK,
        fields: &[("pck_format", "u32"), ("rtz", "1"), ("scale", "0"), ("prog", "0")],
    }],
    slots: [Some(Arg::Special(0)), Some(Arg::Special(0)), Some(Arg::Op(0)), None, None, None],
    mux: Some("s2"),
    iss: &[("is3", "fte"), ("is4", "ft2")],
};

/// `uge(a, b)` as a boolean: the compare group with `tst ge u32` (0xce).
pub const UGE: Template = bcmp!("uge", "ge", "u32");

/// `trans_nir`'s `umul_high`: `imadd64(a, b, 0, 0)` writing only the high
/// word, w1 (0x7a: `w1p`, `is0 = s3`, `is5 = fte`).
pub const UMULHI: Template = Template {
    name: "umul_high",
    bitwise: false,
    dst: true,
    dst_w1: true,
    header: &[],
    steps: &[Step { op: "imadd64", phase: PH_0, fields: &[("s", "0"), ("s2neg", "0")] }],
    slots: [Some(Arg::Op(0)), Some(Arg::Op(1)), Some(Arg::Special(0)), Some(Arg::Special(0)), None, None],
    mux: Some("s3"),
    iss: &[("is5", "fte")],
};

/// `group_map(O_FSIGN)`: `ft0 = sat(x * +inf)` (1.0 for a positive x),
/// `ft1 = -1.0` (the constant one, negated, in phase 1), a `tst` of x's bits
/// as s32 against zero (`gez`: the sign bit is clear), and `movc` taking ft0
/// when it is and ft1 when not. That is pco's sign, and -0.0 gives -1.0 on
/// it where GLSL says 0.0; transcribed as pco has it.
pub const FSIGN: Template = Template {
    name: "fsign",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        Step { op: "fmul", phase: PH_0, fields: &[("sat", "1")] },
        Step { op: "mbyp", phase: PH_1, fields: &[("s0neg", "1")] },
        Step {
            op: "tst",
            phase: PH_2_TST,
            fields: &[("tst_op", "gez"), ("pwen", "0"), ("type", "s32"), ("p2end", "1"), ("elem", "0")],
        },
        Step {
            op: "movc",
            phase: PH_2_MOV,
            fields: &[("movw0", "ft0"), ("movw1", "ft0"), ("maskw0", "eall"), ("p2end", "0")],
        },
    ],
    slots: [Some(Arg::Op(0)), Some(Arg::Special(142)), None, Some(Arg::Special(64)), None, None],
    mux: None,
    iss: &[("is1", "fte"), ("is4", "ft1")],
};

/// `group_map(O_LOGICAL)` as `trans_logical` fills it: `bbyp0s1` passes
/// the first operand through phase 0 (onto `ft2`), `logical` combines it
/// with `s3` in phase 1 and writes `w0`. The first operand rides `s2`, the
/// second `s3`; `s0` and `s1` carry pco's zero placeholders. A boolean here
/// is all ones or zero, so `or`/`and`/`xor` of two booleans is the boolean
/// operation.
macro_rules! logical {
    ($name:literal, $op:literal) => {
        Template {
            name: $name,
            bitwise: true,
            dst: true,
            dst_w1: false,
            header: &[],
            steps: &[
                Step { op: "bbyp0s1", phase: PH_0, fields: &[] },
                Step { op: "logical", phase: PH_1, fields: &[("logical_op", $op), ("mska", "0"), ("mskb", "0")] },
            ],
            slots: [Some(Arg::Special(0)), Some(Arg::Special(0)), Some(Arg::Op(0)), Some(Arg::Op(1)), None, None],
            mux: None,
            iss: &[],
        }
    };
}

pub const IOR: Template = logical!("or", "or");
pub const IAND: Template = logical!("and", "and");
pub const IXOR: Template = logical!("xor", "xor");
/// `inot(x)`: pco's `xnor` of x with zero (pco_trans_nir.c trans_logical,
/// nir_op_inot: PCO_LOGIOP_XNOR, src1 = pco_zero).
pub const INOT: Template = Template {
    name: "inot",
    bitwise: true,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        Step { op: "bbyp0s1", phase: PH_0, fields: &[] },
        Step { op: "logical", phase: PH_1, fields: &[("logical_op", "xnor"), ("mska", "0"), ("mskb", "0")] },
    ],
    slots: [Some(Arg::Special(0)), Some(Arg::Special(0)), Some(Arg::Op(0)), Some(Arg::Special(0)), None, None],
    mux: None,
    iss: &[],
};

/// `group_map(O_UBFE)`: `msk_bbyp0s1` masks the value (s2) to `bits` (s0)
/// from `offset` (s1), `logical` ORs in zero with both masks, `shift`
/// shifts right by `offset` (s4). `ubfe(value, offset, bits)`.
pub const UBFE: Template = Template {
    name: "ubfe",
    bitwise: true,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        Step { op: "msk_bbyp0s1", phase: PH_0, fields: &[] },
        Step { op: "logical", phase: PH_1, fields: &[("logical_op", "or"), ("mska", "1"), ("mskb", "1")] },
        Step { op: "shift", phase: PH_2, fields: &[("shift2_op", "shr")] },
    ],
    slots: [Some(Arg::Op(2)), Some(Arg::Op(1)), Some(Arg::Op(0)), Some(Arg::Special(0)), Some(Arg::Op(1)), None],
    mux: None,
    iss: &[],
};

/// `group_map(O_SHIFT)` with `lsl`: `bbyp0bm` passes the value (s1) on,
/// `shift` shifts it left by s4. `ishl(value, amount)`.
pub const ISHL: Template = Template {
    name: "ishl",
    bitwise: true,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        Step { op: "bbyp0bm", phase: PH_0, fields: &[] },
        Step { op: "shift", phase: PH_2, fields: &[("shift2_op", "lsl")] },
    ],
    slots: [Some(Arg::Special(0)), Some(Arg::Op(0)), None, Some(Arg::Special(0)), Some(Arg::Op(1)), None],
    mux: None,
    iss: &[],
};

/// `group_map(O_SHIFT)` with `asr_twb`: [`ISHL`]'s group shifting right,
/// arithmetically -- pco's `ishr` (pco_trans_nir.c trans_shift:
/// PCO_SHIFTOP_ASR_TWB). `ishr(value, amount)`.
pub const ISHR: Template = Template {
    name: "ishr",
    bitwise: true,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        Step { op: "bbyp0bm", phase: PH_0, fields: &[] },
        Step { op: "shift", phase: PH_2, fields: &[("shift2_op", "asr_twb")] },
    ],
    slots: [Some(Arg::Special(0)), Some(Arg::Op(0)), None, Some(Arg::Special(0)), Some(Arg::Op(1)), None],
    mux: None,
    iss: &[],
};

/// `group_map(O_SHIFT)` with `shr`: [`ISHL`]'s group shifting right,
/// logically. pco's `fs_is_single_sampled` shifts the coverage mask of 1.0
/// right by one (fill_gradient.frag, group 0x5e: s1 the value, s4 special 1).
pub const USHR: Template = Template {
    name: "ushr",
    bitwise: true,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        Step { op: "bbyp0bm", phase: PH_0, fields: &[] },
        Step { op: "shift", phase: PH_2, fields: &[("shift2_op", "shr")] },
    ],
    slots: [Some(Arg::Special(0)), Some(Arg::Op(0)), None, Some(Arg::Special(0)), Some(Arg::Op(1)), None],
    mux: None,
    iss: &[],
};

/// `tstz` into a register: [`KILL_TEST`]'s group writing the all-ones or
/// zero boolean instead of p0 (`movc` of `fte`, the all-ones special 143, or
/// `ft2`, the packed zero). pco's `fs_is_single_sampled` (fill_gradient.frag,
/// group 0x68).
pub const TSTZ: Template = Template {
    name: "tstz",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        mbyp(PH_0),
        Step { op: "pck", phase: PH_2_PCK, fields: &[("pck_format", "zero"), ("scale", "0"), ("rtz", "0")] },
        Step {
            op: "tst",
            phase: PH_2_TST,
            fields: &[("tst_op", "z"), ("pwen", "0"), ("type", "u32"), ("p2end", "0"), ("elem", "0")],
        },
        Step {
            op: "movc",
            phase: PH_2_MOV,
            fields: &[("movw0", "fte"), ("movw1", "ft0"), ("maskw0", "eall"), ("p2end", "0")],
        },
    ],
    slots: [Some(Arg::Op(0)), Some(Arg::Special(143)), None, Some(Arg::Special(0)), None, None],
    mux: Some("s1"),
    iss: &[("is1", "ft0"), ("is2", "ft1"), ("is4", "ft2")],
};

/// `group_map(O_MOVS1)`: a move through S1, the one way pco reads a register
/// of the special class (`x_p`, `samp_num`, ...): `ref_src_map_valid` in
/// Mesa `pco_internal.h` maps them to S1/S2/S4 only, and `insert_mov_ref`
/// turns the move into this. `movwm` in phase 2 writes `fte`, which `is0`
/// takes from s1 (fill_gradient.frag, groups 0x7a and 0x86).
pub const MOVS1: Template = Template {
    name: "movs1",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "movwm", phase: PH_2_MOV, fields: &[("movw0", "fte"), ("maskw0", "eall"), ("p2end", "1")] }],
    slots: [Some(Arg::Special(0)), Some(Arg::Op(0)), None, Some(Arg::Special(0)), None, None],
    mux: Some("s1"),
    iss: &[("is4", "fte")],
};

/// `alpha_to_coverage`'s `pck.cov`: wired as every phase-2 pack, like
/// [`F2I`].
pub const PCK_COV: Template = Template {
    name: "pck.cov",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step {
        op: "pck",
        phase: PH_2_PCK,
        fields: &[("pck_format", "cov"), ("rtz", "0"), ("scale", "0"), ("prog", "0")],
    }],
    slots: [Some(Arg::Special(0)), Some(Arg::Special(0)), Some(Arg::Op(0)), None, None, None],
    mux: Some("s2"),
    iss: &[("is3", "fte"), ("is4", "ft2")],
};

/// `savmsk` in the backend phase, mode `vm`: the sample coverage mask,
/// with pco's zero placeholder in `s0`.
pub const SAVMSK: Template = Template {
    name: "savmsk.vm",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "savmsk", phase: super::isa::PH_BACKEND, fields: &[("msk_mode", "vm")] }],
    slots: [Some(Arg::Special(0)), None, None, None, None, None],
    mux: None,
    iss: &[],
};

/// `trans_nir`'s `isp_feedback` for a shader that discards, part one:
/// `tstz` of the kill flag into p0 (`pco_tstz(null, p0, flag)`), which
/// `group_map` builds as the compare shape -- `mbyp` the flag into ft0, a
/// `pck` of zero, `tst z` with the predicate write enabled, `movc` -- with
/// no destination. p0 is set when the fragment is kept.
pub const KILL_TEST: Template = Template {
    name: "tstz.p0",
    bitwise: false,
    dst: false,
    dst_w1: false,
    header: &[],
    steps: &[
        mbyp(PH_0),
        Step { op: "pck", phase: PH_2_PCK, fields: &[("pck_format", "zero"), ("scale", "0"), ("rtz", "0")] },
        Step {
            op: "tst",
            phase: PH_2_TST,
            fields: &[("tst_op", "z"), ("pwen", "1"), ("type", "u32"), ("p2end", "0"), ("elem", "0")],
        },
        Step {
            op: "movc",
            phase: PH_2_MOV,
            fields: &[("movw0", "fte"), ("movw1", "ft0"), ("maskw0", "eall"), ("p2end", "0")],
        },
    ],
    slots: [Some(Arg::Op(0)), Some(Arg::Special(143)), None, None, None, None],
    mux: Some("s1"),
    iss: &[("is1", "ft0"), ("is2", "ft1"), ("is4", "ft2")],
};

/// Part two: `alphaf(drc0, 0, 0, 7)` executed under p0 (`E1_Z1`, the
/// header's `if(p0)`), with the output-lock check pco sets: the pass
/// feedback the ISP waits for, given only for a fragment that is kept. A
/// `wdf` follows it.
pub const ALPHA_FEEDBACK: Template = Template {
    name: "alphaf",
    bitwise: false,
    dst: false,
    dst_w1: false,
    header: &[("cc", "if(p0)"), ("olchk", "1")],
    steps: &[Step { op: "alphaf", phase: super::isa::PH_BACKEND, fields: &[("pwen", "0")] }],
    slots: [Some(Arg::Special(0)), Some(Arg::Special(0)), Some(Arg::Special(7)), None, None, None],
    mux: None,
    iss: &[],
};

/// `group_map(O_FRED)` part A: `x` in `s0`, the result in `w1` (pco
/// leaves `w0` null), `iter` 0.
macro_rules! fred_a {
    ($ty:literal) => {
        Template {
            name: "fred.a",
            bitwise: false,
            dst: true,
            dst_w1: true,
            header: &[],
            steps: &[Step {
                op: "fred",
                phase: PH_0,
                fields: &[("red_part", "a"), ("iter", "0"), ("red_type", $ty), ("pwen", "0")],
            }],
            slots: [Some(Arg::Op(0)), None, None, Some(Arg::Special(0)), None, None],
            mux: None,
            iss: &[],
        }
    };
}

/// Part B: `x` in `s0`, part A's result in `s3`, the result in `w0`.
macro_rules! fred_b {
    ($ty:literal) => {
        Template {
            name: "fred.b",
            bitwise: false,
            dst: true,
            dst_w1: false,
            header: &[],
            steps: &[Step {
                op: "fred",
                phase: PH_0,
                fields: &[("red_part", "b"), ("iter", "0"), ("red_type", $ty), ("pwen", "0")],
            }],
            slots: [Some(Arg::Op(0)), None, None, Some(Arg::Op(1)), None, None],
            mux: None,
            iss: &[],
        }
    };
}

pub const FRED_A_SIN: Template = fred_a!("sin");
pub const FRED_A_COS: Template = fred_a!("cos");
pub const FRED_B_SIN: Template = fred_b!("sin");
pub const FRED_B_COS: Template = fred_b!("cos");

/// `group_map(O_FSINC)`: one `fsinc` of part B; it also writes p0, which
/// `psel_trig` reads.
pub const FSINC: Template = Template {
    name: "fsinc",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[Step { op: "fsinc", phase: PH_0, fields: &[] }],
    slots: [Some(Arg::Op(0)), None, None, None, None, None],
    mux: None,
    iss: &[],
};

/// `group_map(O_PSEL_TRIG)`: `imadd64` of zeros with p0 as the carry puts
/// p0 in ft0, `fmul` of the `fsinc` result (s3) and part B (s4) in ft1, a
/// `tst` for zero on ft0, and `movc` taking the `fsinc` result (fte, from
/// `is0 = s3`) when p0 is clear and the product when it is set.
pub const PSEL_TRIG: Template = Template {
    name: "psel_trig",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        Step { op: "imadd64", phase: PH_0, fields: &[("s", "0"), ("s2neg", "0"), ("cin", "1")] },
        Step { op: "fmul", phase: PH_1, fields: &[] },
        Step {
            op: "tst",
            phase: PH_2_TST,
            fields: &[("tst_op", "z"), ("pwen", "0"), ("type", "u32"), ("p2end", "1"), ("elem", "0")],
        },
        Step {
            op: "movc",
            phase: PH_2_MOV,
            fields: &[("movw0", "fte"), ("movw1", "ft0"), ("maskw0", "eall"), ("p2end", "0")],
        },
    ],
    slots: [
        Some(Arg::Special(0)),
        Some(Arg::Special(0)),
        Some(Arg::Special(0)),
        Some(Arg::Op(0)),
        Some(Arg::Op(1)),
        None,
    ],
    mux: Some("s3"),
    iss: &[("is1", "ft0"), ("is4", "ft1")],
};

/// The template a selection operation is built from, when it has one.
pub fn template_for(op: SelOp) -> Option<&'static Template> {
    match op {
        SelOp::Alu(a) => template_of(a),
        SelOp::Machine(MachineOp::Ubfe) => Some(&UBFE),
        SelOp::Machine(MachineOp::PckCov) => Some(&PCK_COV),
        SelOp::Machine(MachineOp::SavMsk) => Some(&SAVMSK),
        SelOp::Machine(MachineOp::FredA(Trig::Sin)) => Some(&FRED_A_SIN),
        SelOp::Machine(MachineOp::FredA(Trig::Cos)) => Some(&FRED_A_COS),
        SelOp::Machine(MachineOp::FredB(Trig::Sin)) => Some(&FRED_B_SIN),
        SelOp::Machine(MachineOp::FredB(Trig::Cos)) => Some(&FRED_B_COS),
        SelOp::Machine(MachineOp::Fsinc(_)) => Some(&FSINC),
        SelOp::Machine(MachineOp::PselTrig) => Some(&PSEL_TRIG),
        SelOp::Machine(MachineOp::LoopTest) => Some(&KILL_TEST),
        SelOp::Machine(MachineOp::Ushr) => Some(&USHR),
        SelOp::Machine(MachineOp::Tstz) => Some(&TSTZ),
        SelOp::Machine(MachineOp::MovS1) => Some(&MOVS1),
        SelOp::Machine(MachineOp::Iabs) => Some(&IABS),
        SelOp::Machine(MachineOp::Fsat) => Some(&FSAT),
        SelOp::Machine(MachineOp::U2f) => Some(&U2F),
        SelOp::Machine(MachineOp::F2u) => Some(&F2U),
        SelOp::Machine(MachineOp::Uge) => Some(&UGE),
        SelOp::Machine(MachineOp::UmulHi) => Some(&UMULHI),
        _ => None,
    }
}

/// `group_map(O_FCEIL)`: `ft0 = floor(x) + 1.0`, `ft1 = floor(x) + 0.0`,
/// a `tst` of x (fte, `is0 = s0`) against ft1 for equality, and `movc`
/// taking ft1 when x is already whole and ft0 when not.
pub const FCEIL: Template = Template {
    name: "fceil",
    bitwise: false,
    dst: true,
    dst_w1: false,
    header: &[],
    steps: &[
        Step { op: "fadd", phase: PH_0, fields: &[("s0flr", "1")] },
        Step { op: "fadd", phase: PH_1, fields: &[("s0flr", "1")] },
        Step {
            op: "tst",
            phase: PH_2_TST,
            fields: &[("tst_op", "e"), ("pwen", "0"), ("type", "f32"), ("p2end", "1"), ("elem", "0")],
        },
        Step {
            op: "movc",
            phase: PH_2_MOV,
            fields: &[("movw0", "ft1"), ("movw1", "ft0"), ("maskw0", "eall"), ("p2end", "0")],
        },
    ],
    slots: [Some(Arg::Op(0)), Some(Arg::Special(64)), None, Some(Arg::Op(0)), Some(Arg::Special(0)), None],
    mux: Some("s0"),
    iss: &[("is1", "fte"), ("is2", "ft1"), ("is4", "ft0")],
};

/// The template a kernel operation is built from, when it has one.
pub fn template_of(op: AluOp) -> Option<&'static Template> {
    use AluOp::*;
    Some(match op {
        Fmin => &FMIN,
        Fmax => &FMAX,
        Bcsel => &BCSEL,
        Flt => &FLT,
        Fle => &FLE,
        Feq => &FEQ,
        Fne => &FNE,
        Ilt => &ILT,
        Ile => &ILE,
        Ieq => &IEQ,
        Ine => &INE,
        Iadd => &IADD,
        Imul => &IMUL,
        I2f => &I2F,
        F2i => &F2I,
        Ineg => &INEG,
        Fsign => &FSIGN,
        Ior => &IOR,
        Iand => &IAND,
        Ixor => &IXOR,
        Inot => &INOT,
        Ult => &ULT,
        Ule => &ULE,
        Imin => &IMIN,
        Imax => &IMAX,
        Umin => &UMIN,
        Umax => &UMAX,
        Ushr => &USHR,
        U2f => &U2F,
        F2u => &F2U,
        Ishl => &ISHL,
        Ishr => &ISHR,
        Fceil => &FCEIL,
        _ => return None,
    })
}

/// A field value by name: the name in the field's enum, or a number.
pub(super) fn value(set: BitSetId, field: &str, name: &str) -> Result<u32, BuildError> {
    if let Ok(n) = name.parse::<u32>() {
        return Ok(n);
    }
    let bs = bit_set(set);
    bs.variants
        .iter()
        .flat_map(|v| v.uses.iter())
        .filter(|u| u.name == field)
        .filter_map(|u| bs.fields[u.field].enum_id)
        .find_map(|e| ENUMS[e].elems.iter().find(|(_, n)| *n == name).map(|(v, _)| *v))
        .ok_or(BuildError::Unsupported("a template value its field's enum does not have"))
}

/// Build `t` with `ops` as the kernel operation's operands (already
/// hardware operands) and `dst` as the destination.
pub fn build(t: &Template, ops: &[Option<Operand>; 3], dst: Operand) -> Result<Group, BuildError> {
    build_opt(t, ops, t.dst.then_some(dst))
}

/// [`build`] for a template that may write no destination.
pub fn build_opt(t: &Template, ops: &[Option<Operand>; 3], dst: Option<Operand>) -> Result<Group, BuildError> {
    if dst.is_some() != t.dst {
        return Err(BuildError::Unsupported("a destination the template does not take, or none where it does"));
    }
    let m = [Mods::default(); 3];
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    let mut phases = [0u8; MAX_INSTRS];
    for (k, st) in t.steps.iter().enumerate() {
        let mut extra: [(&'static str, u32); 8] = [("", 0); 8];
        for (i, (f, v)) in st.fields.iter().enumerate() {
            let set = if st.phase == super::isa::PH_BACKEND {
                BitSetId::Backend
            } else if t.bitwise {
                BitSetId::Bitwise
            } else {
                BitSetId::Main
            };
            extra[i] = (f, value(set, f, v)?);
        }
        let r = instr_region(st.op, st.phase, &m, &extra[..st.fields.len()]).map_err(BuildError::Form)?;
        let slot = super::pack::slot_of(st.phase)
            .ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: st.phase }))?;
        instrs[slot as usize] = Some((st.phase, r));
        phases[k] = st.phase;
    }

    let mut srcs: [Option<Operand>; 6] = [None; 6];
    for (s, a) in srcs.iter_mut().zip(t.slots.iter()) {
        *s = match a {
            None => None,
            Some(Arg::Special(i)) => Some(Operand { bank: 0, index: *i as u32 }),
            Some(Arg::Op(i)) => Some(ops[*i].ok_or(BuildError::Unsupported("a template operand that is missing"))?),
        };
    }
    let mux = match t.mux {
        Some(n) => value(BitSetId::Src, "is0", n)?,
        None => 0,
    };
    let lower_ops = [srcs[0], srcs[1], srcs[2], None, None, None];
    let lv = choose_src_variant(&lower_ops, mux).map_err(BuildError::Fit)?;
    let mut lower = Region::new(BitSetId::Src, lv);
    fill_srcs(&mut lower, &lower_ops, mux).map_err(BuildError::Fit)?;
    /* The upper section: the template's operand if it has one, and the
     * narrowest empty section otherwise, as every other group carries. */
    let upper_ops = [None, None, None, srcs[3], srcs[4], srcs[5]];
    let upper = if upper_ops.iter().any(Option::is_some) {
        let uv = choose_src_variant(&upper_ops, 0).map_err(BuildError::Fit)?;
        let mut r = Region::new(BitSetId::Src, uv);
        fill_srcs(&mut r, &upper_ops, 0).map_err(BuildError::Fit)?;
        r
    } else {
        let bs = bit_set(BitSetId::Src);
        let uv = bs
            .variants
            .iter()
            .enumerate()
            .filter(|(_, v)| v.upper_src == Some(true))
            .min_by_key(|(_, v)| v.bytes)
            .map(|(i, _)| i as u16)
            .ok_or(BuildError::Unsupported("no upper source variant"))?;
        Region::new(BitSetId::Src, uv)
    };
    let alutype: u32 = if t.bitwise { 2 } else { 0 };
    /* A bitwise group has no ISS section: its internal registers are wired
     * by the operations themselves. */
    let iss = if super::pack::iss_present(alutype, true) {
        let mut iss = Region::new(BitSetId::Iss, 0);
        for (f, v) in t.iss {
            let val = value(BitSetId::Iss, f, v)?;
            iss.set(f, val).map_err(|_| BuildError::Unsupported("an ISS field that does not take its value"))?;
        }
        Some(iss)
    } else if t.iss.is_empty() {
        None
    } else {
        return Err(BuildError::Unsupported("ISS routing on a group type that has no ISS"));
    };

    let d = match dst {
        Some(dst) => {
            /* One destination fills the section's first field whichever
             * write it is; the header's w0p/w1p says which (pco's `fred`
             * part A: `w1p`, a one-register section). */
            let dst_ops = [Some(dst), None];
            let dv = choose_dst_variant(&dst_ops).map_err(BuildError::Fit)?;
            let mut d = Region::new(BitSetId::Dst, dv);
            fill_dsts(&mut d, &dst_ops).map_err(BuildError::Fit)?;
            Some(d)
        }
        None => None,
    };

    let hv = super::pack::header_variant_of(alutype).ok_or(BuildError::Unsupported("no header for this ALU type"))?;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
    let n = t.steps.len();
    let mut set = phases;
    set[..n].sort_unstable();
    let (field, org) = if t.bitwise {
        ("opcnt", super::pack::opcnt_for(&set[..n]))
    } else {
        ("oporg", super::pack::oporg_for(&set[..n]))
    };
    let org = org.ok_or(BuildError::Phases(super::pack::NotPackable::NotAKnownSet))?;
    hdr.set(field, org).map_err(|_| BuildError::Unsupported("the phase set does not fit the header"))?;
    hdr.set("w0p", u32::from(t.dst && !t.dst_w1)).map_err(|_| BuildError::Unsupported("no w0p in the header"))?;
    if t.dst_w1 {
        hdr.set("w1p", 1).map_err(|_| BuildError::Unsupported("no w1p in the header"))?;
    }
    for (f, v) in t.header {
        let val = value(BitSetId::IgrpHdr, f, v)?;
        hdr.set(f, val).map_err(|_| BuildError::Unsupported("a header field that does not take its value"))?;
    }

    let parts = Parts { alutype, instrs, lower: Some(lower), upper: Some(upper), iss, dst: d, align_pad: 0 };
    close(&parts, hdr).map_err(BuildError::Group)
}
