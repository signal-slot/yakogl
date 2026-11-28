// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/vivante/early_z.rs` against etnaviv's early-depth rules (Mesa 26.2.1,
//! `src/gallium/drivers/etnaviv/`; the etnaviv driver is not under
//! `third_party/`, so the line numbers are those of the mesa-26.2.1 tree in the
//! farm's dependency cache, tag `mesa-26.2.1`, the version `third_party/mesa-26.2.1/UPSTREAM` names).
//!
//! Two parts:
//!
//! - one named case per condition under which etnaviv turns early depth, the
//!   early depth write or the PE depth stage off (shader discard, alpha test,
//!   shader depth output, stencil, a linear colour target, no RA_WRITE_DEPTH,
//!   NO_EARLY_Z, MSAA), each with the expected decision worked out by hand
//!   from the cited lines;
//! - a sweep of every combination of the rule's inputs against an independent
//!   transcription of `etna_update_zsa` and `etna_zsa_state_create`
//!   (below, `mesa_*`), comparing the decision, the PE_DEPTH_CONFIG bits the
//!   rule owns and the whole RA_EARLY_DEPTH word.
//!
//! Etnaviv has no depth-bounds test and no stencil-reference input to this rule
//! (grep of `etnaviv_state.c` and `etnaviv_zsa.c`: none), so there is no
//! such case. `early_z.rs` is not changed by this file.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante --test gc_early_z_rules
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::early_z::*;
use yakogl::vivante::regs::*;
use ColorLayout::*;

const EZ: u32 = GC_PE_DEPTH_EARLY_Z;
const NOZS: u32 = GC_PE_DEPTH_DISABLE_ZS;
const FWD: u32 = GC_RA_EARLY_DEPTH_BASE | GC_RA_EARLY_DEPTH_FORWARD_W | GC_RA_EARLY_DEPTH_FORWARD_Z;
const TEST: u32 = GC_RA_EARLY_DEPTH_TEST_ENABLE;
const WD: u32 = GC_RA_EARLY_DEPTH_WRITE_DISABLE;

/* ---- a transcription of etnaviv, independent of early_z.rs ---- */

/// One face of `pipe_stencil_state`: (enabled, func != ALWAYS, any op != KEEP).
type Face = (bool, bool, bool);

/// `etna_zsa_state_create`, etnaviv_zsa.c:48-49 and :66-85:
/// (z_test_enabled, z_write_enabled, stencil_enabled, stencil_modified).
fn mesa_zsa(depth_enabled: bool, func_always: bool, writemask: bool, s: [Face; 2]) -> (bool, bool, bool, bool) {
    let z_test = depth_enabled && !func_always; /* zsa.c:48 */
    let (mut enabled, mut modified) = (false, false);
    if s[0].0 {
        /* zsa.c:68 */
        if s[0].1 || (s[1].0 && s[1].1) {
            enabled = true; /* zsa.c:69-71 */
        }
        if s[0].2 {
            enabled = true; /* zsa.c:73-77 */
            modified = true;
        } else if s[1].0 && s[1].2 {
            enabled = true; /* zsa.c:78-84 */
            modified = true;
        }
    }
    (z_test, writemask, enabled, modified)
}

/// `etna_update_zsa`, etnaviv_state.c:993-1080: (early_z_test, early_zs, late_zs, late_z_test,
/// PE_DEPTH_CONFIG bits EARLY_Z|DISABLE_ZS, RA_EARLY_DEPTH).
fn mesa_update_zsa(i: &EarlyZInput) -> (bool, bool, bool, bool, u32, u32) {
    let face = |f: &StencilFace| -> Face { (f.enabled, !f.func_always, !f.ops_keep) };
    let (z_test, z_write, st_enabled, st_modified) =
        mesa_zsa(i.depth_enabled, i.depth_func_always, i.depth_write, [face(&i.stencil[0]), face(&i.stencil[1])]);
    let mut early_z_allowed = !i.no_early_z; /* state.c:1002 */
    /* state.c:1010-1023: the first bound colour buffer; LINEAR denies */
    if i.color == Linear {
        early_z_allowed = false;
    }
    let (mut late_zs, mut early_zs, mut late_z_test, mut early_z_test) = (false, false, false, false);
    if z_write || st_enabled {
        /* state.c:1025 */
        if i.ra_write_depth && early_z_allowed && !i.alpha_test && !i.writes_z && !i.uses_discard {
            early_zs = true; /* :1026-1031 */
        } else {
            late_zs = true; /* :1033 */
        }
    }
    if z_test {
        /* state.c:1036 */
        if early_z_allowed && (!st_modified || early_zs) && !i.writes_z {
            early_z_test = true; /* :1037-1040 */
        } else {
            late_z_test = true; /* :1042 */
        }
    }
    let mut pe = 0;
    if early_z_test {
        pe |= EZ; /* :1049 */
    }
    if !late_zs && !late_z_test {
        pe |= NOZS; /* :1050-1051 */
    }
    let mut ra = 0x30 | if early_z_test { TEST } else { 0 }; /* :1055-1056 */
    if i.ra_write_depth {
        /* :1058 */
        ra |= GC_RA_EARLY_DEPTH_FORWARD_W | GC_RA_EARLY_DEPTH_FORWARD_Z; /* :1059-1060 */
        if !early_zs {
            ra |= WD; /* :1062-1063 */
        }
        if i.color != Absent && (late_z_test || late_zs) && i.msaa {
            ra |= RA_EARLY_DEPTH_LATE_DEPTH_MSAA; /* :1065-1077 */
        }
    }
    (early_z_test, early_zs, late_zs, late_z_test, pe, ra)
}

/* ---- the named cases ---- */

fn base(color: ColorLayout) -> EarlyZInput {
    EarlyZInput::gc7000(color)
}
/// depth test + depth write (etnaviv_zsa.c:48-49: LESS, writemask on)
fn write(color: ColorLayout) -> EarlyZInput {
    EarlyZInput { depth_enabled: true, depth_func_always: false, depth_write: true, ..base(color) }
}
fn replace() -> [StencilFace; 2] {
    [StencilFace { enabled: true, func_always: true, ops_keep: false }, StencilFace::default()]
}
fn compare_only() -> [StencilFace; 2] {
    [StencilFace { enabled: true, func_always: false, ops_keep: true }, StencilFace::default()]
}

/// (early_test, early_write, late_test, late_write, PE bits, RA word)
type Want = (bool, bool, bool, bool, u32, u32);

fn check(name: &str, inp: EarlyZInput, want: Want) {
    let r = early_z(&inp);
    let got = (r.early_test, r.early_write, r.late_test, r.late_write, r.pe_bits, r.ra_word);
    assert_eq!(got, want, "{name}: (early_test, early_write, late_test, late_write, pe, ra) for {inp:?}");
    /* and etnaviv's own words for the same draw */
    let m = mesa_update_zsa(&inp);
    assert_eq!((m.0, m.1, m.3, m.2, m.4, m.5), want, "{name}: the transcription disagrees with the hand-worked row");
}

#[test]
fn shader_discard_makes_the_depth_write_late_and_keeps_the_test_early() {
    /* etnaviv_shader.c:286 uses_discard; state.c:1025-1033 (`!uses_discard` is an
     * early_zs condition only) and :1036-1040 (the test does not read it). */
    check(
        "discard",
        EarlyZInput { uses_discard: true, ..write(Tiled) },
        (true, false, false, true, EZ, FWD | TEST | WD),
    );
}

#[test]
fn alpha_test_with_a_depth_write_makes_the_write_late() {
    /* state.c:1028 `!zsa_state->alpha_enabled`; etnaviv_zsa.c:98 sets PE_ALPHA_OP_ALPHA_TEST from it */
    check(
        "alpha test + depth write",
        EarlyZInput { alpha_test: true, ..write(Tiled) },
        (true, false, false, true, EZ, FWD | TEST | WD),
    );
    /* no depth write and no stencil: the alpha test alone changes nothing (state.c:1025) */
    check(
        "alpha test, test only",
        EarlyZInput { alpha_test: true, depth_write: false, ..write(Tiled) },
        (true, false, false, false, EZ | NOZS, FWD | TEST | WD),
    );
}

#[test]
fn a_shader_that_writes_depth_makes_both_halves_late() {
    /* etnaviv_shader.c:285 writes_z = ps_depth_out_reg >= 0; state.c:1029 and :1039 */
    check("writes z", EarlyZInput { writes_z: true, ..write(Tiled) }, (false, false, true, true, 0, FWD | WD));
}

#[test]
fn a_linear_colour_target_denies_early_depth_but_no_target_does_not() {
    /* state.c:1010-1023 (first colour buffer, LINEAR denies); no colour buffer: the loop body never runs */
    check("linear", write(Linear), (false, false, true, true, 0, FWD | WD));
    check("tiled", write(Tiled), (true, true, false, false, EZ | NOZS, FWD | TEST));
    check("no colour (shadow's depth-only pass)", write(Absent), (true, true, false, false, EZ | NOZS, FWD | TEST));
    assert!(!early_z(&write(Linear)).allowed);
    assert!(early_z(&write(Tiled)).allowed && early_z(&write(Absent)).allowed);
}

#[test]
fn the_feature_bits_no_early_z_and_ra_write_depth() {
    /* state.c:1002 ETNA_FEATURE_NO_EARLY_Z: everything late */
    check("NO_EARLY_Z", EarlyZInput { no_early_z: true, ..write(Tiled) }, (false, false, true, true, 0, FWD | WD));
    /* state.c:1026, :1058-1077: without RA_WRITE_DEPTH the write is late, and no FORWARD / WRITE_DISABLE bits */
    check(
        "no RA_WRITE_DEPTH",
        EarlyZInput { ra_write_depth: false, ..write(Tiled) },
        (true, false, false, true, EZ, 0x30 | TEST),
    );
}

#[test]
fn stencil_compare_only_is_enabled_but_not_modified() {
    /* etnaviv_zsa.c:68-71: func != ALWAYS enables the stencil; ops all KEEP: not modified (:73) */
    let mut i = write(Tiled);
    i.depth_write = false;
    i.stencil = compare_only();
    check("stencil compare, depth test", i, (true, true, false, false, EZ | NOZS, FWD | TEST));
    /* a compare-only stencil with no depth test: an early stencil write alone (state.c:1025-1031) */
    i.depth_enabled = false;
    check("stencil compare, no depth test", i, (false, true, false, false, NOZS, FWD));
}

#[test]
fn stencil_ops_that_modify_tie_the_depth_test_to_the_early_write() {
    /* zsa.c:73-77 modified; state.c:1038 `(!stencil_modified || early_zs)` */
    let mut i = write(Tiled);
    i.depth_write = false;
    i.stencil = replace();
    check("stencil replace", i, (true, true, false, false, EZ | NOZS, FWD | TEST));
    /* ... and when the write is forced late (discard), the test follows it (:1038, :1042) */
    check(
        "stencil replace + discard",
        EarlyZInput { uses_discard: true, ..i },
        (false, false, true, true, 0, FWD | WD),
    );
    check("stencil replace + writes_z", EarlyZInput { writes_z: true, ..i }, (false, false, true, true, 0, FWD | WD));
    check(
        "stencil replace, linear",
        EarlyZInput { stencil: replace(), ..write(Linear) },
        (false, false, true, true, 0, FWD | WD),
    );
}

#[test]
fn a_back_face_stencil_op_counts_only_with_the_front_face_enabled() {
    /* zsa.c:68 gates the whole block on stencil[0].enabled; :78-84 reads stencil[1] when the front ops are KEEP */
    let back_only = [StencilFace::default(), StencilFace { enabled: true, func_always: false, ops_keep: false }];
    let mut i = write(Tiled);
    i.depth_write = false;
    i.stencil = back_only;
    check("back face only", i, (true, false, false, false, EZ | NOZS, FWD | TEST | WD));
    /* front enabled with KEEP and ALWAYS, back modifies: modified through :78-84 */
    let both = [StencilFace { enabled: true, func_always: true, ops_keep: true }, back_only[1]];
    i.stencil = both;
    check("front keep, back replace", i, (true, true, false, false, EZ | NOZS, FWD | TEST));
    /* the back face's func != ALWAYS enables the stencil too (:69-71) */
    let back_cmp = [both[0], StencilFace { enabled: true, func_always: false, ops_keep: true }];
    i.stencil = back_cmp;
    check("front keep, back compare", i, (true, true, false, false, EZ | NOZS, FWD | TEST));
}

#[test]
fn depth_func_always_is_no_depth_test() {
    /* etnaviv_zsa.c:48: z_test_enabled = depth_enabled && depth_func != ALWAYS */
    check(
        "ALWAYS + write",
        EarlyZInput { depth_enabled: true, depth_func_always: true, depth_write: true, ..base(Tiled) },
        (false, true, false, false, NOZS, FWD),
    );
    check("depth off", base(Tiled), (false, false, false, false, NOZS, FWD | WD));
    check(
        "depth off, writemask on",
        EarlyZInput { depth_write: true, ..base(Tiled) },
        (false, true, false, false, NOZS, FWD),
    );
}

#[test]
fn a_late_stage_on_a_multisampled_colour_target_sets_late_depth_msaa() {
    /* state.c:1068-1077: first colour buffer with nr_samples > 1 and (late_z_test || late_zs) */
    let late = |c| EarlyZInput { msaa: true, ..write(c) };
    assert_ne!(early_z(&late(Linear)).ra_word & RA_EARLY_DEPTH_LATE_DEPTH_MSAA, 0);
    assert_eq!(early_z(&late(Tiled)).ra_word & RA_EARLY_DEPTH_LATE_DEPTH_MSAA, 0);
    assert_ne!(early_z(&EarlyZInput { uses_discard: true, ..late(Tiled) }).ra_word & RA_EARLY_DEPTH_LATE_DEPTH_MSAA, 0);
    /* no RA_WRITE_DEPTH: the whole block (:1058) is skipped */
    assert_eq!(
        early_z(&EarlyZInput { ra_write_depth: false, ..late(Linear) }).ra_word & RA_EARLY_DEPTH_LATE_DEPTH_MSAA,
        0
    );
}

/* ---- the sweep ---- */

#[test]
fn every_combination_of_inputs_gets_etnaviv_decision() {
    let faces: Vec<StencilFace> =
        (0..8u32).map(|b| StencilFace { enabled: b & 1 != 0, func_always: b & 2 != 0, ops_keep: b & 4 != 0 }).collect();
    let mut n = 0u32;
    for color in [Absent, Linear, Tiled] {
        for bits in 0u32..(1 << 8) {
            /* `msaa` is a property of the first colour target (state.c:1068-1077); with none it cannot be set */
            if color == Absent && bits & 64 != 0 {
                continue;
            }
            for f0 in &faces {
                for f1 in &faces {
                    let inp = EarlyZInput {
                        depth_enabled: bits & 1 != 0,
                        depth_func_always: bits & 2 != 0,
                        depth_write: bits & 4 != 0,
                        alpha_test: bits & 8 != 0,
                        stencil: [*f0, *f1],
                        writes_z: bits & 16 != 0,
                        uses_discard: bits & 32 != 0,
                        color,
                        msaa: bits & 64 != 0,
                        no_early_z: bits & 128 != 0,
                        ra_write_depth: true,
                    };
                    for ra_write_depth in [true, false] {
                        let inp = EarlyZInput { ra_write_depth, ..inp };
                        let r = early_z(&inp);
                        let m = mesa_update_zsa(&inp);
                        let got = (r.early_test, r.early_write, r.late_write, r.late_test, r.pe_bits, r.ra_word);
                        assert_eq!(got, (m.0, m.1, m.2, m.3, m.4, m.5), "{inp:?}");
                        n += 1;
                    }
                }
            }
        }
    }
    assert_eq!(n, (2 * 256 + 128) * 64 * 2);
}
