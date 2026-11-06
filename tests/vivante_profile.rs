// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Vivante chip profiles (`src/vivante/profile.rs`): the ST part's
//! values, and the i.MX8MP's held to the constants the backend and the GL
//! core use today, so neither can drift from the other.
//!
//! The ST values are Mesa 26.2.1's `src/etnaviv/hwdb/st/gc_feature_database.h`,
//! the `GCNANOULTRA31_VIP2` entry at :4720, the ids the board read under
//! ST's Linux (model 0x8000, revision 0x6205, product 0x80003, eco 0; issue
//! #1281). Section 3 of docs/notes/pre-study-stm32mp257f-dk.md compares the
//! two rows.

#![cfg(feature = "gpu-vivante")]

use yakogl::vivante::profile::{self, ChipProfile, GC7000UL_IMX8MP, GCNANOULTRA31_VIP2_ST};
use yakogl::vivante::{backend, regs, state};

#[test]
fn the_st_profile_is_the_gcnanoultra31_vip2_row() {
    let p = &GCNANOULTRA31_VIP2_ST;
    assert_eq!(p.name, c"Vivante GCNANOULTRA31_VIP2 (STM32MP257F)");
    assert_eq!((p.model, p.revision, p.product, p.eco, p.customer), (0x8000, 0x6205, 0x80003, 0, 0x15));
    assert_eq!((p.instructions, p.temp_registers, p.constants, p.varyings, p.streams), (512, 64, 320, 16, 16));
    assert_eq!((p.pixel_pipes, p.shader_cores, p.threads), (1, 2, 512));
    assert!(p.non_power_of_two);
    assert!(p.wideline_triangle_emu, "st :5182");
    assert!(p.no_oneconst_limit, "st :5131");
    assert!(p.no_index_const_on_a0, "st :5130");
    assert!(!p.movai_movar_dirty_data_fix, "st :5553");
    assert!(p.texld_u_fix && p.tx_border_clamp_fix && p.evis && p.fe_draw_direct);
    assert!(p.multiview_render && p.pe_advance_blend_part0 && p.tx_unnormalized_coord);
    /* an emulating core reports 1.0 at most (etnaviv_screen.c:366-368) */
    assert_eq!(p.line_width_range, [1.0, 1.0]);
}

#[test]
fn the_imx8mp_profile_is_what_the_backend_uses_today() {
    let p = &GC7000UL_IMX8MP;
    assert_eq!(profile::ACTIVE, p, "the backend is built for the i.MX8MP");
    assert_eq!((p.model, p.revision, p.product, p.eco, p.customer), (0x7000, 0x6204, 0x70003, 0, 0x11));
    assert_eq!(p.name, c"Vivante GC7000UL (i.MX8MP)");
    // SAFETY: the backend's name is a static C string.
    let name = unsafe { core::ffi::CStr::from_ptr(backend::yakogl_backend_vivante.name) };
    assert_eq!(name, p.name);
    assert_eq!(p.constants, regs::GC_NUM_CONSTANTS);
    assert_eq!(p.constants as usize * 4, state::GC_MAX_UNIFORM_DWORDS);
    assert_eq!(p.varyings as usize, state::GC_MAX_VARYINGS);
    assert_eq!(p.streams as usize, state::GC_MAX_VERTEX_ELEMENTS);
    /* NUM_TEMPS is six bits: the largest count the register takes */
    assert_eq!(p.temp_registers - 1, yakogl::vivante::shader::GC_MAX_TEMPS);
    assert_eq!(p.non_power_of_two, backend::NON_POWER_OF_TWO);
    assert!(!p.wideline_triangle_emu && !p.no_oneconst_limit);
    assert!(!p.no_index_const_on_a0 && p.movai_movar_dirty_data_fix);
    assert!(!p.texld_u_fix && !p.tx_border_clamp_fix && !p.evis && !p.fe_draw_direct);
    assert!(!p.multiview_render && !p.pe_advance_blend_part0 && !p.tx_unnormalized_coord);
    /* src/gl/state.rs GL_ALIASED_LINE_WIDTH_RANGE; tests/es2_report_guard.rs
     * and tests/es3_caps.rs pin the GL core's answer to the same pair */
    assert_eq!(p.line_width_range, [1.0, 16.0]);
}

#[test]
fn the_lookup_finds_each_part_by_its_ids() {
    let find = |p: &ChipProfile| profile::lookup(p.model, p.revision, p.product);
    assert_eq!(find(&GC7000UL_IMX8MP), Some(&GC7000UL_IMX8MP));
    assert_eq!(find(&GCNANOULTRA31_VIP2_ST), Some(&GCNANOULTRA31_VIP2_ST));
    /* ST's 0x6207 row and the i.MX8MP's older revisions are not profiles */
    assert_eq!(profile::lookup(0x8000, 0x6207, 0xa080007), None);
    assert_eq!(profile::lookup(0x7000, 0x6202, 0x70003), None);
    assert_eq!(profile::lookup(0x7000, 0x6204, 0x80003), None);
}
