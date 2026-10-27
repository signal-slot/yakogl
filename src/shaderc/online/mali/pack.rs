// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2021-2022 Collabora Ltd.
// Derived from Mesa 26.2.1 src/panfrost/compiler/bifrost/valhall/va_pack.c
// (MIT). The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Packing: the allocated, flow-annotated BIR into 64-bit words, through the
//! ISA table's encoder ([`super::isa::encode`]), which decodes every word it
//! writes again and refuses what would read back as anything else.
//!
//! Each op's operands are set by name, every one of them: an operand the
//! packer does not name is refused ([`super::Defect::OperandUnset`]), so no
//! default of the generated table (named unmeasured there) reaches a word.
//! The values are `va_pack_instr`'s (va_pack.c:850-1087) for these ops.

use super::bir::{flow, Idx, Ins, LodMode, Op, Prog, Special, Src, Swz, TexDim};
use super::isa::{self, Instr, Op as IsaOp};
use super::{Defect, MaliError};

/// `va_pack_src` (va_pack.c:127-143) and `va_pack_fau_64` (:112-125): a
/// register with its last-use bit, a uniform (`0x80 | word`), a table entry
/// (`0xC0 | i`) or a special value (`0xC0 | (32 + 2 * entry + half)`,
/// valhall/ISA.xml:87-111).
fn src_byte(s: &Src) -> Result<u32, MaliError> {
    Ok(match s.idx {
        Idx::Reg(r) if r < 64 => r as u32 | (s.discard as u32) << 6,
        Idx::Uniform(w) if w < 64 => 0x80 | w as u32,
        Idx::Lut(i) if i < 32 => 0xc0 | i as u32,
        Idx::Special(Special::AtestDatum, hi) => 0xc0 | (32 + 2 * 5 + hi as u32),
        Idx::Special(Special::BlendDescriptor0, hi) => 0xc0 | (32 + 2 * 8 + hi as u32),
        _ => return Err(MaliError::Defect(Defect::Operand)),
    })
}

fn reg(i: Idx) -> Result<u32, MaliError> {
    match i {
        Idx::Reg(r) if r < 64 => Ok(r as u32),
        _ => Err(MaliError::Defect(Defect::Operand)),
    }
}

/// `va_pack_wrmask` (va_pack.c:145-158).
fn wrmask(s: Swz) -> Result<u32, MaliError> {
    match s {
        Swz::H00 => Ok(1),
        Swz::H11 => Ok(2),
        Swz::H01 => Ok(3),
        Swz::H10 | Swz::B0 | Swz::B1 | Swz::B2 | Swz::B3 => Err(MaliError::Defect(Defect::Operand)),
    }
}

/// `va_pack_widen_f32` (va_pack.c:214-227): none, h0, h1 (`widen_m`).
fn widen(s: Swz) -> Result<u32, MaliError> {
    match s {
        Swz::H01 => Ok(0),
        Swz::H00 => Ok(1),
        Swz::H11 => Ok(2),
        Swz::H10 | Swz::B0 | Swz::B1 | Swz::B2 | Swz::B3 => Err(MaliError::Defect(Defect::Operand)),
    }
}

/// `va_pack_widen` of a 32-bit integer source (va_pack.c:310-327,
/// valhall/ISA.xml `widen_int_m`: none 0, h0 2, h1 3, b0..b3 4..7).
fn widen_int(s: Swz) -> Result<u32, MaliError> {
    match s {
        Swz::H01 => Ok(0),
        Swz::H00 => Ok(2),
        Swz::H11 => Ok(3),
        Swz::B0 => Ok(4),
        Swz::B1 => Ok(5),
        Swz::B2 => Ok(6),
        Swz::B3 => Ok(7),
        Swz::H10 => Err(MaliError::Defect(Defect::Operand)),
    }
}

/// `va_pack_swizzle` of a 16-bit source (valhall/ISA.xml `swizzles_16_bit`:
/// h00 0, h10 1, h01 2, h11 3).
fn swz16(s: Swz) -> Result<u32, MaliError> {
    match s {
        Swz::H00 => Ok(0),
        Swz::H10 => Ok(1),
        Swz::H01 => Ok(2),
        Swz::H11 => Ok(3),
        Swz::B0 | Swz::B1 | Swz::B2 | Swz::B3 => Err(MaliError::Defect(Defect::Operand)),
    }
}

/// A byte lane of a 32-bit source (`lanes` of an 8-bit shift source,
/// valhall/ISA.xml `shift_bitwise_i32_lanes_m`: b0..b3 4..7, as `widen_int`).
fn byte_lane(s: Swz) -> Result<u32, MaliError> {
    match s {
        Swz::B0 => Ok(4),
        Swz::B1 => Ok(5),
        Swz::B2 => Ok(6),
        Swz::B3 => Ok(7),
        _ => Err(MaliError::Defect(Defect::Operand)),
    }
}

/// A byte lane select (`MKVEC.v2i8`'s `lane0`/`lane1`, valhall/ISA.xml
/// `lane_8_bit`): b0..b3 0..3.
fn byte(s: Swz) -> Result<u32, MaliError> {
    match s {
        Swz::B0 => Ok(0),
        Swz::B1 => Ok(1),
        Swz::B2 => Ok(2),
        Swz::B3 => Ok(3),
        _ => Err(MaliError::Defect(Defect::Operand)),
    }
}

/// A 16-bit lane select (`MKVEC`'s `lane0`/`lane1`): h0 or h1.
fn lane(s: Swz) -> Result<u32, MaliError> {
    match s {
        Swz::H00 => Ok(0),
        Swz::H11 => Ok(1),
        Swz::H01 | Swz::H10 | Swz::B0 | Swz::B1 | Swz::B2 | Swz::B3 => Err(MaliError::Defect(Defect::Operand)),
    }
}

/// `tex_lod_mode_m` computed_lod (BI_VA_LOD_MODE_COMPUTED_LOD).
const TEX_LOD_COMPUTED: u32 = 1;
/// `tex_lod_mode_m` none (BI_VA_LOD_MODE_ZERO_LOD: level 0).
const TEX_LOD_ZERO: u32 = 0;
/// `tex_lod_mode_m` explicit (BI_VA_LOD_MODE_EXPLICIT: the level in a
/// staging register; valhall/ISA.xml "LOD mode" value 4).
const TEX_LOD_EXPLICIT: u32 = 4;
/// `tex_dimensionality_m` tex2d.
const TEX_DIM_2D: u32 = 1;
/// `tex_dimensionality_m` cube (`BI_DIMENSION_CUBE`; the ISA's `Dimension`
/// enum is 1d, 2d, 3d, cube in order, valhall/ISA.xml:477-482).
const TEX_DIM_CUBE: u32 = 3;
/// `mux_cmpf_m` bit (`BI_MUX_BIT`).
const MUX_BIT: u32 = 3;

/// An instruction of the table being filled, every operand by name.
struct Fill {
    i: Instr,
    set: u32,
}

impl Fill {
    fn new(op: IsaOp) -> Fill {
        Fill { i: Instr { op, vals: [0; isa::MAX_OPERANDS], ignored: 0 }, set: 0 }
    }

    fn put(&mut self, name: &str, v: u32) -> Result<&mut Fill, MaliError> {
        let k = self.i.op.info().operands.iter().position(|o| o.name == name);
        let k = k.ok_or(MaliError::Defect(Defect::OperandName))?;
        self.i.vals[k] = v;
        self.set |= 1 << k;
        Ok(self)
    }

    fn done(self) -> Result<u64, MaliError> {
        let n = self.i.op.info().operands.len();
        if self.set != (1u32 << n) - 1 {
            return Err(MaliError::Defect(Defect::OperandUnset));
        }
        isa::encode(&self.i).map_err(|_| MaliError::Defect(Defect::Encode))
    }
}

/// The common tail: the FAU page (page 0, the only one this back end
/// writes) and the flow.
fn tail(f: &mut Fill, i: &Ins) -> Result<(), MaliError> {
    f.put("fau_page_index", 0)?.put("flow", i.flow as u32)?;
    Ok(())
}

/// The destination of a one-register ALU op and its write mask.
fn alu_dest(f: &mut Fill, i: &Ins, width: &str) -> Result<(), MaliError> {
    f.put("dest_register_index", reg(i.dest[0].idx)?)?.put(width, wrmask(i.dest[0].swz)?)?;
    Ok(())
}

/// A float source: the byte, its widen, its negate and its abs.
fn fsrc(f: &mut Fill, i: &Ins, s: usize) -> Result<(), MaliError> {
    let x = &i.src[s];
    f.put(["src0", "src1", "src2"][s], src_byte(x)?)?;
    f.put(["widen0", "widen1", "widen2"][s], widen(x.swz)?)?;
    f.put(["neg0", "neg1", "neg2"][s], x.neg as u32)?;
    f.put(["abs0", "abs1", "abs2"][s], x.abs as u32)?;
    Ok(())
}

/// One instruction (`va_pack_instr`); `branch` is a relative branch's
/// offset in instructions from the next one ([`branch_offset`]).
pub fn pack_one(i: &Ins, branch: i32) -> Result<u64, MaliError> {
    let mut f;
    match i.op {
        Op::Nop => {
            f = Fill::new(IsaOp::Nop);
            f.put("flow", i.flow as u32)?;
            return f.done();
        }
        Op::MovI32 => {
            f = Fill::new(IsaOp::MovI32);
            f.put("src0", src_byte(&i.src[0])?)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::IaddImmI32(imm) => {
            f = Fill::new(IsaOp::IaddImmI32);
            f.put("src0", src_byte(&i.src[0])?)?.put("imm1w", imm)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FmaF32 => {
            f = Fill::new(IsaOp::FmaF32);
            for s in 0..3 {
                fsrc(&mut f, i, s)?;
            }
            f.put("round", 0)?.put("clamp", i.clamp as u32)?;
            alu_dest(&mut f, i, "dest_width_narrow")?;
        }
        Op::FcmpOrF32 { cmpf, result } => {
            f = Fill::new(IsaOp::FcmpOrF32);
            for s in 0..2 {
                fsrc(&mut f, i, s)?;
            }
            f.put("src2", src_byte(&i.src[2])?)?;
            f.put("cmpf", cmpf as u32)?.put("result_type", result as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FcmpOrV2f16 { cmpf, result } => {
            f = Fill::new(IsaOp::FcmpOrV2f16);
            for s in 0..2 {
                let x = &i.src[s];
                f.put(["src0", "src1"][s], src_byte(x)?)?;
                f.put(["swz0", "swz1"][s], swz16(x.swz)?)?;
                f.put(["neg0", "neg1"][s], x.neg as u32)?;
                f.put(["abs0", "abs1"][s], x.abs as u32)?;
            }
            f.put("src2", src_byte(&i.src[2])?)?;
            f.put("cmpf", cmpf as u32)?.put("result_type", result as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::IcmpOrS32 { cmpf, result } | Op::IcmpOrU32 { cmpf, result } => {
            f = Fill::new(if matches!(i.op, Op::IcmpOrS32 { .. }) { IsaOp::IcmpOrS32 } else { IsaOp::IcmpOrU32 });
            for s in 0..2 {
                f.put(["src0", "src1"][s], src_byte(&i.src[s])?)?
                    .put(["widen0", "widen1"][s], widen_int(i.src[s].swz)?)?;
            }
            f.put("src2", src_byte(&i.src[2])?)?;
            f.put("cmpf", cmpf as u32)?.put("result_type", result as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::MuxI32 => {
            f = Fill::new(IsaOp::MuxI32);
            for s in 0..3 {
                f.put(["src0", "src1", "src2"][s], src_byte(&i.src[s])?)?;
            }
            /* `.int_zero` (BI_MUX_INT_ZERO, the Mux enum's default) */
            f.put("muxf", 1)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::MuxBitI32 => {
            f = Fill::new(IsaOp::MuxI32);
            for s in 0..3 {
                f.put(["src0", "src1", "src2"][s], src_byte(&i.src[s])?)?;
            }
            f.put("muxf", MUX_BIT)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        /* the cube transform (va_pack.c packs them as any three-source
         * op: the coordinates float sources with widen, negate and abs) */
        Op::Cubeface1 | Op::Cubeface2 => {
            f = Fill::new(if i.op == Op::Cubeface1 { IsaOp::Cubeface1 } else { IsaOp::Cubeface2 });
            for s in 0..3 {
                fsrc(&mut f, i, s)?;
            }
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::CubeSsel | Op::CubeTsel => {
            f = Fill::new(if i.op == Op::CubeSsel { IsaOp::CubeSsel } else { IsaOp::CubeTsel });
            for s in 0..2 {
                fsrc(&mut f, i, s)?;
            }
            /* the face index: a plain word (no modifier in the ISA) */
            f.put("src2", src_byte(&i.src[2])?)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::MuxV2i16 => {
            f = Fill::new(IsaOp::MuxV2i16);
            for s in 0..3 {
                f.put(["src0", "src1", "src2"][s], src_byte(&i.src[s])?)?;
                f.put(["swz0", "swz1", "swz2"][s], swz16(i.src[s].swz)?)?;
            }
            f.put("muxf", 1)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::CselU32 => {
            f = Fill::new(IsaOp::CselU32);
            for s in 0..4 {
                f.put(["src0", "src1", "src2", "src3"][s], src_byte(&i.src[s])?)?;
            }
            f.put("cmpf", super::bir::Cmpf::Eq as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::CselS32 { cmpf } => {
            f = Fill::new(IsaOp::CselS32);
            for s in 0..4 {
                f.put(["src0", "src1", "src2", "src3"][s], src_byte(&i.src[s])?)?;
            }
            f.put("cmpf", cmpf as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::IabsS32 => {
            f = Fill::new(IsaOp::IabsS32);
            f.put("src0", src_byte(&i.src[0])?)?.put("widen0", widen_int(i.src[0].swz)?)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        /* `IMUL.i32` is the table's `IMUL.u32` encoding (datatype 0; Mesa's
         * ISA.xml names the same word `IMUL.i32`) */
        Op::ImulI32 => {
            f = Fill::new(IsaOp::ImulU32);
            for s in 0..2 {
                f.put(["src0", "src1"][s], src_byte(&i.src[s])?)?
                    .put(["widen0", "widen1"][s], widen_int(i.src[s].swz)?)?;
            }
            f.put("saturate", 0)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FroundF32 { round } => {
            f = Fill::new(IsaOp::FroundF32);
            fsrc(&mut f, i, 0)?;
            f.put("round", round as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FroundV2f16 { round } => {
            f = Fill::new(IsaOp::FroundV2f16);
            let x = &i.src[0];
            f.put("src0", src_byte(x)?)?.put("swz0", swz16(x.swz)?)?;
            f.put("neg0", x.neg as u32)?.put("abs0", x.abs as u32)?;
            f.put("round", round as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FrsqF32 => {
            f = Fill::new(IsaOp::FrsqF32);
            fsrc(&mut f, i, 0)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::F32ToS32 => {
            f = Fill::new(IsaOp::F32ToS32);
            f.put("src0", src_byte(&i.src[0])?)?;
            f.put("neg0", i.src[0].neg as u32)?.put("abs0", i.src[0].abs as u32)?;
            f.put("round", super::bir::Round::Rtz as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::S32ToF32 => {
            f = Fill::new(IsaOp::S32ToF32);
            f.put("src0", src_byte(&i.src[0])?)?.put("round", 0)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::IaddS32 => {
            f = Fill::new(IsaOp::IaddS32);
            for s in 0..2 {
                f.put(["src0", "src1"][s], src_byte(&i.src[s])?)?
                    .put(["widen0", "widen1"][s], widen_int(i.src[s].swz)?)?;
            }
            f.put("saturate", 0)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::S16ToS32 => {
            f = Fill::new(IsaOp::S16ToS32);
            f.put("src0", src_byte(&i.src[0])?)?.put("lane0", lane(i.src[0].swz)?)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::U16ToU32 => {
            f = Fill::new(IsaOp::U16ToU32);
            f.put("src0", src_byte(&i.src[0])?)?.put("lane0", lane(i.src[0].swz)?)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::U16ToF32 => {
            f = Fill::new(IsaOp::U16ToF32);
            f.put("src0", src_byte(&i.src[0])?)?.put("lane0", lane(i.src[0].swz)?)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::F16ToF32 => {
            f = Fill::new(IsaOp::F16ToF32);
            f.put("src0", src_byte(&i.src[0])?)?.put("lane0", lane(i.src[0].swz)?)?;
            f.put("neg0", i.src[0].neg as u32)?.put("abs0", i.src[0].abs as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FaddImmF32(imm) => {
            f = Fill::new(IsaOp::FaddImmF32);
            f.put("src0", src_byte(&i.src[0])?)?.put("imm1w", imm)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FaddImmV2f16(imm) => {
            f = Fill::new(IsaOp::FaddImmV2f16);
            f.put("src0", src_byte(&i.src[0])?)?.put("imm1w", imm)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FmaV2f16 => {
            f = Fill::new(IsaOp::FmaV2f16);
            for s in 0..3 {
                let x = &i.src[s];
                f.put(["src0", "src1", "src2"][s], src_byte(x)?)?;
                f.put(["swz0", "swz1", "swz2"][s], swz16(x.swz)?)?;
                f.put(["neg0", "neg1", "neg2"][s], x.neg as u32)?;
                f.put(["abs0", "abs1", "abs2"][s], x.abs as u32)?;
            }
            f.put("round", 0)?.put("clamp", i.clamp as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FaddV2f16 => {
            f = Fill::new(IsaOp::FaddV2f16);
            for s in 0..2 {
                let x = &i.src[s];
                f.put(["src0", "src1"][s], src_byte(x)?)?;
                f.put(["swz0", "swz1"][s], swz16(x.swz)?)?;
                f.put(["neg0", "neg1"][s], x.neg as u32)?;
                f.put(["abs0", "abs1"][s], x.abs as u32)?;
            }
            f.put("round", 0)?.put("clamp", i.clamp as u32)?.put("sticky", 0)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FaddF32 => {
            f = Fill::new(IsaOp::FaddF32);
            for s in 0..2 {
                fsrc(&mut f, i, s)?;
            }
            f.put("round", 0)?.put("clamp", i.clamp as u32)?.put("sticky", 0)?;
            alu_dest(&mut f, i, "dest_width_narrow")?;
        }
        Op::FrcpF32 => {
            f = Fill::new(IsaOp::FrcpF32);
            fsrc(&mut f, i, 0)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FmaxF32 | Op::FminF32 => {
            f = Fill::new(if i.op == Op::FmaxF32 { IsaOp::FmaxF32 } else { IsaOp::FminF32 });
            for s in 0..2 {
                fsrc(&mut f, i, s)?;
            }
            /* sem 0: the .nan_suppress the BIR carries */
            f.put("sem", 0)?.put("clamp", i.clamp as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::FmaxV2f16 | Op::FminV2f16 => {
            f = Fill::new(if i.op == Op::FmaxV2f16 { IsaOp::FmaxV2f16 } else { IsaOp::FminV2f16 });
            for s in 0..2 {
                let x = &i.src[s];
                f.put(["src0", "src1"][s], src_byte(x)?)?;
                f.put(["swz0", "swz1"][s], swz16(x.swz)?)?;
                f.put(["neg0", "neg1"][s], x.neg as u32)?;
                f.put(["abs0", "abs1"][s], x.abs as u32)?;
            }
            /* sem 0: the .nan_suppress the BIR carries */
            f.put("sem", 0)?.put("clamp", i.clamp as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::V2f32ToV2f16 => {
            f = Fill::new(IsaOp::V2f32ToV2f16);
            for s in 0..2 {
                let x = &i.src[s];
                f.put(["src0", "src1"][s], src_byte(x)?)?;
                f.put(["neg0", "neg1"][s], x.neg as u32)?.put(["abs0", "abs1"][s], x.abs as u32)?;
            }
            /* the clamp a folded `fsat` left (a blend shader's constants) */
            f.put("round", 0)?.put("clamp", i.clamp as u32)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::MkvecV2i16 => {
            f = Fill::new(IsaOp::MkvecV2i16);
            for s in 0..2 {
                f.put(["src0", "src1"][s], src_byte(&i.src[s])?)?.put(["lane0", "lane1"][s], lane(i.src[s].swz)?)?;
            }
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::MkvecV2i8 => {
            f = Fill::new(IsaOp::MkvecV2i8);
            for s in 0..2 {
                f.put(["src0", "src1"][s], src_byte(&i.src[s])?)?.put(["lane0", "lane1"][s], byte(i.src[s].swz)?)?;
            }
            f.put("src2", src_byte(&i.src[2])?)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::ShaddxS64 => {
            f = Fill::new(IsaOp::ShaddS64);
            f.put("src0", src_byte(&i.src[0])?)?.put("src1", src_byte(&i.src[1])?)?;
            f.put("shift", 0)?.put("widen1", 0)?.put("dest_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::LdAttrImm { index, vecsize } => {
            f = Fill::new(IsaOp::LdAttrImm);
            f.put("attribute_table_index", 1)?.put("attribute_index", index as u32)?;
            /* register format f32 (va_pack_register_format), vecsize n - 1 */
            f.put("register_format", 2)?.put("vecsize", vecsize as u32 - 1)?;
            f.put("src0", src_byte(&i.src[0])?)?.put("src1", src_byte(&i.src[1])?)?;
            f.put("message_slot_index", i.slot as u32)?.put("staging_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::LeaBufImm { table, index } => {
            f = Fill::new(IsaOp::LeaBufImm);
            f.put("buffer_table_index", table as u32)?.put("buffer_index", index as u32)?;
            f.put("src0", src_byte(&i.src[0])?)?;
            f.put("message_slot_index", i.slot as u32)?.put("staging_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::StoreI32 { access } | Op::StoreI16 { access } => {
            f = Fill::new(if matches!(i.op, Op::StoreI16 { .. }) { IsaOp::StoreI16 } else { IsaOp::StoreI32 });
            /* the address pair, lo word; the data is the staging register */
            f.put("src0", src_byte(&i.src[1])?)?;
            f.put("access", if access == super::bir::Access::Istream { 1 } else { 2 })?;
            f.put("message_slot_index", i.slot as u32)?.put("staging_register_index", reg(i.src[0].idx)?)?;
            f.put("offset", 0)?;
            if matches!(i.op, Op::StoreI16 { .. }) {
                /* va_pack_store leaves the 16-bit lane 0 (the low half) */
                f.put("lane", 0)?;
            }
        }
        Op::LdVarBufImmF32 { index, vecsize } => {
            f = Fill::new(IsaOp::LdVarBufImmF32);
            /* centre, .store, source format f32 */
            f.put("sample_position", 0)?.put("update", 0)?.put("source_format", 2)?;
            f.put("offset", index as u32)?.put("vecsize", vecsize as u32 - 1)?;
            f.put("src0", src_byte(&i.src[0])?)?;
            f.put("message_slot_index", i.slot as u32)?.put("staging_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::LdVarSpecialF32 { name, vecsize } => {
            f = Fill::new(IsaOp::LdVarSpecial);
            /* the name at the centre; the update is the one the varying
             * name implies (valhall ISA.xml: `.none` for `frag_z`, else
             * `.clobber`; the word's encoded update is the same) */
            let update = if name == super::bir::VaryingName::FragZ { 4 } else { 3 };
            f.put("varying_name", name as u32)?.put("update", update)?.put("sample_position", 0)?;
            f.put("vecsize", vecsize as u32 - 1)?.put("src0", src_byte(&i.src[0])?)?;
            f.put("message_slot_index", i.slot as u32)?.put("staging_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::LdVarImmF32 { table, index, vecsize } => {
            f = Fill::new(IsaOp::LdVarImm);
            /* centre, .store, register format f32 */
            f.put("sample_position", 0)?.put("update", 0)?.put("register_format", 2)?;
            f.put("varying_table_index", table as u32)?.put("varying_index", index as u32)?;
            f.put("vecsize", vecsize as u32 - 1)?.put("src0", src_byte(&i.src[0])?)?;
            f.put("message_slot_index", i.slot as u32)?.put("staging_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::Atest => {
            f = Fill::new(IsaOp::Atest);
            f.put("src0", src_byte(&i.src[0])?)?.put("src1", src_byte(&i.src[1])?)?;
            f.put("widen1", widen(i.src[1].swz)?)?.put("src2", src_byte(&i.src[2])?)?;
            f.put("staging_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::ZsEmit { z, s } => {
            f = Fill::new(IsaOp::ZsEmit);
            /* depth, stencil, coverage; the modifiers (va_pack.c:590-595) */
            f.put("src0", src_byte(&i.src[0])?)?.put("src1", src_byte(&i.src[1])?)?;
            f.put("src2", src_byte(&i.src[2])?)?;
            f.put("z", z as u32)?.put("stencil", s as u32)?;
            f.put("staging_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::Blend { f32, u32, .. } => {
            f = Fill::new(IsaOp::Blend);
            /* the descriptor's low word; the coverage as a bare register,
             * without its last-use bit (`va_pack_reg`, va_pack.c:1011-1012);
             * register format f16 (3), f32 (2) or u32 (6) */
            f.put("src0", src_byte(&i.src[2])?)?.put("src2", reg(i.src[1].idx)?)?;
            let fmt = match (f32, u32) {
                (_, true) => 6,
                (true, false) => 2,
                (false, false) => 3,
            };
            f.put("register_format", fmt)?.put("message_slot_index", i.slot as u32)?;
            f.put("staging_register_index", reg(i.src[0].idx)?)?.put("offset", 0)?;
        }
        Op::BranchzI { eq } => {
            f = Fill::new(IsaOp::Jump);
            /* .eq (not) or .ne, absolute, no lane combine (va_pack.c:493-505) */
            f.put("src0", src_byte(&i.src[0])?)?.put("src1", src_byte(&i.src[1])?)?;
            f.put("not", eq as u32)?.put("combine", 0)?.put("address_mode", 1)?.put("conservative", 0)?;
        }
        Op::LshiftAndI32 | Op::LshiftOrI32 | Op::RshiftAndI32 | Op::RshiftOrI32 => {
            f = Fill::new(match i.op {
                Op::LshiftAndI32 => IsaOp::LshiftAndI32,
                Op::LshiftOrI32 => IsaOp::LshiftOrI32,
                Op::RshiftOrI32 => IsaOp::RshiftOrI32,
                _ => IsaOp::RshiftAndI32,
            });
            /* BIR (value, mask or or-value, shift) as Valhall (src0, src2,
             * src1): `va_swap_12`; the shift a byte lane of its source */
            f.put("src0", src_byte(&i.src[0])?)?.put("widen0", widen_int(i.src[0].swz)?)?;
            f.put("src2", src_byte(&i.src[1])?)?.put("not2", 0)?;
            f.put("src1", src_byte(&i.src[2])?)?.put("lanes1", byte_lane(i.src[2].swz)?)?;
            f.put("not_result", 0)?;
            alu_dest(&mut f, i, "dest_width")?;
        }
        Op::LdPka { words, half } => {
            /* `va_pack_load` (va_pack.c:746-769): the address at bit 0, the
             * mode descriptor at bit 8, the memory access `.none` (0); the
             * width's opcode carries the register count, lane and extend */
            f = Fill::new(match (words, half) {
                (1, true) => IsaOp::LdPkaI16,
                (1, false) => IsaOp::LdPkaI32,
                (2, false) => IsaOp::LdPkaI64,
                (3, false) => IsaOp::LdPkaI96,
                (4, false) => IsaOp::LdPkaI128,
                _ => return Err(MaliError::Defect(Defect::Operand)),
            });
            if words == 1 {
                /* the low half (`va_pack_load`'s `lane` of a 16-bit load is
                 * the destination's half, h0) or the whole word of a 32-bit
                 * one, no extension (`none`, which the opcode encodes as its
                 * compressed 1; case108_uniidx's `LD_PKA.i16.unsigned`,
                 * uniarray_highp's `LD_PKA.i32.unsigned`) */
                f.put("lane", 0)?.put("extend", 2)?;
            }
            f.put("src0", src_byte(&i.src[0])?)?.put("src1", src_byte(&i.src[1])?)?.put("access", 0)?;
            f.put("message_slot_index", i.slot as u32)?.put("staging_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::LdTile { vecsize, f16 } => {
            f = Fill::new(IsaOp::LdTile);
            /* the pixel indices, the coverage, the conversion; register
             * format f16 (3) or f32 (2), colour (not z_stencil) */
            f.put("src0", src_byte(&i.src[0])?)?.put("src1", src_byte(&i.src[1])?)?;
            f.put("src2", src_byte(&i.src[2])?)?;
            f.put("register_format", if f16 { 3 } else { 2 })?.put("vecsize", vecsize as u32 - 1)?;
            f.put("z_stencil", 0)?.put("message_slot_index", i.slot as u32)?;
            f.put("staging_register_index", reg(i.dest[0].idx)?)?;
        }
        Op::Branchz { eq, .. } => {
            f = Fill::new(IsaOp::Branch);
            /* the source's half as the lane combine (`<src combine>`: the
             * widen of its swizzle, va_pack.c:680-684: none, h0, h1), `.eq`
             * as `not`, the offset in bytes (the field holds it in
             * instructions, 27 bits signed) */
            f.put("src0", src_byte(&i.src[0])?)?.put("combine", widen(i.src[0].swz)?)?;
            f.put("not", eq as u32)?.put("conservative", 0)?;
            f.put("offset", branch.checked_mul(8).ok_or(MaliError::Defect(Defect::Operand))? as u32)?;
        }
        Op::Phi | Op::DiscardB32 => return Err(MaliError::Defect(Defect::PseudoOp)),
        /* two float sources with their modifiers and widens, the condition;
         * no destination (va_pack.c packs it as any ALU op) */
        Op::DiscardF32 { cmpf } => {
            f = Fill::new(IsaOp::DiscardF32);
            for s in 0..2 {
                fsrc(&mut f, i, s)?;
            }
            f.put("cmpf", cmpf as u32)?;
        }
        /* `TEX_FETCH`: the fields of TEX_SINGLE without a level mode, a
         * compare or a skip bit (va_pack.c:1052-1077) */
        Op::TexSingle { mask, f16, lod: LodMode::Fetch, dim, .. } => {
            f = Fill::new(IsaOp::TexFetch);
            let dim = match dim {
                TexDim::D2 => TEX_DIM_2D,
                TexDim::Cube => TEX_DIM_CUBE,
            };
            f.put("src0", src_byte(&i.src[1])?)?;
            f.put("array_enable", 0)?.put("texel_offset", 0)?.put("wide_indices", 0)?;
            f.put("dimensionality", dim)?.put("write_mask", mask as u32)?;
            f.put("register_width", if f16 { 0 } else { 1 })?.put("skip", 0)?;
            f.put("message_slot_index", i.slot as u32)?;
            f.put("staging_register_index", reg(i.src[0].idx)?)?;
            f.put("staging_register_write_index", reg(i.dest[0].idx)?)?;
        }
        Op::TexSingle { mask, f16, skip, lod, dim, .. } => {
            f = Fill::new(IsaOp::TexSingle);
            /* the handle pair (`va_pack_src` of source 1, its register
             * pair after va_lower_split_64bit), the look-up of every
             * channel at the LOD mode and the texture's dimension, no
             * array, offset, compare or wide indices (va_pack.c:1052-1077
             * with pan_nir_lower_tex.c's flags) */
            let lod = match lod {
                LodMode::Computed => TEX_LOD_COMPUTED,
                LodMode::Zero => TEX_LOD_ZERO,
                LodMode::Explicit => TEX_LOD_EXPLICIT,
                LodMode::Fetch => return Err(MaliError::Defect(Defect::Operand)),
            };
            let dim = match dim {
                TexDim::D2 => TEX_DIM_2D,
                TexDim::Cube => TEX_DIM_CUBE,
            };
            f.put("src0", src_byte(&i.src[1])?)?;
            f.put("array_enable", 0)?.put("texel_offset", 0)?.put("compare_enable", 0)?;
            f.put("wide_indices", 0)?.put("lod_mode", lod)?;
            f.put("dimensionality", dim)?.put("write_mask", mask as u32)?;
            f.put("register_width", if f16 { 0 } else { 1 })?.put("skip", skip as u32)?;
            f.put("message_slot_index", i.slot as u32)?;
            f.put("staging_register_index", reg(i.src[0].idx)?)?;
            f.put("staging_register_write_index", reg(i.dest[0].idx)?)?;
        }
        Op::Collect | Op::Split => return Err(MaliError::Defect(Defect::Vector)),
        Op::FabsnegF32 | Op::FabsnegV2f16 | Op::FclampF32 | Op::FclampV2f16 => {
            return Err(MaliError::Defect(Defect::PseudoOp))
        }
    }
    tail(&mut f, i)?;
    f.done()
}

/// `va_lower_blend` (va_pack.c:1172-1202): after the BLEND that ends the
/// shader, `r48 = 0` (the blend shader's link register: return to nothing)
/// and the branch to the blend shader the descriptor's high word names,
/// taken when it is not zero.
pub fn lower_blend(p: &mut Prog<'_>) -> Result<(), MaliError> {
    let mut k = 0;
    while k < p.n {
        let i = p.ins[k];
        if let Op::Blend { .. } = i.op {
            if i.flow != flow::END {
                return Err(MaliError::Defect(Defect::BlendNotLast));
            }
            let link = Ins::new(Op::IaddImmI32(0), &[i.dest[0]], &[Src::of(Idx::Lut(0))]);
            let call = Ins::new(Op::BranchzI { eq: true }, &[], &[Src::of(Idx::Lut(0)), i.src[3]]);
            p.insert(k + 1, link)?;
            p.insert(k + 2, call)?;
            k += 2;
        }
        k += 1;
    }
    Ok(())
}

/// `va_lower_branch_target` (va_pack.c:1104-1158) for the branch at `k`:
/// the instructions from the one after it to the start of its target
/// block (an empty target starts where the next block does), as the
/// offset is relative to the next instruction; 0 for any other op.
fn branch_offset(p: &Prog<'_>, k: usize) -> i32 {
    match p.ins[k].op {
        Op::Branchz { target, .. } => p.block_start(target) as i32 - k as i32 - 1,
        _ => 0,
    }
}

/// The bytes of a program: its words, then zeros up to the next multiple of
/// 128 bytes past 16 more (`bi_pack_valhall`, va_pack.c:1225-1239); an
/// empty program stays empty. Returns the words written, padding included.
pub fn pack(p: &Prog<'_>, out: &mut [u64]) -> Result<usize, MaliError> {
    if p.n == 0 {
        return Ok(0);
    }
    let padded = (p.n * 8 + 16).div_ceil(128) * 128 / 8;
    if padded > out.len() {
        return Err(MaliError::Defect(Defect::CodeBuffer));
    }
    for (k, i) in p.as_slice().iter().enumerate() {
        out[k] = pack_one(i, branch_offset(p, k))?;
    }
    /* the last word is the `.end` instruction or, in a fragment shader, the
     * blend-shader call after it: neither packs to zero, so a program's end
     * is its last non-zero word (the padding is zeroes, and a program of
     * zeroes is an encoding fault, va_pack.c:1225-1239) */
    if out[p.n - 1] == 0 {
        return Err(MaliError::Defect(Defect::ZeroLastWord));
    }
    for w in out[p.n..padded].iter_mut() {
        *w = 0;
    }
    Ok(padded)
}
