// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every entry of Mesa 26.2.1's V3D 7.1 add and mul opcode tables packs and
//! unpacks in `codec71`, and every `AddOp71` / `MulOp71` variant corresponds
//! to one Mesa entry.
//!
//! The source is Mesa 26.2.1 `src/broadcom/qpu/qpu_pack.c`: `v3d71_add_ops`
//! (line 586) and `v3d71_mul_ops` (line 772), transcribed below with the
//! line of each entry (`first`/`last` are `opcode_first`/`opcode_last`, `mask`
//! the `raddr_mask` as an inclusive range of raddr_b (add) or raddr_d (mul)
//! values, `ANY` for `ANYOPMASK`, `ver` the entry's `first_ver`), and
//! `src/broadcom/qpu/qpu_instr.c`: `add_op_args` (line 404) and
//! `mul_op_args` (line 531), which give each operation's destination and
//! operand count (`v3d_qpu_add_op_num_src`, `v3d_qpu_mul_op_num_src`). Both
//! are MIT.
//!
//! Where the 7.1 word keeps the opcode (`V3D_QPU_OP_ADD`, bits 31..24;
//! `V3D_QPU_OP_MUL`, bits 63..58) and the raddr that completes it (raddr_b,
//! bits 5..0, for an add operation with fewer than two operands, raddr_d,
//! bits 17..12, for the mul side's, `qpu_pack.c:1868`, `:2290`), the tests
//! read them from the word `encode71` wrote, and check them against the
//! entries of the operation. Three checks:
//!
//! * per variant: it has entries; its operand count and destination are
//!   Mesa's `*_op_args`; an instruction for it with every input unpack and
//!   output pack encodes where Mesa's pack accepts it, decodes back to it, and
//!   its opcode and raddr fall into an entry of that operation and are
//!   resolved to it by Mesa's first-match lookup (`lookup_opcode_from_packed`,
//!   `qpu_pack.c:833`);
//! * per entry: the entry is reached by one of those encodings, or it is
//!   listed ([`UNREACHED_ADD`], [`UNREACHED_MUL`]);
//! * per entry, over its whole range: a word with each opcode and each raddr
//!   of the entry, in an otherwise valid instruction, decodes (when `codec71`
//!   accepts it) to the operation Mesa's first-match lookup gives.
//!
//! The variants with no entry are listed ([`ADD_WITHOUT_ENTRY`]) and so is
//! the Mesa operation with no variant ([`MESA_WITHOUT_VARIANT`]). Feature
//! sets: `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table`.
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::isa::{num_src, OutPack, ARG_A, ARG_B, ARG_D};
use yakogl::shaderc::online::v3d::isa71::*;

/// One `struct opcode_desc` of Mesa's 7.1 tables.
struct E {
    line: u32,
    first: u32,
    last: u32,
    mask: (u8, u8),
    op: &'static str,
    ver: u32,
}

/// `ANYOPMASK`.
const ANY: (u8, u8) = (0, 63);

/// One `[op] = D | A | B` line of `add_op_args` / `mul_op_args`.
struct Args {
    line: u32,
    op: &'static str,
    dst: bool,
    a: bool,
    b: bool,
}

/// `qpu_pack.c` v3d71_add_ops, lines 586..

const ADD_OPS: &[E] = &[
    E { line: 588, first: 0, last: 47, mask: ANY, op: "FADD", ver: 0 },
    E { line: 589, first: 0, last: 47, mask: ANY, op: "FADDNF", ver: 0 },
    E { line: 590, first: 48, last: 52, mask: ANY, op: "VFADD", ver: 71 },
    E { line: 591, first: 53, last: 55, mask: ANY, op: "VFPACK", ver: 0 },
    E { line: 592, first: 56, last: 56, mask: ANY, op: "ADD", ver: 0 },
    E { line: 593, first: 57, last: 59, mask: ANY, op: "VFPACK", ver: 0 },
    E { line: 594, first: 60, last: 60, mask: ANY, op: "SUB", ver: 0 },
    E { line: 595, first: 61, last: 63, mask: ANY, op: "VFPACK", ver: 0 },
    E { line: 596, first: 64, last: 111, mask: ANY, op: "FSUB", ver: 0 },
    E { line: 597, first: 112, last: 119, mask: ANY, op: "VFSUB", ver: 71 },
    E { line: 598, first: 120, last: 120, mask: ANY, op: "MIN", ver: 0 },
    E { line: 599, first: 121, last: 121, mask: ANY, op: "MAX", ver: 0 },
    E { line: 600, first: 122, last: 122, mask: ANY, op: "UMIN", ver: 0 },
    E { line: 601, first: 123, last: 123, mask: ANY, op: "UMAX", ver: 0 },
    E { line: 602, first: 124, last: 124, mask: ANY, op: "SHL", ver: 0 },
    E { line: 603, first: 125, last: 125, mask: ANY, op: "SHR", ver: 0 },
    E { line: 604, first: 126, last: 126, mask: ANY, op: "ASR", ver: 0 },
    E { line: 605, first: 127, last: 127, mask: ANY, op: "ROR", ver: 0 },
    E { line: 607, first: 128, last: 175, mask: ANY, op: "FMIN", ver: 0 },
    E { line: 608, first: 128, last: 175, mask: ANY, op: "FMAX", ver: 0 },
    E { line: 609, first: 176, last: 180, mask: ANY, op: "VFMIN", ver: 0 },
    E { line: 611, first: 181, last: 181, mask: ANY, op: "AND", ver: 0 },
    E { line: 612, first: 182, last: 182, mask: ANY, op: "OR", ver: 0 },
    E { line: 613, first: 183, last: 183, mask: ANY, op: "XOR", ver: 0 },
    E { line: 614, first: 184, last: 184, mask: ANY, op: "VADD", ver: 0 },
    E { line: 615, first: 185, last: 185, mask: ANY, op: "VSUB", ver: 0 },
    E { line: 617, first: 186, last: 186, mask: (0, 0), op: "NOT", ver: 0 },
    E { line: 618, first: 186, last: 186, mask: (1, 1), op: "NEG", ver: 0 },
    E { line: 619, first: 186, last: 186, mask: (2, 2), op: "FLAPUSH", ver: 0 },
    E { line: 620, first: 186, last: 186, mask: (3, 3), op: "FLBPUSH", ver: 0 },
    E { line: 621, first: 186, last: 186, mask: (4, 4), op: "FLPOP", ver: 0 },
    E { line: 622, first: 186, last: 186, mask: (5, 5), op: "CLZ", ver: 0 },
    E { line: 623, first: 186, last: 186, mask: (6, 6), op: "SETMSF", ver: 0 },
    E { line: 624, first: 186, last: 186, mask: (7, 7), op: "SETREVF", ver: 0 },
    E { line: 626, first: 187, last: 187, mask: (0, 0), op: "NOP", ver: 0 },
    E { line: 627, first: 187, last: 187, mask: (1, 1), op: "TIDX", ver: 0 },
    E { line: 628, first: 187, last: 187, mask: (2, 2), op: "EIDX", ver: 0 },
    E { line: 629, first: 187, last: 187, mask: (3, 3), op: "LR", ver: 0 },
    E { line: 630, first: 187, last: 187, mask: (4, 4), op: "VFLA", ver: 0 },
    E { line: 631, first: 187, last: 187, mask: (5, 5), op: "VFLNA", ver: 0 },
    E { line: 632, first: 187, last: 187, mask: (6, 6), op: "VFLB", ver: 0 },
    E { line: 633, first: 187, last: 187, mask: (7, 7), op: "VFLNB", ver: 0 },
    E { line: 634, first: 187, last: 187, mask: (8, 8), op: "XCD", ver: 0 },
    E { line: 635, first: 187, last: 187, mask: (9, 9), op: "YCD", ver: 0 },
    E { line: 636, first: 187, last: 187, mask: (10, 10), op: "MSF", ver: 0 },
    E { line: 637, first: 187, last: 187, mask: (11, 11), op: "REVF", ver: 0 },
    E { line: 638, first: 187, last: 187, mask: (12, 12), op: "IID", ver: 0 },
    E { line: 639, first: 187, last: 187, mask: (13, 13), op: "SAMPID", ver: 0 },
    E { line: 640, first: 187, last: 187, mask: (14, 14), op: "BARRIERID", ver: 0 },
    E { line: 641, first: 187, last: 187, mask: (15, 15), op: "TMUWT", ver: 0 },
    E { line: 642, first: 187, last: 187, mask: (16, 16), op: "VPMWT", ver: 0 },
    E { line: 643, first: 187, last: 187, mask: (17, 17), op: "FLAFIRST", ver: 0 },
    E { line: 644, first: 187, last: 187, mask: (18, 18), op: "FLNAFIRST", ver: 0 },
    E { line: 646, first: 187, last: 187, mask: (32, 34), op: "FXCD", ver: 0 },
    E { line: 647, first: 187, last: 187, mask: (36, 38), op: "FYCD", ver: 0 },
    E { line: 649, first: 187, last: 187, mask: (48, 48), op: "SETNNMODE_UU", ver: 71 },
    E { line: 650, first: 187, last: 187, mask: (49, 49), op: "SETNNMODE_SU", ver: 71 },
    E { line: 651, first: 187, last: 187, mask: (50, 50), op: "SETNNMODE_US", ver: 71 },
    E { line: 652, first: 187, last: 187, mask: (51, 51), op: "SETNNMODE_SS", ver: 71 },
    E { line: 654, first: 188, last: 188, mask: (0, 0), op: "LDVPMV_IN", ver: 71 },
    E { line: 655, first: 188, last: 188, mask: (1, 1), op: "LDVPMD_IN", ver: 71 },
    E { line: 656, first: 188, last: 188, mask: (2, 2), op: "LDVPMP", ver: 71 },
    E { line: 658, first: 188, last: 188, mask: (32, 32), op: "RECIP", ver: 71 },
    E { line: 659, first: 188, last: 188, mask: (33, 33), op: "RSQRT", ver: 71 },
    E { line: 660, first: 188, last: 188, mask: (34, 34), op: "EXP", ver: 71 },
    E { line: 661, first: 188, last: 188, mask: (35, 35), op: "LOG", ver: 71 },
    E { line: 662, first: 188, last: 188, mask: (36, 36), op: "SIN", ver: 71 },
    E { line: 663, first: 188, last: 188, mask: (37, 37), op: "RSQRT2", ver: 71 },
    E { line: 664, first: 188, last: 188, mask: (38, 38), op: "BALLOT", ver: 71 },
    E { line: 665, first: 188, last: 188, mask: (39, 39), op: "BCASTF", ver: 71 },
    E { line: 666, first: 188, last: 188, mask: (40, 40), op: "ALLEQ", ver: 71 },
    E { line: 667, first: 188, last: 188, mask: (41, 41), op: "ALLFEQ", ver: 71 },
    E { line: 669, first: 189, last: 189, mask: ANY, op: "LDVPMG_IN", ver: 71 },
    E { line: 672, first: 190, last: 190, mask: ANY, op: "STVPMV", ver: 71 },
    E { line: 673, first: 190, last: 190, mask: ANY, op: "STVPMD", ver: 71 },
    E { line: 674, first: 190, last: 190, mask: ANY, op: "STVPMP", ver: 71 },
    E { line: 686, first: 191, last: 191, mask: (0, 0), op: "VFMOV", ver: 71 },
    E { line: 687, first: 191, last: 191, mask: (5, 5), op: "VFABS", ver: 71 },
    E { line: 688, first: 191, last: 191, mask: (10, 10), op: "VFNEG", ver: 71 },
    E { line: 689, first: 191, last: 191, mask: (15, 15), op: "VFNAB", ver: 71 },
    E { line: 691, first: 192, last: 207, mask: ANY, op: "FCMP", ver: 71 },
    E { line: 692, first: 208, last: 215, mask: ANY, op: "VFCMP", ver: 71 },
    E { line: 693, first: 240, last: 244, mask: ANY, op: "VFMAX", ver: 71 },
    E { line: 695, first: 245, last: 245, mask: (0, 2), op: "FROUND", ver: 71 },
    E { line: 696, first: 245, last: 245, mask: (4, 6), op: "FROUND", ver: 71 },
    E { line: 697, first: 245, last: 245, mask: (8, 10), op: "FROUND", ver: 71 },
    E { line: 698, first: 245, last: 245, mask: (12, 14), op: "FROUND", ver: 71 },
    E { line: 700, first: 245, last: 245, mask: (3, 3), op: "FTOIN", ver: 71 },
    E { line: 701, first: 245, last: 245, mask: (7, 7), op: "FTOIN", ver: 71 },
    E { line: 702, first: 245, last: 245, mask: (11, 11), op: "FTOIN", ver: 71 },
    E { line: 703, first: 245, last: 245, mask: (15, 15), op: "FTOIN", ver: 71 },
    E { line: 705, first: 245, last: 245, mask: (16, 18), op: "FTRUNC", ver: 71 },
    E { line: 706, first: 245, last: 245, mask: (20, 22), op: "FTRUNC", ver: 71 },
    E { line: 707, first: 245, last: 245, mask: (24, 26), op: "FTRUNC", ver: 71 },
    E { line: 708, first: 245, last: 245, mask: (28, 30), op: "FTRUNC", ver: 71 },
    E { line: 710, first: 245, last: 245, mask: (19, 19), op: "FTOIZ", ver: 71 },
    E { line: 711, first: 245, last: 245, mask: (23, 23), op: "FTOIZ", ver: 71 },
    E { line: 712, first: 245, last: 245, mask: (27, 27), op: "FTOIZ", ver: 71 },
    E { line: 713, first: 245, last: 245, mask: (31, 31), op: "FTOIZ", ver: 71 },
    E { line: 715, first: 245, last: 245, mask: (32, 34), op: "FFLOOR", ver: 71 },
    E { line: 716, first: 245, last: 245, mask: (36, 38), op: "FFLOOR", ver: 71 },
    E { line: 717, first: 245, last: 245, mask: (40, 42), op: "FFLOOR", ver: 71 },
    E { line: 718, first: 245, last: 245, mask: (44, 46), op: "FFLOOR", ver: 71 },
    E { line: 720, first: 245, last: 245, mask: (35, 35), op: "FTOUZ", ver: 71 },
    E { line: 721, first: 245, last: 245, mask: (39, 39), op: "FTOUZ", ver: 71 },
    E { line: 722, first: 245, last: 245, mask: (43, 43), op: "FTOUZ", ver: 71 },
    E { line: 723, first: 245, last: 245, mask: (47, 47), op: "FTOUZ", ver: 71 },
    E { line: 725, first: 245, last: 245, mask: (48, 50), op: "FCEIL", ver: 71 },
    E { line: 726, first: 245, last: 245, mask: (52, 54), op: "FCEIL", ver: 71 },
    E { line: 727, first: 245, last: 245, mask: (56, 58), op: "FCEIL", ver: 71 },
    E { line: 728, first: 245, last: 245, mask: (60, 62), op: "FCEIL", ver: 71 },
    E { line: 730, first: 245, last: 245, mask: (51, 51), op: "FTOC", ver: 0 },
    E { line: 731, first: 245, last: 245, mask: (55, 55), op: "FTOC", ver: 0 },
    E { line: 732, first: 245, last: 245, mask: (59, 59), op: "FTOC", ver: 0 },
    E { line: 733, first: 245, last: 245, mask: (63, 63), op: "FTOC", ver: 0 },
    E { line: 735, first: 246, last: 246, mask: (0, 2), op: "FDX", ver: 71 },
    E { line: 736, first: 246, last: 246, mask: (4, 6), op: "FDX", ver: 71 },
    E { line: 737, first: 246, last: 246, mask: (8, 10), op: "FDX", ver: 71 },
    E { line: 738, first: 246, last: 246, mask: (12, 14), op: "FDX", ver: 71 },
    E { line: 739, first: 246, last: 246, mask: (16, 18), op: "FDY", ver: 71 },
    E { line: 740, first: 246, last: 246, mask: (20, 22), op: "FDY", ver: 71 },
    E { line: 741, first: 246, last: 246, mask: (24, 26), op: "FDY", ver: 71 },
    E { line: 742, first: 246, last: 246, mask: (28, 30), op: "FDY", ver: 71 },
    E { line: 744, first: 246, last: 246, mask: (32, 34), op: "ITOF", ver: 71 },
    E { line: 745, first: 246, last: 246, mask: (36, 38), op: "UTOF", ver: 71 },
    E { line: 747, first: 247, last: 247, mask: ANY, op: "VPACK", ver: 71 },
    E { line: 748, first: 248, last: 248, mask: ANY, op: "V8PACK", ver: 71 },
    E { line: 750, first: 249, last: 249, mask: (0, 2), op: "FMOV", ver: 71 },
    E { line: 751, first: 249, last: 249, mask: (4, 6), op: "FMOV", ver: 71 },
    E { line: 752, first: 249, last: 249, mask: (8, 10), op: "FMOV", ver: 71 },
    E { line: 753, first: 249, last: 249, mask: (12, 14), op: "FMOV", ver: 71 },
    E { line: 754, first: 249, last: 249, mask: (16, 18), op: "FMOV", ver: 71 },
    E { line: 755, first: 249, last: 249, mask: (20, 22), op: "FMOV", ver: 71 },
    E { line: 756, first: 249, last: 249, mask: (24, 26), op: "FMOV", ver: 71 },
    E { line: 758, first: 249, last: 249, mask: (3, 3), op: "MOV", ver: 71 },
    E { line: 759, first: 249, last: 249, mask: (7, 7), op: "MOV", ver: 71 },
    E { line: 760, first: 249, last: 249, mask: (11, 11), op: "MOV", ver: 71 },
    E { line: 761, first: 249, last: 249, mask: (15, 15), op: "MOV", ver: 71 },
    E { line: 762, first: 249, last: 249, mask: (19, 19), op: "MOV", ver: 71 },
    E { line: 764, first: 250, last: 250, mask: ANY, op: "V10PACK", ver: 71 },
    E { line: 765, first: 251, last: 251, mask: ANY, op: "V11FPACK", ver: 71 },
    E { line: 767, first: 252, last: 252, mask: ANY, op: "ROTQ", ver: 71 },
    E { line: 768, first: 253, last: 253, mask: ANY, op: "ROT", ver: 71 },
    E { line: 769, first: 254, last: 254, mask: ANY, op: "SHUFFLE", ver: 71 },
];

/// `qpu_pack.c` v3d71_mul_ops, lines 772..
const MUL_OPS: &[E] = &[
    E { line: 774, first: 1, last: 1, mask: ANY, op: "ADD", ver: 71 },
    E { line: 775, first: 2, last: 2, mask: ANY, op: "SUB", ver: 71 },
    E { line: 776, first: 3, last: 3, mask: ANY, op: "UMUL24", ver: 71 },
    E { line: 777, first: 3, last: 3, mask: ANY, op: "UMUL24_RTOP0", ver: 71 },
    E { line: 778, first: 4, last: 8, mask: ANY, op: "VFMUL", ver: 71 },
    E { line: 779, first: 9, last: 9, mask: ANY, op: "SMUL24", ver: 71 },
    E { line: 780, first: 10, last: 10, mask: ANY, op: "MULTOP", ver: 71 },
    E { line: 782, first: 11, last: 11, mask: ANY, op: "V8DOT", ver: 71 },
    E { line: 784, first: 14, last: 14, mask: (0, 2), op: "FMOV", ver: 71 },
    E { line: 785, first: 14, last: 14, mask: (4, 6), op: "FMOV", ver: 71 },
    E { line: 786, first: 14, last: 14, mask: (8, 10), op: "FMOV", ver: 71 },
    E { line: 787, first: 14, last: 14, mask: (12, 14), op: "FMOV", ver: 71 },
    E { line: 788, first: 14, last: 14, mask: (16, 18), op: "FMOV", ver: 71 },
    E { line: 789, first: 14, last: 14, mask: (20, 22), op: "FMOV", ver: 71 },
    E { line: 790, first: 14, last: 14, mask: (24, 26), op: "FMOV", ver: 71 },
    E { line: 791, first: 14, last: 14, mask: (28, 30), op: "FMOV", ver: 71 },
    E { line: 793, first: 14, last: 14, mask: (3, 3), op: "MOV", ver: 71 },
    E { line: 794, first: 14, last: 14, mask: (7, 7), op: "MOV", ver: 71 },
    E { line: 795, first: 14, last: 14, mask: (11, 11), op: "MOV", ver: 71 },
    E { line: 796, first: 14, last: 14, mask: (15, 15), op: "MOV", ver: 71 },
    E { line: 797, first: 14, last: 14, mask: (19, 19), op: "MOV", ver: 71 },
    E { line: 799, first: 14, last: 14, mask: (32, 32), op: "FTOUNORM16", ver: 71 },
    E { line: 800, first: 14, last: 14, mask: (33, 33), op: "FTOSNORM16", ver: 71 },
    E { line: 801, first: 14, last: 14, mask: (34, 34), op: "VFTOUNORM8", ver: 71 },
    E { line: 802, first: 14, last: 14, mask: (35, 35), op: "VFTOSNORM8", ver: 71 },
    E { line: 803, first: 14, last: 14, mask: (36, 36), op: "FUNPACKUNORMLO", ver: 71 },
    E { line: 804, first: 14, last: 14, mask: (37, 37), op: "FUNPACKUNORMHI", ver: 71 },
    E { line: 805, first: 14, last: 14, mask: (38, 38), op: "FUNPACKSNORMLO", ver: 71 },
    E { line: 806, first: 14, last: 14, mask: (39, 39), op: "FUNPACKSNORMHI", ver: 71 },
    E { line: 807, first: 14, last: 14, mask: (48, 48), op: "VFTOUNORM10LO", ver: 71 },
    E { line: 808, first: 14, last: 14, mask: (49, 49), op: "VFTOUNORM10HI", ver: 71 },
    E { line: 810, first: 14, last: 14, mask: (63, 63), op: "NOP", ver: 71 },
    E { line: 812, first: 16, last: 63, mask: ANY, op: "FMUL", ver: 0 },
];

const ADD_ROWS: usize = 144;
const MUL_ROWS: usize = 33;

/// `qpu_instr.c` add_op_args, lines 404..
const ADD_ARGS: &[Args] = &[
    Args { line: 405, op: "FADD", dst: true, a: true, b: true },
    Args { line: 406, op: "FADDNF", dst: true, a: true, b: true },
    Args { line: 407, op: "VFPACK", dst: true, a: true, b: true },
    Args { line: 408, op: "ADD", dst: true, a: true, b: true },
    Args { line: 409, op: "VFPACK", dst: true, a: true, b: true },
    Args { line: 410, op: "SUB", dst: true, a: true, b: true },
    Args { line: 411, op: "VFPACK", dst: true, a: true, b: true },
    Args { line: 412, op: "FSUB", dst: true, a: true, b: true },
    Args { line: 413, op: "MIN", dst: true, a: true, b: true },
    Args { line: 414, op: "MAX", dst: true, a: true, b: true },
    Args { line: 415, op: "UMIN", dst: true, a: true, b: true },
    Args { line: 416, op: "UMAX", dst: true, a: true, b: true },
    Args { line: 417, op: "SHL", dst: true, a: true, b: true },
    Args { line: 418, op: "SHR", dst: true, a: true, b: true },
    Args { line: 419, op: "ASR", dst: true, a: true, b: true },
    Args { line: 420, op: "ROR", dst: true, a: true, b: true },
    Args { line: 421, op: "FMIN", dst: true, a: true, b: true },
    Args { line: 422, op: "FMAX", dst: true, a: true, b: true },
    Args { line: 423, op: "VFMIN", dst: true, a: true, b: true },
    Args { line: 424, op: "VFADD", dst: true, a: true, b: true },
    Args { line: 425, op: "VFSUB", dst: true, a: true, b: true },
    Args { line: 426, op: "VFCMP", dst: true, a: true, b: true },
    Args { line: 427, op: "VFMOV", dst: true, a: true, b: false },
    Args { line: 428, op: "VFABS", dst: true, a: true, b: false },
    Args { line: 429, op: "VFNEG", dst: true, a: true, b: false },
    Args { line: 430, op: "VFNAB", dst: true, a: true, b: false },
    Args { line: 432, op: "AND", dst: true, a: true, b: true },
    Args { line: 433, op: "OR", dst: true, a: true, b: true },
    Args { line: 434, op: "XOR", dst: true, a: true, b: true },
    Args { line: 436, op: "VADD", dst: true, a: true, b: true },
    Args { line: 437, op: "VSUB", dst: true, a: true, b: true },
    Args { line: 438, op: "NOT", dst: true, a: true, b: false },
    Args { line: 439, op: "NEG", dst: true, a: true, b: false },
    Args { line: 440, op: "FLAPUSH", dst: true, a: true, b: false },
    Args { line: 441, op: "FLBPUSH", dst: true, a: true, b: false },
    Args { line: 442, op: "FLPOP", dst: true, a: true, b: false },
    Args { line: 443, op: "RECIP", dst: true, a: true, b: false },
    Args { line: 444, op: "SETMSF", dst: true, a: true, b: false },
    Args { line: 445, op: "SETREVF", dst: true, a: true, b: false },
    Args { line: 446, op: "NOP", dst: false, a: false, b: false },
    Args { line: 447, op: "TIDX", dst: true, a: false, b: false },
    Args { line: 448, op: "EIDX", dst: true, a: false, b: false },
    Args { line: 449, op: "LR", dst: true, a: false, b: false },
    Args { line: 450, op: "VFLA", dst: true, a: false, b: false },
    Args { line: 451, op: "VFLNA", dst: true, a: false, b: false },
    Args { line: 452, op: "VFLB", dst: true, a: false, b: false },
    Args { line: 453, op: "VFLNB", dst: true, a: false, b: false },
    Args { line: 455, op: "FXCD", dst: true, a: false, b: false },
    Args { line: 456, op: "XCD", dst: true, a: false, b: false },
    Args { line: 457, op: "FYCD", dst: true, a: false, b: false },
    Args { line: 458, op: "YCD", dst: true, a: false, b: false },
    Args { line: 460, op: "MSF", dst: true, a: false, b: false },
    Args { line: 461, op: "REVF", dst: true, a: false, b: false },
    Args { line: 462, op: "VDWWT", dst: true, a: false, b: false },
    Args { line: 463, op: "IID", dst: true, a: false, b: false },
    Args { line: 464, op: "SAMPID", dst: true, a: false, b: false },
    Args { line: 465, op: "BARRIERID", dst: true, a: false, b: false },
    Args { line: 466, op: "TMUWT", dst: true, a: false, b: false },
    Args { line: 467, op: "VPMWT", dst: true, a: false, b: false },
    Args { line: 468, op: "FLAFIRST", dst: true, a: false, b: false },
    Args { line: 469, op: "FLNAFIRST", dst: true, a: false, b: false },
    Args { line: 471, op: "VPMSETUP", dst: true, a: true, b: false },
    Args { line: 473, op: "LDVPMV_IN", dst: true, a: true, b: false },
    Args { line: 474, op: "LDVPMV_OUT", dst: true, a: true, b: false },
    Args { line: 475, op: "LDVPMD_IN", dst: true, a: true, b: false },
    Args { line: 476, op: "LDVPMD_OUT", dst: true, a: true, b: false },
    Args { line: 477, op: "LDVPMP", dst: true, a: true, b: false },
    Args { line: 478, op: "RSQRT", dst: true, a: true, b: false },
    Args { line: 479, op: "EXP", dst: true, a: true, b: false },
    Args { line: 480, op: "LOG", dst: true, a: true, b: false },
    Args { line: 481, op: "SIN", dst: true, a: true, b: false },
    Args { line: 482, op: "RSQRT2", dst: true, a: true, b: false },
    Args { line: 483, op: "LDVPMG_IN", dst: true, a: true, b: true },
    Args { line: 484, op: "LDVPMG_OUT", dst: true, a: true, b: true },
    Args { line: 488, op: "FCMP", dst: true, a: true, b: true },
    Args { line: 489, op: "VFMAX", dst: true, a: true, b: true },
    Args { line: 491, op: "FROUND", dst: true, a: true, b: false },
    Args { line: 492, op: "FTOIN", dst: true, a: true, b: false },
    Args { line: 493, op: "FTRUNC", dst: true, a: true, b: false },
    Args { line: 494, op: "FTOIZ", dst: true, a: true, b: false },
    Args { line: 495, op: "FFLOOR", dst: true, a: true, b: false },
    Args { line: 496, op: "FTOUZ", dst: true, a: true, b: false },
    Args { line: 497, op: "FCEIL", dst: true, a: true, b: false },
    Args { line: 498, op: "FTOC", dst: true, a: true, b: false },
    Args { line: 500, op: "FDX", dst: true, a: true, b: false },
    Args { line: 501, op: "FDY", dst: true, a: true, b: false },
    Args { line: 503, op: "STVPMV", dst: false, a: true, b: true },
    Args { line: 504, op: "STVPMD", dst: false, a: true, b: true },
    Args { line: 505, op: "STVPMP", dst: false, a: true, b: true },
    Args { line: 507, op: "ITOF", dst: true, a: true, b: false },
    Args { line: 508, op: "CLZ", dst: true, a: true, b: false },
    Args { line: 509, op: "UTOF", dst: true, a: true, b: false },
    Args { line: 511, op: "MOV", dst: true, a: true, b: false },
    Args { line: 512, op: "FMOV", dst: true, a: true, b: false },
    Args { line: 513, op: "VPACK", dst: true, a: true, b: true },
    Args { line: 514, op: "V8PACK", dst: true, a: true, b: true },
    Args { line: 515, op: "V10PACK", dst: true, a: true, b: true },
    Args { line: 516, op: "V11FPACK", dst: true, a: true, b: true },
    Args { line: 518, op: "BALLOT", dst: true, a: true, b: false },
    Args { line: 519, op: "BCASTF", dst: true, a: true, b: false },
    Args { line: 520, op: "ALLEQ", dst: true, a: true, b: false },
    Args { line: 521, op: "ALLFEQ", dst: true, a: true, b: false },
    Args { line: 522, op: "ROTQ", dst: true, a: true, b: true },
    Args { line: 523, op: "ROT", dst: true, a: true, b: true },
    Args { line: 524, op: "SHUFFLE", dst: true, a: true, b: true },
    Args { line: 525, op: "SETNNMODE_UU", dst: false, a: false, b: false },
    Args { line: 526, op: "SETNNMODE_SU", dst: false, a: false, b: false },
    Args { line: 527, op: "SETNNMODE_US", dst: false, a: false, b: false },
    Args { line: 528, op: "SETNNMODE_SS", dst: false, a: false, b: false },
];

/// `qpu_instr.c` mul_op_args, lines 531..
const MUL_ARGS: &[Args] = &[
    Args { line: 532, op: "ADD", dst: true, a: true, b: true },
    Args { line: 533, op: "SUB", dst: true, a: true, b: true },
    Args { line: 534, op: "UMUL24", dst: true, a: true, b: true },
    Args { line: 535, op: "UMUL24_RTOP0", dst: true, a: true, b: true },
    Args { line: 536, op: "VFMUL", dst: true, a: true, b: true },
    Args { line: 537, op: "SMUL24", dst: true, a: true, b: true },
    Args { line: 538, op: "MULTOP", dst: true, a: true, b: true },
    Args { line: 539, op: "FMOV", dst: true, a: true, b: false },
    Args { line: 540, op: "NOP", dst: false, a: false, b: false },
    Args { line: 541, op: "MOV", dst: true, a: true, b: false },
    Args { line: 542, op: "FMUL", dst: true, a: true, b: true },
    Args { line: 543, op: "FTOUNORM16", dst: true, a: true, b: false },
    Args { line: 544, op: "FTOSNORM16", dst: true, a: true, b: false },
    Args { line: 545, op: "VFTOUNORM8", dst: true, a: true, b: false },
    Args { line: 546, op: "VFTOSNORM8", dst: true, a: true, b: false },
    Args { line: 547, op: "FUNPACKUNORMLO", dst: true, a: true, b: false },
    Args { line: 548, op: "FUNPACKUNORMHI", dst: true, a: true, b: false },
    Args { line: 549, op: "FUNPACKSNORMLO", dst: true, a: true, b: false },
    Args { line: 550, op: "FUNPACKSNORMHI", dst: true, a: true, b: false },
    Args { line: 551, op: "VFTOUNORM10LO", dst: true, a: true, b: false },
    Args { line: 552, op: "VFTOUNORM10HI", dst: true, a: true, b: false },
    Args { line: 553, op: "V8DOT", dst: true, a: true, b: true },
];

fn key(s: &str) -> String {
    s.replace('_', "").to_lowercase()
}

/// An operation of either ALU.
#[derive(Clone, Copy)]
enum Op {
    A(AddOp71),
    M(MulOp71),
}

impl Op {
    fn all() -> Vec<Op> {
        AddOp71::ALL.iter().map(|o| Op::A(*o)).chain(MulOp71::ALL.iter().map(|o| Op::M(*o))).collect()
    }
    fn name(self) -> String {
        match self {
            Op::A(o) => format!("add {o:?}"),
            Op::M(o) => format!("mul {o:?}"),
        }
    }
    fn key(self) -> String {
        match self {
            Op::A(o) => key(&format!("{o:?}")),
            Op::M(o) => key(&format!("{o:?}")),
        }
    }
    fn is_add(self) -> bool {
        matches!(self, Op::A(_))
    }
    fn rows(self) -> &'static [E] {
        if self.is_add() {
            ADD_OPS
        } else {
            MUL_OPS
        }
    }
    fn mesa_args(self) -> Option<&'static Args> {
        let t = if self.is_add() { ADD_ARGS } else { MUL_ARGS };
        t.iter().find(|a| key(a.op) == self.key())
    }
    /// The model's own operand count and destination.
    fn model_args(self) -> u8 {
        match self {
            Op::A(o) => add_op_args(o),
            Op::M(o) => mul_op_args(o),
        }
    }
    /// The rows of this operation (by `key`).
    fn own_rows(self) -> Vec<usize> {
        let k = self.key();
        self.rows().iter().enumerate().filter(|(_, e)| key(e.op) == k).map(|(i, _)| i).collect()
    }
    fn nsrc(self) -> usize {
        let a = self.mesa_args().expect("operation has an entry in Mesa's *_op_args");
        if a.b {
            2
        } else if a.a {
            1
        } else {
            0
        }
    }
    /// An instruction of this operation alone (the other ALU is a nop).
    fn instr(self, au: Unpack71, bu: Unpack71, pack: OutPack) -> Instr71 {
        let a = self.mesa_args().expect("Mesa args");
        let n = self.nsrc();
        let stvpm = matches!(self, Op::A(AddOp71::Stvpmv | AddOp71::Stvpmd | AddOp71::Stvpmp));
        let (waddr, magic, pack) = if a.dst {
            (7, false, pack)
        } else if stvpm {
            (0, false, OutPack::None)
        } else {
            (WADDR_NOP, true, OutPack::None)
        };
        let ra = Operand { raddr: if n >= 1 { 3 } else { 0 }, unpack: au };
        let rb = Operand { raddr: if n >= 2 { 5 } else { 0 }, unpack: bu };
        let mut alu = Alu71::nop();
        match self {
            Op::A(op) => alu.add = AddAlu71 { op, a: ra, b: rb, waddr, magic, pack },
            Op::M(op) => alu.mul = MulAlu71 { op, a: ra, b: rb, waddr, magic, pack },
        }
        Instr71::Alu(alu)
    }
    /// The opcode field and the raddr that completes it, of a word.
    fn fields(self, w: u64) -> (u32, u8) {
        if self.is_add() {
            (((w >> 24) & 0xff) as u32, (w & 63) as u8)
        } else {
            (((w >> 58) & 63) as u32, ((w >> 12) & 63) as u8)
        }
    }
    fn patch(self, w: u64, opc: u32, raddr: u8) -> u64 {
        if self.is_add() {
            (w & !(0xff << 24) & !63) | (opc as u64) << 24 | raddr as u64
        } else {
            (w & !(63 << 58) & !(63 << 12)) | (opc as u64) << 58 | (raddr as u64) << 12
        }
    }
    /// The key of the operation of this ALU in a decoded instruction.
    fn decoded(self, i: &Instr71) -> Option<String> {
        let Instr71::Alu(a) = i else { return None };
        Some(match self {
            Op::A(_) => key(&format!("{:?}", a.add.op)),
            Op::M(_) => key(&format!("{:?}", a.mul.op)),
        })
    }
}

/// Mesa's `lookup_opcode_from_packed` for ver 71 (`qpu_pack.c:833`): the first
/// entry whose opcode range and raddr mask hold.
fn lookup(rows: &'static [E], opc: u32, raddr: u8) -> Option<usize> {
    rows.iter().position(|e| opc >= e.first && opc <= e.last && e.ver <= 71 && raddr >= e.mask.0 && raddr <= e.mask.1)
}

/// Operations one table row names for several (operand order, waddr or the MA
/// bit decide, not the opcode).
fn same_op(got: &str, want: &str) -> bool {
    const GROUPS: [&[&str]; 6] = [
        &["fadd", "faddnf"],
        &["fmin", "fmax"],
        &["stvpmv", "stvpmd", "stvpmp"],
        &["ldvpmvin", "ldvpmvout"],
        &["ldvpmdin", "ldvpmdout"],
        &["ldvpmgin", "ldvpmgout"],
    ];
    got == want || GROUPS.iter().any(|g| g.contains(&got) && g.contains(&want))
}

/// The Mesa operations (rows or args) with no variant, by `key`.
fn mesa_without_variant() -> Vec<String> {
    let have: Vec<String> = Op::all().iter().map(|o| format!("{}:{}", o.is_add() as u8, o.key())).collect();
    let mut out = Vec::new();
    for (add, names) in [
        (1, ADD_OPS.iter().map(|e| e.op).chain(ADD_ARGS.iter().map(|a| a.op)).collect::<Vec<_>>()),
        (0, MUL_OPS.iter().map(|e| e.op).chain(MUL_ARGS.iter().map(|a| a.op)).collect::<Vec<_>>()),
    ] {
        for n in names {
            let s = format!("{}:{}", add, key(n));
            if !have.contains(&s) && !out.contains(&format!("{} {}", if add == 1 { "add" } else { "mul" }, n)) {
                out.push(format!("{} {}", if add == 1 { "add" } else { "mul" }, n));
            }
        }
    }
    out
}

/// Mesa's add operations with no entry in `v3d71_add_ops` (so Mesa's pack
/// refuses them, `qpu_pack.c:1856`): VPMSETUP has an `add_op_args` line but
/// no 7.1 row; the three `LDVPM*_OUT` are the `_IN` entries with the MA bit
/// (`qpu_pack.c:1190`, :1860 for the pack side `assert`s only).
const ADD_WITHOUT_ENTRY: [&str; 4] = ["Vpmsetup", "LdvpmvOut", "LdvpmdOut", "LdvpmgOut"];

/// Mesa operations with no variant of ours: `UMUL24_RTOP0` is `UMUL24`'s
/// opcode (`isa::MulOp` says so); `VDWWT` has an `add_op_args` line and no
/// 7.1 row.
const MESA_WITHOUT_VARIANT: [&str; 2] = ["add VDWWT", "mul UMUL24_RTOP0"];

/// Rows no encoding of its operation's variant reached (by line), see
/// `every_entry_is_reached_or_listed`. Two causes, both Mesa's: the ABS rows
/// of FROUND, FTOIN, FTRUNC, FTOIZ, FFLOOR, FTOUZ, FCEIL, FTOC, FDX and FDY
/// (raddr 0..2, 3, 16..18, 19, 32..34, 35, 48..50, 51; Mesa's pack refuses
/// `packed == 0`, `qpu_pack.c:2024`, `:2043`), and the L and H rows of
/// FROUND, FTRUNC, FFLOOR, FCEIL, FDX and FDY (raddr 8..10, 12..14, ...): Mesa
/// packs the input unpack into raddr_b bits 3..2 (`qpu_pack.c:2026`) but unpacks
/// it from the opcode, `(op >> 2) & 3` (`qpu_pack.c:1302`), which is 1 (none)
/// for opcode 245/246, so its own pack of L or H is not reproduced by its
/// unpack and `codec71` refuses the word ([`l_and_h_input_unpacks_round_trip`]).
/// The one mul row is FMOV's raddr_d 28..30 (`qpu_pack.c:791`): input unpack 7,
/// which `v3d_qpu_float32_unpack_pack` (:899) and its inverse have no value
/// for, so no instruction packs or unpacks to it (a word is Mesa-refused).
const UNREACHED_ADD: [u32; 22] =
    [695, 697, 698, 700, 705, 707, 708, 710, 715, 717, 718, 720, 725, 727, 728, 730, 735, 737, 738, 739, 741, 742];
const UNREACHED_MUL: [u32; 1] = [791];

/// Pinned totals of the sweep: words that `decode71` accepted and refused.
const SWEEP_ADD: (usize, usize) = (20144, 638);
const SWEEP_MUL: (usize, usize) = (3877, 3);

#[test]
fn transcription_is_whole() {
    assert_eq!(ADD_OPS.len(), ADD_ROWS);
    assert_eq!(MUL_OPS.len(), MUL_ROWS);
    for t in [ADD_OPS, MUL_OPS] {
        assert!(t.windows(2).all(|w| w[0].line < w[1].line));
        for e in t {
            assert!(e.first <= e.last && e.mask.0 <= e.mask.1, "line {}", e.line);
            assert!(e.last < if std::ptr::eq(t, ADD_OPS) { 256 } else { 64 }, "line {}", e.line);
        }
    }
}

/// Each operation of the model: Mesa's operand count and destination are
/// the model's.
#[test]
fn operand_counts_and_destinations_are_mesas() {
    let mut bad = Vec::new();
    for op in Op::all() {
        let Some(m) = op.mesa_args() else {
            bad.push(format!("{}: no Mesa *_op_args line", op.name()));
            continue;
        };
        let args = op.model_args();
        let mesa = (m.dst as u8 * ARG_D) | (m.a as u8 * ARG_A) | (m.b as u8 * ARG_B);
        if args != mesa || num_src(args) != op.nsrc() {
            bad.push(format!("{}: model {args:#x}, Mesa {mesa:#x} (qpu_instr.c:{})", op.name(), m.line));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn variants_and_mesa_operations_correspond() {
    let no_entry: Vec<String> = Op::all().into_iter().filter(|o| o.own_rows().is_empty()).map(|o| o.name()).collect();
    let want: Vec<String> = ADD_WITHOUT_ENTRY.iter().map(|n| format!("add {n}")).collect();
    assert_eq!(no_entry, want, "variants with no entry in Mesa's 7.1 tables");
    assert_eq!(mesa_without_variant(), MESA_WITHOUT_VARIANT, "Mesa operations with no variant");
    // Mesa refuses to pack a variant it has no entry for; we say so too.
    for n in ADD_WITHOUT_ENTRY {
        let op = AddOp71::ALL.iter().copied().find(|o| format!("{o:?}") == n).unwrap();
        let i = Op::A(op).instr(Unpack71::None, Unpack71::None, OutPack::None);
        assert!(encode71(&i).is_err(), "{n} encodes though Mesa 7.1 has no entry for it");
    }
}

/// Encode one operation with every unpack and pack; returns the rows hit and
/// the problems.
fn walk(op: Op, hit: &mut Vec<usize>) -> Vec<String> {
    let mut bad = Vec::new();
    let n = op.nsrc();
    let a_unp: &[Unpack71] = if n >= 1 { &Unpack71::ALL } else { &[Unpack71::None] };
    let b_unp: &[Unpack71] = if n >= 2 { &Unpack71::ALL } else { &[Unpack71::None] };
    let packs: &[OutPack] = if op.mesa_args().unwrap().dst { &OutPack::ALL } else { &[OutPack::None] };
    let rows = op.own_rows();
    let mut encoded = 0;
    for &au in a_unp {
        for &bu in b_unp {
            for &p in packs {
                let i = op.instr(au, bu, p);
                let Ok(w) = encode71(&i) else { continue };
                encoded += 1;
                let back = match decode71(w) {
                    Ok(b) => b,
                    Err(e) => {
                        bad.push(format!("{}: {w:#018x} does not decode: {e:?}", op.name()));
                        continue;
                    }
                };
                let got = op.decoded(&back).unwrap();
                if !same_op(&got, &op.key()) {
                    bad.push(format!("{}: {w:#018x} decodes to {got}", op.name()));
                }
                let (opc, ra) = op.fields(w);
                // the first of its own rows that holds the word's fields
                let own = rows.iter().copied().find(|&r| {
                    let e = &op.rows()[r];
                    opc >= e.first && opc <= e.last && ra >= e.mask.0 && ra <= e.mask.1
                });
                let Some(own) = own else {
                    bad.push(format!("{}: {w:#018x} (opcode {opc}, raddr {ra}) is in none of its rows", op.name()));
                    continue;
                };
                hit.push(own);
                // Mesa's first match names this operation (or the one the
                // order of operands, the waddr or the MA bit tells apart)
                let first = lookup(op.rows(), opc, ra).map(|r| key(op.rows()[r].op));
                if !first.as_deref().is_some_and(|f| same_op(f, &op.key())) {
                    bad.push(format!(
                        "{}: {w:#018x} (opcode {opc}, raddr {ra}) Mesa's lookup gives {first:?}",
                        op.name()
                    ));
                }
            }
        }
    }
    if encoded == 0 {
        bad.push(format!("{}: no combination encodes", op.name()));
    }
    bad
}

fn walk_all(add: bool) -> (Vec<String>, Vec<u32>) {
    let mut bad = Vec::new();
    let mut unreached = Vec::new();
    for op in Op::all().into_iter().filter(|o| o.is_add() == add && !o.own_rows().is_empty()) {
        let mut hit = Vec::new();
        bad.extend(walk(op, &mut hit));
        for r in op.own_rows() {
            if !hit.contains(&r) {
                unreached.push(op.rows()[r].line);
            }
        }
    }
    unreached.sort();
    (bad, unreached)
}

#[test]
fn add_variants_pack_and_unpack_as_mesa() {
    let (bad, _) = walk_all(true);
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn mul_variants_pack_and_unpack_as_mesa() {
    let (bad, _) = walk_all(false);
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn every_entry_is_reached_or_listed() {
    let (_, add) = walk_all(true);
    let (_, mul) = walk_all(false);
    assert_eq!(add, UNREACHED_ADD, "add rows (qpu_pack.c lines) no encoding reaches");
    assert_eq!(mul, UNREACHED_MUL, "mul rows (qpu_pack.c lines) no encoding reaches");
}

/// Every opcode and raddr of every row, in an otherwise valid word: when
/// `decode71` accepts it, it names the operation Mesa's first-match lookup
/// gives.
fn sweep(add: bool) -> (usize, usize, Vec<String>) {
    let base_op = if add { Op::A(AddOp71::Fadd) } else { Op::M(MulOp71::Fmul) };
    let base = encode71(&base_op.instr(Unpack71::None, Unpack71::None, OutPack::None)).unwrap();
    let rows = base_op.rows();
    let (mut ok, mut err, mut bad) = (0, 0, Vec::new());
    for e in rows {
        for opc in e.first..=e.last {
            for ra in e.mask.0..=e.mask.1 {
                let w = base_op.patch(base, opc, ra);
                let want = key(rows[lookup(rows, opc, ra).unwrap()].op);
                match decode71(w) {
                    Ok(i) => {
                        ok += 1;
                        let got = base_op.decoded(&i).unwrap();
                        // UMUL24_RTOP0 is UMUL24 (no variant of its own)
                        if !same_op(&got, &want) && !(want == "umul24rtop0" && got == "umul24") {
                            bad.push(format!(
                                "line {}: opcode {opc} raddr {ra}: {w:#018x} decodes to {got}, Mesa's lookup gives {want}",
                                e.line
                            ));
                        }
                    }
                    Err(_) => err += 1,
                }
            }
        }
    }
    (ok, err, bad)
}

#[test]
fn add_table_sweep_agrees_with_mesa_lookup() {
    let (ok, err, bad) = sweep(true);
    assert!(bad.is_empty(), "{bad:#?}");
    assert_eq!((ok, err), SWEEP_ADD);
}

#[test]
fn mul_table_sweep_agrees_with_mesa_lookup() {
    let (ok, err, bad) = sweep(false);
    assert!(bad.is_empty(), "{bad:#?}");
    assert_eq!((ok, err), SWEEP_MUL);
}

/// Mesa 26.2.1 packs L/H on the input of FROUND, FTRUNC, FFLOOR, FCEIL, FDX
/// and FDY and does not read them back (see [`UNREACHED_ADD`]): these
/// instructions cannot be encoded at 7.1 until that is fixed in Mesa (and in
/// the vendored copy), though the table lists their rows.
#[test]
#[ignore = "Mesa 26.2.1 qpu_pack.c:1302 unpacks the unpack of FROUND/FTRUNC/FFLOOR/FCEIL/FDX/FDY from the opcode, not raddr_b (:2026 packs it there)"]
fn l_and_h_input_unpacks_round_trip() {
    let mut bad = Vec::new();
    for op in [AddOp71::Fround, AddOp71::Ftrunc, AddOp71::Ffloor, AddOp71::Fceil, AddOp71::Fdx, AddOp71::Fdy] {
        for u in [Unpack71::L, Unpack71::H] {
            let i = Op::A(op).instr(u, Unpack71::None, OutPack::None);
            if let Err(e) = encode71(&i) {
                bad.push(format!("{op:?} with unpack {u:?}: {e:?}"));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
