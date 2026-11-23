// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The back end's limits come from `profile::ACTIVE`: the helpers it routes
//! through return each profile's values, and with ACTIVE = GC7000UL the
//! reported GL limits are today's (the i.MX8MP behaviour does not change).
//!
//! Features: `cargo test --no-default-features --features
//! host-test,gpu-vivante,shaderc-online --test vivante_st_profile` (the set
//! the gate gives tests/vivante_profile.rs).

#![cfg(feature = "gpu-vivante")]

use yakogl::vivante::backend::{line_width_range_of, LINE_WIDTH_RANGE, NON_POWER_OF_TWO};
use yakogl::vivante::profile::{self, GC7000UL_IMX8MP, GCNANOULTRA31_VIP2_ST, PROFILES};
use yakogl::vivante::{regs, state};

#[test]
fn each_profile_gives_its_own_line_width_range() {
    for p in PROFILES {
        assert_eq!(line_width_range_of(p), p.line_width_range, "{:?}", p.name);
    }
    assert_eq!(line_width_range_of(&GC7000UL_IMX8MP), [1.0, 16.0]);
    assert_eq!(line_width_range_of(&GCNANOULTRA31_VIP2_ST), [1.0, 1.0]);
}

#[test]
fn with_the_imx8mp_active_the_limits_are_todays() {
    assert_eq!(profile::ACTIVE, &GC7000UL_IMX8MP);
    assert_eq!(LINE_WIDTH_RANGE, [1.0, 16.0]);
    assert!(NON_POWER_OF_TWO);
    assert_eq!(regs::GC_NUM_CONSTANTS, profile::ACTIVE.constants);
    assert_eq!(state::GC_MAX_VARYINGS, profile::ACTIVE.varyings as usize);
    assert_eq!(state::GC_MAX_VERTEX_ELEMENTS, profile::ACTIVE.streams as usize);
    assert_eq!(state::GC_MAX_UNIFORM_DWORDS, profile::ACTIVE.constants as usize * 4);
}
