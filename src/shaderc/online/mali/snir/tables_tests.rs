// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
//! Tests of the generated tables (opcodes.rs, intrinsics.rs): they read as
//! Mesa's nir_op_info and nir_intrinsic_info do.

use super::intrinsics::{flags, NirIndex, NirIntrinsic, MAX_CONST_INDEX, NIR_INTRINSIC_INFOS};
use super::opcodes::{alu_type, fp, op_is, NirOp, SearchConv, NIR_OP_INFOS, REACHABLE};

/// The enum's value is the row of its info, and names are sorted, as
/// nir_opcodes_h.py and nir_opcodes_c.py both walk `sorted(opcodes)`.
#[test]
fn every_opcode_is_its_row_and_its_name() {
    assert_eq!(NirOp::COUNT, NIR_OP_INFOS.len());
    for (k, info) in NIR_OP_INFOS.iter().enumerate() {
        let op = NirOp::from_u16(k as u16).unwrap();
        assert_eq!(op as usize, k);
        assert_eq!(NirOp::from_name(info.name), Some(op));
        assert_eq!(info.input_sizes.len(), info.num_inputs as usize);
        assert_eq!(info.input_types.len(), info.num_inputs as usize);
    }
    assert!(NIR_OP_INFOS.windows(2).all(|w| w[0].name < w[1].name));
    assert_eq!(NirOp::from_u16(NirOp::COUNT as u16), None);
    assert_eq!(NirOp::from_name("no_such_op"), None);
}

/// A few rows read against nir_opcodes.py.
#[test]
fn rows_hold_mesa_s_facts() {
    let fadd = NirOp::Fadd.info();
    assert_eq!((fadd.name, fadd.num_inputs, fadd.output_size, fadd.output_type), ("fadd", 2, 0, alu_type::FLOAT));
    assert_eq!(fadd.algebraic_properties, op_is::_2SRC_COMMUTATIVE | op_is::INEXACT_ASSOCIATIVE);
    assert_eq!(fadd.valid_fp_math_ctrl, fp::PRESERVE_SZ_INF_NAN | fp::EXACT);
    let dot = NirOp::Fdot4.info();
    assert_eq!((dot.output_size, dot.input_sizes), (1, &[4u8, 4][..]));
    let iand = NirOp::Iand.info();
    assert_eq!(iand.algebraic_properties, op_is::_2SRC_COMMUTATIVE | op_is::ASSOCIATIVE);
    assert_eq!((iand.output_type, iand.valid_fp_math_ctrl), (alu_type::UINT, 0));
    let f2f16 = NirOp::F2f16.info();
    assert!(f2f16.is_conversion);
    assert_eq!(f2f16.output_type, alu_type::FLOAT | 16);
    assert_eq!(f2f16.output_type & alu_type::SIZE_MASK, 16);
    assert_eq!(NirOp::Bcsel.info().algebraic_properties, op_is::SELECTION);
    assert_eq!(NirOp::Pack32_4x8Split.name(), "pack_32_4x8_split");
}

#[test]
fn search_conversions_round_trip() {
    for (c, bits) in [(SearchConv::F2f, 16), (SearchConv::F2f, 32), (SearchConv::U2u, 8), (SearchConv::B2i, 64)] {
        let op = c.sized(bits).unwrap();
        assert_eq!(op.search_conv(), Some(c));
        assert_eq!(op.info().output_type & alu_type::SIZE_MASK, bits);
    }
    assert_eq!(SearchConv::F2f.sized(8), None);
    assert_eq!(NirOp::Fadd.search_conv(), None);
}

/// REACHABLE holds snir_ops.txt and is what the rows say.
#[test]
fn reachable_is_the_rows_flag() {
    for op in REACHABLE {
        assert!(op.is_reachable());
    }
    assert_eq!(NIR_OP_INFOS.iter().filter(|i| i.reachable).count(), REACHABLE.len());
    for op in [NirOp::Fadd, NirOp::Ffma, NirOp::Vec4, NirOp::Fcanonicalize, NirOp::Pack32_4x8Split] {
        assert!(op.is_reachable(), "{}", op.name());
    }
}

/// load_input's const indices sit where nir_intrinsics.py puts them:
/// [BASE, RANGE, COMPONENT, DEST_TYPE, IO_SEMANTICS], io semantics two slots.
#[test]
fn intrinsic_indices_are_in_mesa_s_slots() {
    let li = NirIntrinsic::LoadInput;
    assert_eq!(li.name(), "load_input");
    assert_eq!(li.slot(NirIndex::Base), Some(0));
    assert_eq!(li.slot(NirIndex::DestType), Some(3));
    assert_eq!(li.slot(NirIndex::IoSemantics), Some(4));
    assert_eq!(li.slot(NirIndex::WriteMask), None);
    assert_eq!(li.info().num_index_slots, 6);
    assert_eq!(NirIndex::IoSemantics.info().slots, 2);
    assert_eq!(NirIndex::WriteMask.info().name, "write_mask");
    let ubo = NirIntrinsic::LoadUbo.info();
    assert_eq!(ubo.src_components, &[-1, 1]);
    assert_eq!(ubo.flags, flags::CAN_ELIMINATE | flags::CAN_REORDER);
    for info in &NIR_INTRINSIC_INFOS {
        assert!(info.num_index_slots as usize <= MAX_CONST_INDEX, "{}", info.name);
    }
    assert!(NIR_INTRINSIC_INFOS.windows(2).all(|w| w[0].name < w[1].name));
}
