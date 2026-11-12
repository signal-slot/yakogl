// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The early-depth rule as a pure function (docs/notes/gc-tiled-targets.md,
//! "(c) Early depth"). `emit_pe_state` and `emit_ra_state` call
//! it through `draw_early_z` (a linear target, plus the program's `writes_z`),
//! and so still say "never".
//!
//! The rule is Mesa 26.2.1's `etna_update_zsa`
//! (`src/gallium/drivers/etnaviv/etnaviv_state.c:992-1080`), with the inputs
//! it reads:
//!
//! - `z_test_enabled`, `z_write_enabled`, `stencil_enabled`,
//!   `stencil_modified`: `etna_zsa_state_create`, `etnaviv_zsa.c:47-81`;
//! - `writes_z`, `uses_discard`: `compiled_shader_state`,
//!   `etnaviv_shader.c:285-286`;
//! - `alpha_enabled`: the pipe alpha test (GLES has none; a GL-core caller
//!   passes `false`, but Mesa's rule reads it);
//! - the first colour target's layout: `etnaviv_state.c:1005-1023`;
//! - `RA_EARLY_DEPTH` reaches the stream in `etnaviv_emit.c:463`.
//!
//! Mesa's rule has no SINGLE_BUFFER or draw-mode input: early depth reads the
//! colour layout alone (linear or not). A clear is an RS operation in Mesa and
//! never reaches this function; the back end's clear quad (a draw) must keep
//! its own answer when this is wired in.
#![allow(dead_code)]

use super::pe_zs::{pe_zs, ZsFunc, ZsInput};
use super::regs::*;
use crate::backend::{GlesDraw, GlesStencilOp, GLES_CMP_ALWAYS, GLES_SOP_DECR_WRAP, GLES_SOP_KEEP};

/// `VIVS_RA_EARLY_DEPTH_LATE_DEPTH_MSAA` (`hw/state_3d.xml.h:592`).
pub const RA_EARLY_DEPTH_LATE_DEPTH_MSAA: u32 = 0x0200_0000;

/// The colour render target as the rule sees it (`etnaviv_state.c:1005-1023`:
/// the first bound colour buffer decides, and only `ETNA_LAYOUT_LINEAR` denies).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorLayout {
    /// No colour buffer bound (shadow's depth-only pass): early depth stays allowed.
    Absent,
    Linear,
    /// Tiled, super-tiled or multi-tiled.
    Tiled,
}

/// One face's stencil state (`pipe_stencil_state`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StencilFace {
    pub enabled: bool,
    /// `func == PIPE_FUNC_ALWAYS`
    pub func_always: bool,
    /// fail, zfail and zpass are all KEEP
    pub ops_keep: bool,
}

/// What `etna_update_zsa` reads of one draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EarlyZInput {
    /// `so->depth_enabled`
    pub depth_enabled: bool,
    /// `so->depth_func == PIPE_FUNC_ALWAYS`
    pub depth_func_always: bool,
    /// `so->depth_writemask`
    pub depth_write: bool,
    /// `so->alpha_enabled`
    pub alpha_test: bool,
    pub stencil: [StencilFace; 2],
    /// the fragment shader writes depth (`ps_depth_out_reg >= 0`)
    pub writes_z: bool,
    pub uses_discard: bool,
    pub color: ColorLayout,
    /// the first colour target has more than one sample
    pub msaa: bool,
    /// `ETNA_FEATURE_NO_EARLY_Z` (or `ETNA_DBG_NO_EARLY_Z`, `etnaviv_screen.c:1084-1085`)
    pub no_early_z: bool,
    /// `ETNA_FEATURE_RA_WRITE_DEPTH` (the GC7000UL has it)
    pub ra_write_depth: bool,
}

impl EarlyZInput {
    /// A GC7000UL draw with depth, stencil, alpha test and discard all off.
    pub const fn gc7000(color: ColorLayout) -> Self {
        let f = StencilFace { enabled: false, func_always: true, ops_keep: true };
        EarlyZInput {
            depth_enabled: false,
            depth_func_always: true,
            depth_write: false,
            alpha_test: false,
            stencil: [f, f],
            writes_z: false,
            uses_discard: false,
            color,
            msaa: false,
            no_early_z: false,
            ra_write_depth: true,
        }
    }
}

/// The decision and the bits Mesa derives from it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EarlyZ {
    /// `early_z_allowed` (`etnaviv_state.c:1001-1023`)
    pub allowed: bool,
    /// `early_zs`: the depth/stencil write happens early (`:1025-1034`)
    pub early_write: bool,
    /// `early_z_test` (`:1036-1043`)
    pub early_test: bool,
    pub late_write: bool,
    pub late_test: bool,
    /// the `PE_DEPTH_CONFIG` bits this rule owns: `EARLY_Z`, `DISABLE_ZS` (`:1045-1051`)
    pub pe_bits: u32,
    /// the whole `RA_EARLY_DEPTH` word (`:1055-1074`)
    pub ra_word: u32,
}

/// `etnaviv_zsa.c:63-81`: (`stencil_enabled`, `stencil_modified`).
fn stencil_flags(s: &[StencilFace; 2]) -> (bool, bool) {
    let mut enabled = false;
    let mut modified = false;
    if s[0].enabled {
        if !s[0].func_always || (s[1].enabled && !s[1].func_always) {
            enabled = true;
        }
        if !s[0].ops_keep || (s[1].enabled && !s[1].ops_keep) {
            enabled = true;
            modified = true;
        }
    }
    (enabled, modified)
}

/// Mesa's early-depth decision for one draw (`etna_update_zsa`).
pub fn early_z(i: &EarlyZInput) -> EarlyZ {
    let (stencil_enabled, stencil_modified) = stencil_flags(&i.stencil);
    let z_test = i.depth_enabled && !i.depth_func_always; /* etnaviv_zsa.c:49 */

    /* etnaviv_state.c:1001, :1005-1023 */
    let allowed = !i.no_early_z && i.color != ColorLayout::Linear;

    /* :1025-1034 */
    let (mut early_zs, mut late_zs) = (false, false);
    if i.depth_write || stencil_enabled {
        if i.ra_write_depth && allowed && !i.alpha_test && !i.writes_z && !i.uses_discard {
            early_zs = true;
        } else {
            late_zs = true;
        }
    }
    /* :1036-1043 */
    let (mut early_test, mut late_test) = (false, false);
    if z_test {
        if allowed && (!stencil_modified || early_zs) && !i.writes_z {
            early_test = true;
        } else {
            late_test = true;
        }
    }

    /* :1045-1074 */
    let mut pe_bits = 0;
    if early_test {
        pe_bits |= GC_PE_DEPTH_EARLY_Z;
    }
    if !late_zs && !late_test {
        pe_bits |= GC_PE_DEPTH_DISABLE_ZS;
    }
    let mut ra_word = GC_RA_EARLY_DEPTH_BASE;
    if early_test {
        ra_word |= GC_RA_EARLY_DEPTH_TEST_ENABLE;
    }
    if i.ra_write_depth {
        ra_word |= GC_RA_EARLY_DEPTH_FORWARD_W | GC_RA_EARLY_DEPTH_FORWARD_Z;
        if !early_zs {
            ra_word |= GC_RA_EARLY_DEPTH_WRITE_DISABLE;
        }
        if (late_test || late_zs) && i.msaa {
            ra_word |= RA_EARLY_DEPTH_LATE_DEPTH_MSAA;
        }
    }
    EarlyZ { allowed, early_write: early_zs, early_test, late_write: late_zs, late_test, pe_bits, ra_word }
}

/// The rule for a draw of this back end: every colour surface it renders to is
/// linear, which denies early depth outright, and a program that writes depth
/// denies it by itself (`etnaviv_state.c:1029`, `:1039`). `emit_ra_state` and
/// `emit_pe_state` take their words from here, so a tiled target only has to
/// change the layout.
pub fn draw_early_z(writes_z: bool) -> EarlyZ {
    early_z(&EarlyZInput { writes_z, ..EarlyZInput::gc7000(ColorLayout::Linear) })
}

/// The early-depth decision's inputs for one draw of the back end, read from
/// the `GlesDraw` as `emit_pe_state` and `pe_zs` read it (a pure function;
/// `emit_ra_state` does not call it yet).
///
/// - depth: `pe_zs` with the draw's attachments: the test is on only with a
///   depth or stencil attachment, `depth_func_always` is the GL function, the
///   write is `pe_zs`'s (the test on and `glDepthMask`);
/// - stencil: the GL test on with an attachment; per face the function
///   ALWAYS and all three operations KEEP (`stencil_op` maps an unknown
///   operation to KEEP too, and so does this);
/// - `alpha_test`: GLES has none, always false; `msaa`: more than one sample;
/// - colour: `Tiled` for a shadow target, `Linear` otherwise.
///
/// Policy over Mesa's rule, for the first wiring: a draw whose program
/// discards or whose stencil operations modify the buffer keeps today's
/// words (the colour is reported `Linear`, which denies early depth), though
/// `etna_update_zsa` would still test early there. A clear quad is a draw in
/// this back end, not an RS clear, and is denied the same way.
///
/// `writes_z` and `uses_discard` are the program's (`shader::discards`).
pub fn input_of(gl: &GlesDraw, shadow: bool, writes_z: bool, uses_discard: bool, is_clear: bool) -> EarlyZInput {
    let mut i =
        EarlyZInput { writes_z, uses_discard, msaa: gl.samples > 1, ..EarlyZInput::gc7000(ColorLayout::Linear) };
    if is_clear {
        return i;
    }
    let has_depth = gl.depth_id != 0;
    let has_stencil = gl.stencil_id != 0;
    let zs = pe_zs(ZsInput {
        depth_test: gl.depth_test,
        depth_write: gl.depth_write,
        stencil_test: gl.stencil,
        has_depth_attachment: has_depth,
        has_stencil_attachment: has_stencil,
        ..ZsInput::default()
    });
    i.depth_enabled = gl.depth_test && (has_depth || has_stencil);
    i.depth_func_always = zs.depth_func == ZsFunc::Always || gl.depth_func == GLES_CMP_ALWAYS;
    i.depth_write = zs.depth_write;
    if gl.stencil && (has_depth || has_stencil) {
        let keep = |op: GlesStencilOp| op > GLES_SOP_DECR_WRAP || op == GLES_SOP_KEEP;
        let face = |func, sfail, dpfail, dppass| StencilFace {
            enabled: true,
            func_always: func == GLES_CMP_ALWAYS,
            ops_keep: keep(sfail) && keep(dpfail) && keep(dppass),
        };
        i.stencil = [
            face(gl.stencil_func, gl.stencil_sfail, gl.stencil_dpfail, gl.stencil_dppass),
            face(gl.stencil_back_func, gl.stencil_back_sfail, gl.stencil_back_dpfail, gl.stencil_back_dppass),
        ];
    }
    let modifies = i.stencil.iter().any(|f| f.enabled && !f.ops_keep);
    if shadow && !uses_discard && !modifies {
        i.color = ColorLayout::Tiled;
    }
    i
}

/// A small stable key of a decision for `pe_key`: bit 0 early test, 1 early
/// write, 2 late test, 3 late write, 4 `DISABLE_ZS`, 5 the MSAA late-depth
/// flag. Equal keys emit equal `PE_DEPTH_CONFIG` and `RA_EARLY_DEPTH` bits
/// for one GPU (`ra_write_depth` is a property of the chip and not in the key).
pub fn decision_bits(e: &EarlyZ) -> u32 {
    (e.early_test as u32)
        | (e.early_write as u32) << 1
        | (e.late_test as u32) << 2
        | (e.late_write as u32) << 3
        | ((e.pe_bits & GC_PE_DEPTH_DISABLE_ZS != 0) as u32) << 4
        | ((e.ra_word & RA_EARLY_DEPTH_LATE_DEPTH_MSAA != 0) as u32) << 5
}

#[cfg(test)]
mod tests {
    use super::*;
    use ColorLayout::*;

    const EZ: u32 = GC_PE_DEPTH_EARLY_Z;
    const NOZS: u32 = GC_PE_DEPTH_DISABLE_ZS;
    const FWD: u32 = GC_RA_EARLY_DEPTH_BASE | GC_RA_EARLY_DEPTH_FORWARD_W | GC_RA_EARLY_DEPTH_FORWARD_Z;
    const TEST: u32 = GC_RA_EARLY_DEPTH_TEST_ENABLE;
    const WD: u32 = GC_RA_EARLY_DEPTH_WRITE_DISABLE;

    fn base(color: ColorLayout) -> EarlyZInput {
        EarlyZInput::gc7000(color)
    }
    fn test_only(color: ColorLayout) -> EarlyZInput {
        EarlyZInput { depth_enabled: true, depth_func_always: false, ..base(color) }
    }
    fn write(color: ColorLayout) -> EarlyZInput {
        EarlyZInput { depth_write: true, ..test_only(color) }
    }
    fn stencil_ops_replace() -> [StencilFace; 2] {
        let f = StencilFace { enabled: true, func_always: true, ops_keep: false };
        [f, StencilFace::default()]
    }
    fn stencil_func_only() -> [StencilFace; 2] {
        let f = StencilFace { enabled: true, func_always: false, ops_keep: true };
        [f, StencilFace::default()]
    }

    /* (name, input, early_test, early_write, PE bits, RA word); each row
     * worked through etna_update_zsa by hand */
    #[test]
    fn the_decision_table_follows_etna_update_zsa() {
        let t: Vec<(&str, EarlyZInput, bool, bool, u32, u32)> = vec![
            /* terrain's draw: depth test and write, no discard. Linear: denied
             * (:1018-1019), a late test and a late write: no bits but the base. */
            ("terrain linear", write(Linear), false, false, 0, FWD | WD),
            /* the same draw on a tiled target: early test and early write,
             * nothing late, so DISABLE_ZS too. */
            ("terrain tiled", write(Tiled), true, true, EZ | NOZS, FWD | TEST),
            ("terrain, no colour target", write(Absent), true, true, EZ | NOZS, FWD | TEST),
            /* depth test, no write: early test, the write-disable stays */
            ("test only tiled", test_only(Tiled), true, false, EZ | NOZS, FWD | TEST | WD),
            ("test only linear", test_only(Linear), false, false, 0, FWD | WD),
            /* nothing enabled: DISABLE_ZS, no early bits */
            ("off linear", base(Linear), false, false, NOZS, FWD | WD),
            ("off tiled", base(Tiled), false, false, NOZS, FWD | WD),
            /* depth func ALWAYS with depth enabled is no test (etnaviv_zsa.c:49) */
            (
                "always+write tiled",
                EarlyZInput { depth_enabled: true, depth_write: true, ..base(Tiled) },
                false,
                true,
                NOZS,
                FWD,
            ),
            /* discard: the test stays early, the write goes late (:1028-1030) */
            ("discard tiled", EarlyZInput { uses_discard: true, ..write(Tiled) }, true, false, EZ, FWD | TEST | WD),
            ("alpha test tiled", EarlyZInput { alpha_test: true, ..write(Tiled) }, true, false, EZ, FWD | TEST | WD),
            /* depth written by the shader: both late (:1029, :1039) */
            ("writes z tiled", EarlyZInput { writes_z: true, ..write(Tiled) }, false, false, 0, FWD | WD),
            /* no RA_WRITE_DEPTH: the write is late, no FORWARD bits, no WRITE_DISABLE */
            (
                "no ra_write_depth tiled",
                EarlyZInput { ra_write_depth: false, ..write(Tiled) },
                true,
                false,
                EZ,
                GC_RA_EARLY_DEPTH_BASE | TEST,
            ),
            /* ETNA_DBG_NO_EARLY_Z */
            ("no_early_z tiled", EarlyZInput { no_early_z: true, ..write(Tiled) }, false, false, 0, FWD | WD),
            /* stencil that only compares: enabled, not modified (zsa.c:67-71) */
            (
                "stencil func tiled",
                EarlyZInput { stencil: stencil_func_only(), ..test_only(Tiled) },
                true,
                true,
                EZ | NOZS,
                FWD | TEST,
            ),
            /* stencil ops modify: the write is early, so the test is too */
            (
                "stencil replace tiled",
                EarlyZInput { stencil: stencil_ops_replace(), ..test_only(Tiled) },
                true,
                true,
                EZ | NOZS,
                FWD | TEST,
            ),
            /* ... but with a discard the stencil write is late and the test follows it (:1038) */
            (
                "stencil replace + discard tiled",
                EarlyZInput { stencil: stencil_ops_replace(), uses_discard: true, ..test_only(Tiled) },
                false,
                false,
                0,
                FWD | WD,
            ),
            /* stencil modified, depth test off: an early stencil write alone */
            (
                "stencil replace, no depth test, tiled",
                EarlyZInput { stencil: stencil_ops_replace(), ..base(Tiled) },
                false,
                true,
                NOZS,
                FWD,
            ),
            (
                "stencil replace linear",
                EarlyZInput { stencil: stencil_ops_replace(), ..test_only(Linear) },
                false,
                false,
                0,
                FWD | WD,
            ),
        ];
        for (name, inp, early_test, early_write, pe, ra) in t {
            let r = early_z(&inp);
            assert_eq!(r.early_test, early_test, "{name}: early_test");
            assert_eq!(r.early_write, early_write, "{name}: early_write");
            assert_eq!(r.pe_bits, pe, "{name}: PE_DEPTH_CONFIG bits {:#x}", r.pe_bits);
            assert_eq!(r.ra_word, ra, "{name}: RA_EARLY_DEPTH {:#x}", r.ra_word);
        }
    }

    #[test]
    fn a_linear_target_gives_what_the_back_end_writes_today() {
        /* backend.rs emit_ra_state: BASE | FORWARD_Z | FORWARD_W | WRITE_DISABLE,
         * no TEST_ENABLE, and PE_DEPTH_CONFIG without EARLY_Z. */
        let today = GC_RA_EARLY_DEPTH_BASE
            | GC_RA_EARLY_DEPTH_FORWARD_Z
            | GC_RA_EARLY_DEPTH_FORWARD_W
            | GC_RA_EARLY_DEPTH_WRITE_DISABLE;
        for inp in [write(Linear), test_only(Linear), EarlyZInput { uses_discard: true, ..write(Linear) }] {
            let r = early_z(&inp);
            assert!(!r.allowed);
            assert_eq!(r.ra_word, today);
            assert_eq!(r.pe_bits & EZ, 0);
        }
    }

    #[test]
    fn both_halves_follow_one_decision() {
        for color in [Absent, Linear, Tiled] {
            for bits in 0u32..(1 << 7) {
                let mut inp = write(color);
                inp.depth_enabled = bits & 1 != 0;
                inp.depth_func_always = bits & 2 != 0;
                inp.depth_write = bits & 4 != 0;
                inp.alpha_test = bits & 8 != 0;
                inp.writes_z = bits & 16 != 0;
                inp.uses_discard = bits & 32 != 0;
                if bits & 64 != 0 {
                    inp.stencil = stencil_ops_replace();
                }
                let r = early_z(&inp);
                assert_eq!(r.pe_bits & EZ != 0, r.ra_word & TEST != 0, "{inp:?}");
                assert_eq!(r.ra_word & WD == 0, r.early_write, "{inp:?}");
                assert!(!(r.early_test && r.late_test));
            }
        }
    }

    fn gl_draw(depth_test: bool, depth_write: bool) -> GlesDraw {
        let mut d = GlesDraw::ZERO;
        d.depth_id = 1;
        d.stencil_id = 1;
        d.depth_test = depth_test;
        d.depth_func = 1; /* LESS */
        d.depth_write = depth_write;
        d
    }

    /* tools/vivante/README.md, ts-golden mode 8: RA_EARLY_DEPTH 0x05000031 and
     * PE_DEPTH_CONFIG 0x05051111 (of which EARLY_Z | DISABLE_ZS are 0x01010000) */
    const GOLDEN8_RA: u32 = 0x0500_0031;
    const GOLDEN8_PE: u32 = 0x0101_0000;
    /* today's RA word: BASE | FORWARD_W | FORWARD_Z | WRITE_DISABLE */
    const TODAY_RA: u32 = 0x1500_0030;

    #[test]
    fn a_depth_writing_draw_into_a_shadow_gives_golden_mode_8() {
        let r = early_z(&input_of(&gl_draw(true, true), true, false, false, false));
        assert!(r.early_test && r.early_write);
        assert_eq!((r.pe_bits, r.ra_word), (GOLDEN8_PE, GOLDEN8_RA));
        assert_eq!(decision_bits(&r), 0b10011);
    }

    #[test]
    fn what_keeps_todays_words() {
        let d = gl_draw(true, true);
        let mut stencil = gl_draw(false, false);
        stencil.stencil = true;
        stencil.stencil_func = GLES_CMP_ALWAYS;
        stencil.stencil_back_func = GLES_CMP_ALWAYS;
        stencil.stencil_dppass = 2; /* REPLACE */
        stencil.stencil_back_dppass = 2;
        let cases = [
            ("linear target", input_of(&d, false, false, false, false)),
            ("discard", input_of(&d, true, false, true, false)),
            ("writes_z", input_of(&d, true, true, false, false)),
            ("clear quad", input_of(&d, true, false, false, true)),
            ("always stencil, ops not keep", input_of(&stencil, true, false, false, false)),
        ];
        for (name, i) in cases {
            let r = early_z(&i);
            assert!(!r.early_test && !r.early_write, "{name}");
            assert_eq!(r.pe_bits & EZ, 0, "{name}");
            assert_eq!(r.ra_word, TODAY_RA, "{name}");
        }
    }

    #[test]
    fn input_of_reads_depth_stencil_and_samples_from_the_draw() {
        /* no attachment: the PE skips depth whatever GL says */
        let mut d = gl_draw(true, true);
        d.depth_id = 0;
        d.stencil_id = 0;
        let i = input_of(&d, true, false, false, false);
        assert!(!i.depth_enabled && !i.depth_write);
        /* a test with ALWAYS is no test; a write needs the test on */
        let mut d = gl_draw(true, true);
        d.depth_func = GLES_CMP_ALWAYS;
        assert!(input_of(&d, true, false, false, false).depth_func_always);
        assert!(!input_of(&gl_draw(false, true), true, false, false, false).depth_write);
        /* a comparing stencil is not a modification: early stays */
        let mut s = gl_draw(true, true);
        s.stencil = true;
        s.stencil_func = 2;
        let r = early_z(&input_of(&s, true, false, false, false));
        assert!(r.early_test && r.early_write);
        let mut m = gl_draw(true, true);
        m.samples = 4;
        assert!(input_of(&m, true, false, false, false).msaa);
        assert_eq!(input_of(&gl_draw(true, true), true, false, false, false).color, Tiled);
    }

    #[test]
    fn decision_bits_tell_every_word_apart() {
        let mut seen = std::collections::BTreeMap::new();
        for bits in 0u32..(1 << 8) {
            let mut inp = write(Tiled);
            inp.depth_enabled = bits & 1 != 0;
            inp.depth_write = bits & 2 != 0;
            inp.uses_discard = bits & 4 != 0;
            inp.writes_z = bits & 8 != 0;
            inp.msaa = bits & 16 != 0;
            if bits & 64 != 0 {
                inp.stencil = stencil_ops_replace();
            }
            if bits & 128 != 0 {
                inp.color = Linear;
            }
            let r = early_z(&inp);
            let words = (r.pe_bits, r.ra_word);
            assert_eq!(*seen.entry(decision_bits(&r)).or_insert(words), words, "{inp:?}");
        }
    }

    #[test]
    fn a_late_stage_on_a_multisampled_target_sets_late_depth_msaa() {
        let r = early_z(&EarlyZInput { msaa: true, ..write(Linear) });
        assert_ne!(r.ra_word & RA_EARLY_DEPTH_LATE_DEPTH_MSAA, 0);
        let r = early_z(&EarlyZInput { msaa: true, ..write(Tiled) });
        assert_eq!(r.ra_word & RA_EARLY_DEPTH_LATE_DEPTH_MSAA, 0);
    }
}
