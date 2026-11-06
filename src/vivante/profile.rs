// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What one Vivante part is, as this backend needs to know it: the ids, the
//! limits and the feature bits of its hardware database row, and the line
//! width range the GL core reports for it.
//!
//! Two parts of the same family: the i.MX8MP's GC7000UL, which the backend
//! was written for and the only one it has run on, and the STM32MP257F's
//! `GCNANOULTRA31_VIP2` (model 0x8000, revision 0x6205, product 0x80003,
//! eco 0, read on the board under ST's Linux, issue #1281). Every value is
//! from Mesa 26.2.1's copies of the vendors' databases (tag mesa-26.2.1):
//! NXP's `src/etnaviv/hwdb/nxp/gc_feature_database.h`, the `gc7000ULN_v123`
//! entry at :18839, and ST's `src/etnaviv/hwdb/st/gc_feature_database.h`,
//! the `GCNANOULTRA31_VIP2` entry at :4720. The comparison is section 3 of
//! docs/notes/pre-study-stm32mp257f-dk.md.
//!
//! The backend is built for one part, [`ACTIVE`], which is the i.MX8MP's:
//! nothing carries the GPU's identity to the backend at run time yet, so
//! the ST profile is data and its test, not a path.

use core::ffi::CStr;

/// One part: its database row as far as this backend reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChipProfile {
    /// The backend's name (`YakoglBackend::name`).
    pub name: &'static CStr,
    /// `ChipID`, `ChipRevision`, `ProductID`: what [`lookup`] matches.
    pub model: u32,
    pub revision: u32,
    pub product: u32,
    /// `EcoID` and `CustomerID` of the entry cited. The lookup does not
    /// match them: the i.MX8MP's two entries (eco 0 and 1) and ST's two
    /// (eco 0 and 1, :4720 and :5651) read the same in every field here,
    /// and neither board's customer id was read.
    pub eco: u32,
    pub customer: u32,

    /// `gcFEATURE_VALUE_InstructionCount`.
    pub instructions: u32,
    /// `gcFEATURE_VALUE_TempRegisters`.
    pub temp_registers: u32,
    /// `gcFEATURE_VALUE_NumberOfConstants`, vec4 registers of both stages
    /// ([`super::regs::GC_NUM_CONSTANTS`]).
    pub constants: u32,
    /// `gcFEATURE_VALUE_VaryingCount` ([`super::state::GC_MAX_VARYINGS`]).
    pub varyings: u32,
    /// `gcFEATURE_VALUE_Streams` (the GL core's `MAX_VERTEX_ATTRIBS`,
    /// [`super::state::GC_MAX_VERTEX_ELEMENTS`]).
    pub streams: u32,
    /// `gcFEATURE_VALUE_NumPixelPipes`.
    pub pixel_pipes: u32,
    /// `gcFEATURE_VALUE_NumShaderCores`.
    pub shader_cores: u32,
    /// `gcFEATURE_VALUE_ThreadCount`.
    pub threads: u32,

    /// `gcFEATURE_BIT_REG_NonPowerOfTwo`: the texture unit samples an NPOT
    /// texture with any wrap mode ([`super::backend::NON_POWER_OF_TWO`]).
    pub non_power_of_two: bool,
    /// `gcFEATURE_BIT_WIDELINE_TRIANGLE_EMU`: wide lines are drawn as
    /// triangles by the driver, so Mesa reports a line width of 1.0 at most
    /// (`etnaviv_screen.c:366-368`), and so does [`Self::line_width_range`].
    pub wideline_triangle_emu: bool,
    /// `gcFEATURE_BIT_SH_NO_ONECONST_LIMIT`: one instruction may read two
    /// different uniforms (Mesa `etnaviv_screen.c:893-894`). Our compiler
    /// obeys the stricter rule everywhere, which stays valid where it is set.
    pub no_oneconst_limit: bool,

    /* The bits below are in the rows and Mesa maps none of them (no
     * ETNA_FEATURE in etna_hwdb.c); nothing here reads them. They are kept
     * so a difference between the parts' conformance results has a list to
     * be checked against (pre-study section 3). */
    /// `gcFEATURE_BIT_SH_NO_INDEX_CONST_ON_A0`: by its name, constants
    /// cannot be read through the address register.
    pub no_index_const_on_a0: bool,
    /// `gcFEATURE_BIT_SH_MOVAI_MOVAR_UNUSED_COMPONENTS_WRITE_DIRTY_DATA_FIX`:
    /// by its name, a partial-component address register write leaves the
    /// other components alone. Set on the i.MX8MP, absent on the ST part.
    pub movai_movar_dirty_data_fix: bool,
    /// `gcFEATURE_BIT_SH_TEXLD_U_FIX`.
    pub texld_u_fix: bool,
    /// `gcFEATURE_BIT_TX_BORDER_CLAMP_FIX`.
    pub tx_border_clamp_fix: bool,
    /// `gcFEATURE_BIT_REG_Evis`.
    pub evis: bool,
    /// `gcFEATURE_BIT_FE_DRAW_DIRECT`.
    pub fe_draw_direct: bool,
    /// `gcFEATURE_BIT_MULTIVIEW_RENDER`.
    pub multiview_render: bool,
    /// `gcFEATURE_BIT_PE_ADVANCE_BLEND_PART0`.
    pub pe_advance_blend_part0: bool,
    /// `gcFEATURE_BIT_TX_UNNORMALIZED_COORD`.
    pub tx_unnormalized_coord: bool,

    /// `GL_ALIASED_LINE_WIDTH_RANGE`, min and max.
    pub line_width_range: [f32; 2],
}

/// The i.MX8MP's GC7000UL: NXP `gc7000ULN_v123` (:18839), the ids :18841-18845.
pub const GC7000UL_IMX8MP: ChipProfile = ChipProfile {
    name: c"Vivante GC7000UL (i.MX8MP)",
    model: 0x7000,                    // :18841
    revision: 0x6204,                 // :18842
    product: 0x70003,                 // :18843
    eco: 0,                           // :18844
    customer: 0x11,                   // :18845
    instructions: 512,                // :18852
    temp_registers: 64,               // :18849
    constants: 320,                   // :18853
    varyings: 16,                     // :18876
    streams: 16,                      // :18875
    pixel_pipes: 1,                   // :18866
    shader_cores: 2,                  // :18851
    threads: 512,                     // :18850
    non_power_of_two: true,           // :19036
    wideline_triangle_emu: false,     // :19297
    no_oneconst_limit: false,         // :19246
    no_index_const_on_a0: false,      // :19245
    movai_movar_dirty_data_fix: true, // :19645
    texld_u_fix: false,               // :19272
    tx_border_clamp_fix: false,       // :19291
    evis: false,                      // :19173
    fe_draw_direct: false,            // :19315
    multiview_render: false,          // :19314
    pe_advance_blend_part0: false,    // :19275
    tx_unnormalized_coord: false,     // :19317
    /* what the GL core reports (src/gl/state.rs GL_ALIASED_LINE_WIDTH_RANGE):
     * the wide-line registers are programmed (backend.rs GC_PA_CONFIG_WIDE_LINE) */
    line_width_range: [1.0, 16.0],
};

/// The STM32MP257F's GPU: ST `GCNANOULTRA31_VIP2` (:4720), the ids
/// :4722-4726; ST's own GL stack on the board names it "Vivante
/// GCNANOULTRA31_VIP2".
pub const GCNANOULTRA31_VIP2_ST: ChipProfile = ChipProfile {
    name: c"Vivante GCNANOULTRA31_VIP2 (STM32MP257F)",
    model: 0x8000,                     // :4722
    revision: 0x6205,                  // :4723
    product: 0x80003,                  // :4724
    eco: 0,                            // :4725
    customer: 0x15,                    // :4726, not read on the board (#1281)
    instructions: 512,                 // :4733
    temp_registers: 64,                // :4730
    constants: 320,                    // :4734
    varyings: 16,                      // :4757
    streams: 16,                       // :4756
    pixel_pipes: 1,                    // :4747
    shader_cores: 2,                   // :4732
    threads: 512,                      // :4731
    non_power_of_two: true,            // :4921
    wideline_triangle_emu: true,       // :5182
    no_oneconst_limit: true,           // :5131
    no_index_const_on_a0: true,        // :5130
    movai_movar_dirty_data_fix: false, // :5553
    texld_u_fix: true,                 // :5157
    tx_border_clamp_fix: true,         // :5176
    evis: true,                        // :5058
    fe_draw_direct: true,              // :5200
    multiview_render: true,            // :5199
    pe_advance_blend_part0: true,      // :5160
    tx_unnormalized_coord: true,       // :5202
    /* Mesa's range on an emulating core (etnaviv_screen.c:366-368), until a
     * board test says what the wide-line registers do on this part (Q11) */
    line_width_range: [1.0, 1.0],
};

/// Every part this backend knows.
pub const PROFILES: [&ChipProfile; 2] = [&GC7000UL_IMX8MP, &GCNANOULTRA31_VIP2_ST];

/// The part the backend is built for.
pub const ACTIVE: &ChipProfile = &GC7000UL_IMX8MP;

/// The profile of the part with these ids (`CHIP_MODEL`, `CHIP_REV`,
/// `CHIP_PRODUCT_ID`), or `None` for a part this backend does not know.
pub fn lookup(model: u32, revision: u32, product: u32) -> Option<&'static ChipProfile> {
    PROFILES.into_iter().find(|p| p.model == model && p.revision == revision && p.product == product)
}
