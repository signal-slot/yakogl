// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `StageMeta::of`'s varying counts (docs/notes/mali-draw-limits.md): a vertex
//! layout row of no component, which Mesa's link leaves out of the stored
//! varyings, is not a varying written.
//!
//! Feature set: `--no-default-features --features
//! host-test,gpu-mali,shaderc-online,no-shader-table`.

use super::*;
use crate::shaderc::online::mali::{VaryingSlot, MALI_RECORD_MAGIC};

fn record(stage: u32, varyings: &[VaryingSlot]) -> MaliShader<'_> {
    MaliShader {
        magic: MALI_RECORD_MAGIC,
        stage,
        preload: 0,
        work_reg_count: 0,
        secondary: None,
        no_psiz_offset: None,
        fau: &[],
        attributes: &[],
        varyings,
        colour_f16: false,
        colour_outputs: 0,
        compacted: false,
        moved: &[],
        sampler: None,
        samplers: &[],
        cube_samplers: 0,
        helper_threads: false,
        can_discard: false,
        ubo: &[],
    }
}

const POS: VaryingSlot = VaryingSlot { varying: None, comps: 4, section: Section::Position, offset: 0, mask: 0xf };

fn generic(location: u8, mask: u8, offset: u16) -> VaryingSlot {
    VaryingSlot { varying: Some(location), comps: mask.count_ones() as u8, section: Section::Generic, offset, mask }
}

fn counts(stage: u32, rows: &[VaryingSlot]) -> (u32, u32) {
    let meta = StageMeta::of(&record(stage, rows)).expect("a valid record");
    let meta = meta.meta();
    (meta.varyings, meta.varying_bytes)
}

/// Every lane folded to a constant: the layout holds only placeholder rows,
/// so the vertex shader writes no varying (the 24 `shaders.matrix` cases).
#[test]
fn vertex_all_placeholders_write_no_varying() {
    let rows = [POS, generic(0, 0, 0), generic(1, 0, 0)];
    assert_eq!(counts(SHC_STAGE_VERTEX, &rows), (0, 0));
}

/// A placeholder beside a stored varying is not counted; the bytes are the
/// stored row's.
#[test]
fn vertex_placeholder_beside_stored_row() {
    let rows = [POS, generic(0, 0, 0), generic(1, 0x7, 0), generic(2, 0, 12)];
    assert_eq!(counts(SHC_STAGE_VERTEX, &rows), (1, 12));
}

/// Stored rows are all counted, partial masks by their components.
#[test]
fn vertex_stored_rows_counted() {
    let rows = [POS, generic(0, 0xf, 0), generic(1, 0x3, 16)];
    assert_eq!(counts(SHC_STAGE_VERTEX, &rows), (2, 24));
}

/// A fragment record's rows are the varyings read: counted as before, with
/// no bytes.
#[test]
fn fragment_rows_counted_as_read() {
    let rows = [generic(0, 0x7, 0), generic(1, 0x1, 12)];
    assert_eq!(counts(SHC_STAGE_FRAGMENT, &rows), (2, 0));
}
