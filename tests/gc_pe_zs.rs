// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/vivante/pe_zs.rs`: when the PE may skip depth and stencil
//! (`DISABLE_ZS`), against Mesa 26.2.1 etnaviv and against today's inline
//! code in backend.rs `emit_pe_state` for every target with a depth or
//! stencil attachment.

#[path = "../src/vivante/pe_zs.rs"]
mod pe_zs;

use pe_zs::*;

const NO_ZS: ZsOutput = ZsOutput { disable_zs: true, depth_func: ZsFunc::Always, depth_write: false };

#[test]
fn terrain_colour_only_clears_disable_zs() {
    /* noise, normal, bloom_h, bloom_v and tilt_h all clear colour into a
     * target with no depth or stencil attachment: one input. Today they get
     * 0x00040711 (no DISABLE_ZS); Mesa's draws there 0x01000700. */
    let clear = ZsInput { is_clear: true, ..Default::default() };
    assert_eq!(pe_zs(clear), NO_ZS);
    /* ...and the GL enables left on from an earlier pass change nothing */
    let left_on = ZsInput { depth_test: true, depth_write: true, stencil_test: true, ..clear };
    assert_eq!(pe_zs(left_on), NO_ZS);
}

#[test]
fn depth_clear_writes_with_always() {
    let z = ZsInput { is_clear: true, clear_depth: true, has_depth_attachment: true, ..Default::default() };
    assert_eq!(pe_zs(z), ZsOutput { disable_zs: false, depth_func: ZsFunc::Always, depth_write: true });
    /* the GL's own depth state does not matter to a clear */
    let off = ZsInput { depth_test: false, depth_write: false, ..z };
    assert_eq!(pe_zs(off), pe_zs(z));
}

#[test]
fn stencil_clear_keeps_the_zs_stage_and_no_depth_write() {
    let z = ZsInput {
        is_clear: true,
        clear_stencil: true,
        has_depth_attachment: true,
        has_stencil_attachment: true,
        ..Default::default()
    };
    assert_eq!(pe_zs(z), ZsOutput { disable_zs: false, depth_func: ZsFunc::Always, depth_write: false });
}

#[test]
fn colour_clear_with_a_depth_attachment_is_unchanged() {
    /* today's 0x00040711: ALWAYS, no write, no DISABLE_ZS */
    let z = ZsInput { is_clear: true, has_depth_attachment: true, has_stencil_attachment: true, ..Default::default() };
    assert_eq!(pe_zs(z), ZsOutput { disable_zs: false, depth_func: ZsFunc::Always, depth_write: false });
}

#[test]
fn draws() {
    let d = ZsInput { has_depth_attachment: true, ..Default::default() };
    /* depth test on: the GL's function, written as glDepthMask says */
    let t = ZsInput { depth_test: true, depth_write: true, ..d };
    assert_eq!(pe_zs(t), ZsOutput { disable_zs: false, depth_func: ZsFunc::Gl, depth_write: true });
    let t = ZsInput { depth_write: false, ..t };
    assert_eq!(pe_zs(t), ZsOutput { disable_zs: false, depth_func: ZsFunc::Gl, depth_write: false });
    /* depth test off, glDepthMask true: not written (GL ES 2.0 4.1.5) and
     * nothing for the stage to do */
    assert_eq!(pe_zs(ZsInput { depth_write: true, ..d }), NO_ZS);
    /* the stencil test alone keeps the stage */
    let s = ZsInput { stencil_test: true, has_stencil_attachment: true, ..d };
    assert_eq!(pe_zs(s), ZsOutput { disable_zs: false, depth_func: ZsFunc::Always, depth_write: false });
    /* no attachment: the tests pass and nothing is written whatever the
     * enables (st_atom_depth.c:86, :100) */
    let none = ZsInput { depth_test: true, depth_write: true, stencil_test: true, ..Default::default() };
    assert_eq!(pe_zs(none), NO_ZS);
}

/// backend.rs `emit_pe_state` as of this commit, transcribed: `depth_test`,
/// `depth_write`, `depth_func` and the DISABLE_ZS condition.
fn today(z: ZsInput) -> ZsOutput {
    let depth_test = if z.is_clear { true } else { z.depth_test };
    let depth_write = if z.is_clear { z.clear_depth } else { z.depth_test && z.depth_write };
    let depth_func = if z.is_clear { ZsFunc::Always } else { ZsFunc::Gl };
    let disable_zs = !depth_test && !depth_write && !z.stencil_test && !z.is_clear;
    ZsOutput { disable_zs, depth_func: if depth_test { depth_func } else { ZsFunc::Always }, depth_write }
}

#[test]
fn every_case_with_an_attachment_is_todays() {
    let mut with = 0;
    for bits in 0u32..256 {
        let b = |i: u32| bits >> i & 1 != 0;
        let z = ZsInput {
            is_clear: b(0),
            clear_depth: b(1),
            clear_stencil: b(2),
            depth_test: b(3),
            depth_write: b(4),
            stencil_test: b(5),
            has_depth_attachment: b(6),
            has_stencil_attachment: b(7),
        };
        if z.has_depth_attachment || z.has_stencil_attachment {
            assert_eq!(pe_zs(z), today(z), "{z:?}");
            with += 1;
        } else {
            assert_eq!(pe_zs(z), NO_ZS, "{z:?}");
        }
    }
    assert_eq!(with, 192);
}
