// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000UL's NON_POWER_OF_TWO bit, pinned from the etnaviv hardware
//! database, and the GL_OES_texture_npot claim that rests on it.
//!
//! Mesa's etnaviv driver claims GL_OES_texture_npot only where the part's
//! NON_POWER_OF_TWO feature is set (docs/es2-cts-failures.md section H),
//! and its hwdb reader turns the database bit into that feature:
//! `ETNA_FEATURE(REG_NonPowerOfTwo, NON_POWER_OF_TWO)` in
//! `src/etnaviv/hwdb/etna_hwdb.c:65` (tag mesa-26.2.1). The authority on
//! what the i.MX8MP's part is is NXP's own feature database, which Mesa
//! 26.2.1 carries at `src/etnaviv/hwdb/nxp/gc_feature_database.h` (our
//! `third_party/mesa-26.2.1` vendors only the directories yakogl builds
//! against, not `src/etnaviv`).
//!
//! The board's part is model 0x7000, revision 0x6204, product 0x70003
//! (measured on the board: tools/cts/RESULTS.md "GC7000 identity (rev
//! 0x6204, product 0x70003)"; the removed gc_identify sample expected
//! model 0x7000 revision 0x6204). Two database entries match those ids,
//! both named `gc7000ULN_v123`:
//!
//! | entry | ChipID | ChipRevision | ProductID | EcoID | CustomerID | entry line |
//! |---|---|---|---|---|---|---|
//! | `gc7000ULN_v123` | 0x7000 | 0x6204 | 0x70003 | 0x0 | 0x11 | :18841 |
//! | `gc7000ULN_v123` | 0x7000 | 0x6204 | 0x70003 | 0x1 | 0x11 | :19735 |
//!
//! Both set `gcFEATURE_BIT_REG_NonPowerOfTwo` to 1 (:19036 and :19930).
//! The same entries read `gcFEATURE_BIT_REG_Halti5` 1 (:19172),
//! `gcFEATURE_BIT_REG_UnifiedSamplers` 1 (:19090),
//! `gcFEATURE_BIT_REG_SeamlessCubeMap` 1 (:19049),
//! `gcFEATURE_BIT_REG_LinearTextureSupport` 1 (:19037) and
//! `gcFEATURE_BIT_NO_ASTC` 1 (:19200): a HALTI5 core with unified
//! samplers, seamless cube maps and linear textures, and no ASTC. The
//! older GC7000UL entries set the bit too (`gc7000UL_6200` at :15263,
//! ChipRevision 0x6202, bit at :15460; `gc7000ULN_v122` at :17945,
//! ChipRevision 0x6203, bit at :18142), so every GC7000UL in the database
//! has NPOT sampling. (The stock `GC7000L` hwdb entry is a different part,
//! revision 0x6214: tools/vivante/README.md.)
//!
//! The board's own customer and eco ids were never recorded
//! (driver/docs/vivante-kernel-rust.md: "product id 0x70003 match ...
//! customer and eco ids not recorded"); the entries match on the ids that
//! were.

#![cfg(feature = "gpu-vivante")]

/// One `gc_feature_database.h` entry, as far as this pin reads it.
struct Entry {
    chip_id: u32,
    chip_revision: u32,
    product_id: u32,
    eco_id: u32,
    customer_id: u32,
    /// `gcFEATURE_BIT_REG_NonPowerOfTwo`, and the line it is on.
    non_power_of_two: (bool, u32),
}

/// The two entries matching the board's ids, from the table above.
const BOARD_ENTRIES: [Entry; 2] = [
    Entry {
        chip_id: 0x7000,
        chip_revision: 0x6204,
        product_id: 0x70003,
        eco_id: 0x0,
        customer_id: 0x11,
        non_power_of_two: (true, 19036),
    },
    Entry {
        chip_id: 0x7000,
        chip_revision: 0x6204,
        product_id: 0x70003,
        eco_id: 0x1,
        customer_id: 0x11,
        non_power_of_two: (true, 19930),
    },
];

/// The bit, as recorded in `vivante::backend::NON_POWER_OF_TWO`, agrees
/// with every database entry for the board's part. The GC7000 build's
/// GL_OES_texture_npot claim (`gl::state::NPOT_EXTENSION`) is gated on
/// this constant, so pinning it pins the claim.
#[test]
fn the_gc7000ul_has_non_power_of_two() {
    for e in &BOARD_ENTRIES {
        assert_eq!(
            (e.chip_id, e.chip_revision, e.product_id, e.customer_id),
            (0x7000, 0x6204, 0x70003, 0x11),
            "the entry is the board's part"
        );
        let (bit, line) = e.non_power_of_two;
        assert_eq!(
            yakogl::vivante::backend::NON_POWER_OF_TWO,
            bit,
            "gc_feature_database.h:{line} (Mesa 26.2.1), gc7000ULN_v123 eco 0x{:x}",
            e.eco_id
        );
    }
}
