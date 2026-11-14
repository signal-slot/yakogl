// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64): PBESTATE_STATE_WORD1 of the three mixed integer and
//! float/unorm colour-target mixes, pinned against Mesa (plan #361, item
//! pbe-words-pin; docs/notes/pvr-int-mixed-targets.md sections 3 and 6).
//!
//! The end-of-tile test of the item pvr-accept compares the words the back end
//! emits for a mixed draw with the constants below, so they are fixed here
//! first and independently: each word is packed through the public csbgen
//! packer (`ROGUE_PBESTATE_STATE_WORD1_pack`, src/pvr/csbgen.rs) from the
//! field values of one attachment, and compared, masked with `& !0x3f` (the
//! low six bits are `address_high`, the render target's address, which these
//! words do not carry), with a literal worked out by hand from Mesa's field
//! definitions. The literals do not come from the back end or from csbgen's
//! own constants, so a field moved in either shows here.
//!
//! The mixes, with the register starts `mrt_layout::layout` gives (it is the
//! emit's walk, note section 3; the test asserts them first, so the words
//! rest on the layout the back end uses):
//!
//! - A: RGBA8 (U8U8U8U8) at register 0, R32UI (U32) at 1;
//! - B: RGBA8 at 0, RG32UI (U32U32) at 2 (register 1 is a hole);
//! - C: RGBA16F (F16F16F16F16) at 0, R32UI at 2, RGBA8 at 3.
//!
//! Layout of the word, Mesa 26.2.1 src/imagination/csbgen/rogue/pbestate.xml
//! `STATE_WORD1`, lines 230-243: `source_format` bit 19, `mrt_index` 16..18,
//! `source_pos` 14..15 (in 32-bit units: START_BIT0..96, lines 160-165),
//! `norm` 13, `packmode` 7..12, `emptytile` 6, `address_high` 0..5. The
//! pack modes are the `PACKMODE` enum values there: U8U8U8U8 0x0 (line 61),
//! F16F16F16F16 0xa (:71), U32U32 0x28 (:101), U32 0x34 (:113).
//!
//! The fields besides those three are the ones the back end sets in
//! `pbe_emit_words` (src/pvr/backend.rs:1032-1050): `source_format`
//! 8_PER_CHANNEL (1) for the unorm and integer targets and F16_PER_CHANNEL (0)
//! for the float one, `norm` set for the unorm RGBA8 only (Mesa
//! pvr_vk_format_is_fully_normalized, pvr_formats.h:264-273), `emptytile`
//! clear. The Mesa sources of the format table (vulkan/pvr_arch_formats.c) and
//! of the job code are not vendored under third_party/; the table rows are
//! cited by the lines backend.rs gives them (`pbe_int_packmode` :955-986,
//! `pbe_emit_words`).
//!
//! Hand-derived words (`source_format`, `norm`, `mrt_index`, `source_pos`,
//! `packmode` shifted into place, summed):
//!
//! | target | start | mrt | word & !0x3f |
//! |---|---|---|---|
//! | A0 RGBA8 | 0 | 0 | `0x0008_2000` |
//! | A1 R32UI | 1 | 1 | `0x0009_5A00` |
//! | B0 RGBA8 | 0 | 0 | `0x0008_2000` |
//! | B1 RG32UI | 2 | 1 | `0x0009_9400` |
//! | C0 RGBA16F | 0 | 0 | `0x0000_0500` |
//! | C1 R32UI | 2 | 1 | `0x0009_9A00` |
//! | C2 RGBA8 | 3 | 2 | `0x000A_E000` |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_pbe_mixed_words
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", target_os = "linux"))]

use yakogl::pvr::csbgen as csb;
use yakogl::pvr::mrt_layout::{layout, Place};

/// One attachment's end-of-tile state: its pack mode, whether its source is
/// the 8-bit-per-channel one and whether it is normalised, and the literal
/// word the field values must pack to.
struct Target {
    name: &'static str,
    /// Output-register dwords of the format (`surf_out_dwords`).
    dwords: u8,
    packmode: u32,
    /// The pack mode value in Mesa's `PACKMODE` enum, as a literal.
    packmode_mesa: u32,
    source_8: bool,
    norm: bool,
    /// Expected `source_pos` and `mrt_index`.
    start: u8,
    mrt: u32,
    word: u32,
}

fn pack(t: &Target) -> u32 {
    let v = csb::ROGUE_PBESTATE_STATE_WORD1 {
        source_format: if t.source_8 {
            csb::ROGUE_PBESTATE_SOURCE_FORMAT_8_PER_CHANNEL
        } else {
            csb::ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL
        },
        norm: t.norm,
        packmode: t.packmode,
        source_pos: t.start as u32,
        mrt_index: t.mrt,
        ..Default::default()
    };
    let mut dw = [0u32; 1];
    // SAFETY: the packer reads one initialised structure and writes its one dword.
    unsafe { csb::ROGUE_PBESTATE_STATE_WORD1_pack(dw.as_mut_ptr().cast(), &v) };
    dw[0] & !0x3f
}

fn rgba8(start: u8, mrt: u32, word: u32) -> Target {
    Target {
        name: "RGBA8",
        dwords: 1,
        packmode: csb::ROGUE_PBESTATE_PACKMODE_U8U8U8U8,
        /* pbestate.xml:61 */
        packmode_mesa: 0x0,
        source_8: true,
        norm: true,
        start,
        mrt,
        word,
    }
}

fn r32ui(start: u8, mrt: u32, word: u32) -> Target {
    Target {
        name: "R32UI",
        dwords: 1,
        packmode: csb::ROGUE_PBESTATE_PACKMODE_U32,
        /* pbestate.xml:113; R32_UINT -> U32, pvr_arch_formats.c:246 */
        packmode_mesa: 0x34,
        source_8: true,
        norm: false,
        start,
        mrt,
        word,
    }
}

fn rg32ui(start: u8, mrt: u32, word: u32) -> Target {
    Target {
        name: "RG32UI",
        dwords: 2,
        packmode: csb::ROGUE_PBESTATE_PACKMODE_U32U32,
        /* pbestate.xml:101; R32G32_UINT -> U32U32, pvr_arch_formats.c:249 */
        packmode_mesa: 0x28,
        source_8: true,
        norm: false,
        start,
        mrt,
        word,
    }
}

fn rgba16f(start: u8, mrt: u32, word: u32) -> Target {
    Target {
        name: "RGBA16F",
        dwords: 2,
        packmode: csb::ROGUE_PBESTATE_PACKMODE_F16F16F16F16,
        /* pbestate.xml:71; R16G16B16A16_SFLOAT, pvr_arch_formats.c:137 */
        packmode_mesa: 0xa,
        source_8: false,
        norm: false,
        start,
        mrt,
        word,
    }
}

/// Checks a mix: the layout gives the starts, each pack mode is Mesa's value,
/// and each packed word is the hand-derived literal.
fn check(mix: &[Target]) {
    let dwords: Vec<u8> = mix.iter().map(|t| t.dwords).collect();
    let l = layout(&dwords).expect("the mix has a layout");
    for (i, t) in mix.iter().enumerate() {
        assert_eq!(l.place[i], Place::Register { first: t.start }, "{} start", t.name);
        assert_eq!(t.packmode, t.packmode_mesa, "{} pack mode against Mesa's enum", t.name);
        let w = pack(t);
        assert_eq!(w, t.word, "{} (target {i}): {w:#010x}", t.name);
    }
}

#[test]
fn mix_a_rgba8_and_r32ui() {
    /* r0 RGBA8, r1 R32UI */
    check(&[
        /* 0<<7 | 1<<19 | 1<<13 */
        rgba8(0, 0, 0x0008_2000),
        /* 0x34<<7 (0x1a00) | 1<<14 | 1<<16 | 1<<19 */
        r32ui(1, 1, 0x0009_5A00),
    ]);
}

#[test]
fn mix_b_rgba8_and_rg32ui() {
    /* r0 RGBA8, r1 hole, r2-r3 RG32UI */
    check(&[
        rgba8(0, 0, 0x0008_2000),
        /* 0x28<<7 (0x1400) | 2<<14 (0x8000) | 1<<16 | 1<<19 */
        rg32ui(2, 1, 0x0009_9400),
    ]);
}

#[test]
fn mix_c_rgba16f_r32ui_and_rgba8() {
    /* r0-r1 halves, r2 R32UI, r3 RGBA8 */
    check(&[
        /* 0xa<<7 (0x500), source F16, norm clear */
        rgba16f(0, 0, 0x0000_0500),
        /* 0x1a00 | 2<<14 (0x8000) | 1<<16 | 1<<19 */
        r32ui(2, 1, 0x0009_9A00),
        /* 0 | 3<<14 (0xc000) | 2<<16 (0x20000) | 1<<19 | 1<<13 */
        rgba8(3, 2, 0x000A_E000),
    ]);
}
