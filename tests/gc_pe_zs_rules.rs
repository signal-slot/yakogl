// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/vivante/pe_zs.rs` (the depth/stencil half of `PE_DEPTH_CONFIG`)
//! against etnaviv, Mesa 26.2.1. The etnaviv driver is not under
//! `third_party/` (`third_party/mesa-26.2.1/UPSTREAM` names the tarball); the
//! `file:line` citations are those of that tarball's tree (the farm's
//! dependency cache, tag `mesa-26.2.1`).
//!
//! Scope: only the config values pe_zs owns: `DISABLE_ZS`, `WRITE_ENABLE` and
//! the depth function, composed with the format bits into a whole word. The
//! early-z enable rules are tests/gc_early_z_rules.rs, the register
//! addresses tests/gc_tiled_regs_mesa.rs; neither is repeated here. The
//! backend runs depth late (early Z off), so the model sets `early_z_allowed`
//! false: `late_zs`/`late_z_test` as etnaviv_state.c:1025-1043 gives them.
//!
//! Known differences from etnaviv (pe_zs is conservative, never wrong) are
//! `#[ignore]`d cases below, with the reason.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante --test gc_pe_zs_rules
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::pe_zs::*;
use yakogl::vivante::regs::*;

/* gallium pipe_func values (src/gallium/include/pipe/p_defines.h:66-72);
 * etnaviv_state.c:1045-1046 says "compare funcs have 1 to 1 mapping". */
const F_LESS: u32 = 1;
const F_LEQUAL: u32 = 3;
const F_ALWAYS: u32 = 7;

/// The GL state of a draw, as st_atom_depth.c turns it into gallium.
#[derive(Clone, Copy)]
struct Gl {
    depth_test: bool,
    func: u32,
    mask: bool,
    /// GL_STENCIL_TEST with a function other than ALWAYS (stencil_enabled).
    stencil_func_test: bool,
    d16: bool,
}

/// etnaviv's PE_DEPTH_CONFIG for a draw on a zsbuf, depth late:
/// - st_atom_depth.c:86-91: depth enabled with the test; the write mask only
///   through the test (and not for EQUAL, which pe_zs cannot see);
/// - etnaviv_zsa.c:48-49: `z_test_enabled`, `z_write_enabled`;
/// - etnaviv_zsa.c:68-72: a stencil function other than ALWAYS gives
///   `stencil_enabled`;
/// - etnaviv_state.c:1025-1026, :1036-1043: `late_zs`, `late_z_test`;
/// - etnaviv_state.c:1045-1051: the word's func, WRITE_ENABLE, DISABLE_ZS;
/// - etnaviv_state.c:428-432 (+ etnaviv_translate.h:220-231 for D16/D24S8):
///   format | DEPTH_MODE_Z | UNK18.
fn etnaviv_word(g: Gl) -> u32 {
    let depth_enabled = g.depth_test;
    let writemask = g.depth_test && g.func != 2 /* EQUAL */ && g.mask;
    let z_test = depth_enabled && g.func != F_ALWAYS;
    let z_write = writemask;
    let late_zs = z_write || g.stencil_func_test;
    let late_z_test = z_test;
    let zsa = pe_depth_func(if z_test { g.func } else { F_ALWAYS })
        | if z_write { GC_PE_DEPTH_WRITE_ENABLE } else { 0 }
        | if !late_zs && !late_z_test { GC_PE_DEPTH_DISABLE_ZS } else { 0 };
    let fmt = if g.d16 { GC_PE_DEPTH_FORMAT_D16 } else { GC_PE_DEPTH_FORMAT_D24S8 };
    fmt | GC_PE_DEPTH_MODE_Z | GC_PE_DEPTH_UNK18 | zsa
}

/// pe_zs's answer composed into the same word.
fn pe_word(g: Gl) -> u32 {
    let o = pe_zs(ZsInput {
        depth_test: g.depth_test,
        depth_write: g.mask,
        stencil_test: g.stencil_func_test,
        has_depth_attachment: true,
        has_stencil_attachment: !g.d16,
        ..Default::default()
    });
    let func = match o.depth_func {
        ZsFunc::Always => F_ALWAYS,
        ZsFunc::Gl => g.func,
    };
    let fmt = if g.d16 { GC_PE_DEPTH_FORMAT_D16 } else { GC_PE_DEPTH_FORMAT_D24S8 };
    fmt | GC_PE_DEPTH_MODE_Z
        | GC_PE_DEPTH_UNK18
        | pe_depth_func(func)
        | if o.depth_write { GC_PE_DEPTH_WRITE_ENABLE } else { 0 }
        | if o.disable_zs { GC_PE_DEPTH_DISABLE_ZS } else { 0 }
}

fn gl(depth_test: bool, func: u32, mask: bool, stencil: bool, d16: bool) -> Gl {
    Gl { depth_test, func, mask, stencil_func_test: stencil, d16 }
}

/// Each glmark2 state with a hand-worked word (cited rules above), and
/// pe_zs equal to etnaviv's.
#[test]
fn glmark2_states_hand_worked() {
    // (state, word): D24S8 = 0x10 | MODE_Z 1 | UNK18 0x40000
    let cases = [
        // LESS, write on: shadow/refract/terrain scene pass. 0x1000 write,
        // func 1<<8, no DISABLE_ZS (etnaviv_state.c:1025, :1036, :1045-1051).
        (gl(true, F_LESS, true, false, false), 0x0004_1111u32),
        // LEQUAL, write on (the GL default clear-and-draw of shadow's map pass).
        (gl(true, F_LEQUAL, true, false, false), 0x0004_1311),
        // LESS, write off: test only (etnaviv_zsa.c:48 test, :49 no write).
        (gl(true, F_LESS, false, false, false), 0x0004_0111),
        (gl(true, F_LEQUAL, false, false, false), 0x0004_0311),
        // D16 (etnaviv_translate.h:224-225: D16 = 0, no 0x10).
        (gl(true, F_LESS, true, false, true), 0x0004_1101),
        (gl(true, F_LEQUAL, false, false, true), 0x0004_0301),
    ];
    for (g, want) in cases {
        assert_eq!(etnaviv_word(g), want, "etnaviv model");
        assert_eq!(pe_word(g), want, "pe_zs");
    }
}

/// Depth test off: DEPTH_FUNC is ALWAYS, no write (st_atom_depth.c:86-91 gives
/// no depth_enabled and no mask), DISABLE_ZS (etnaviv_state.c:1050-1051),
/// whatever glDepthMask and glDepthFunc say.
#[test]
fn depth_test_off() {
    for &d16 in &[false, true] {
        for &mask in &[false, true] {
            for &func in &[F_LESS, F_LEQUAL, F_ALWAYS] {
                let g = gl(false, func, mask, false, d16);
                let fmt = if d16 { 0 } else { 0x10 };
                let want = fmt | 1 | 0x0004_0000 | 0x0700 | GC_PE_DEPTH_DISABLE_ZS;
                assert_eq!(etnaviv_word(g), want);
                assert_eq!(pe_word(g), want);
            }
        }
    }
}

/// Func ALWAYS, write off: `z_test_enabled` false (etnaviv_zsa.c:48), no
/// write, so etnaviv sets DISABLE_ZS (etnaviv_state.c:1050-1051); pe_zs counts
/// the enabled test and leaves the ZS stage on (see the ignored case).
#[test]
fn always_write_off_etnaviv_word() {
    let off = gl(true, F_ALWAYS, false, false, false);
    assert_eq!(etnaviv_word(off), 0x0104_0711);
}

/// Func ALWAYS with write on: the ZS stage is needed for the write
/// (etnaviv_state.c:1025), DISABLE_ZS clear; pe_zs agrees.
#[test]
fn always_write_on() {
    let on = gl(true, F_ALWAYS, true, false, false);
    assert_eq!(etnaviv_word(on), 0x0004_1711);
    assert_eq!(pe_word(on), 0x0004_1711);
}

/// A missed saving, not a wrong picture: pe_zs keeps the ZS stage on for an
/// enabled test with ALWAYS and no write, which etnaviv skips.
#[test]
#[ignore = "pe_zs leaves DISABLE_ZS clear for depth test ALWAYS + write off; etnaviv_zsa.c:48 makes z_test_enabled false, etnaviv_state.c:1050 sets DISABLE_ZS"]
fn always_write_off_disables_zs() {
    let g = gl(true, F_ALWAYS, false, false, false);
    assert_eq!(pe_word(g), etnaviv_word(g));
}

/// Stencil on with a function other than ALWAYS (etnaviv_zsa.c:68-72):
/// `stencil_enabled` needs the ZS stage (etnaviv_state.c:1025), so no
/// DISABLE_ZS even with the depth test off and no write.
#[test]
fn stencil_on() {
    for &test in &[false, true] {
        for &mask in &[false, true] {
            let g = gl(test, F_LESS, mask, true, false);
            assert_eq!(pe_word(g), etnaviv_word(g), "test {test} mask {mask}");
            assert_eq!(pe_word(g) & GC_PE_DEPTH_DISABLE_ZS, 0);
        }
    }
}

/// Every combination of the inputs pe_zs sees (EQUAL and ALWAYS-without-write
/// left out: see the ignored cases) agrees with etnaviv.
#[test]
fn sweep_equals_etnaviv() {
    for t in [false, true] {
        for m in [false, true] {
            for s in [false, true] {
                for d in [false, true] {
                    for f in [F_LESS, F_LEQUAL, F_ALWAYS] {
                        if t && f == F_ALWAYS && !m && !s {
                            continue; // the ignored ALWAYS case
                        }
                        let g = gl(t, f, m, s, d);
                        assert_eq!(pe_word(g), etnaviv_word(g), "t{t} m{m} s{s} d16 {d} f{f}");
                    }
                }
            }
        }
    }
}

/// No zsbuf: etnaviv_state.c:409 (`fb->zsbuf.texture != NULL` fails) and
/// :473 (`DEPTH_MODE_NONE`); the state tracker enables neither depth nor
/// stencil then (st_atom_depth.c:86 `depthBits > 0`, :100 stencil bits), so
/// the ZS stage is off (etnaviv_state.c:1050-1051).
#[test]
fn no_zsbuf_disables_zs() {
    for t in [false, true] {
        for m in [false, true] {
            for s in [false, true] {
                let o = pe_zs(ZsInput { depth_test: t, depth_write: m, stencil_test: s, ..Default::default() });
                assert!(o.disable_zs && !o.depth_write && o.depth_func == ZsFunc::Always);
            }
        }
    }
}

/// st_atom_depth.c:88-91: GL_EQUAL never carries a write mask in etnaviv's
/// input, so LEQUAL-less EQUAL with write on has no WRITE_ENABLE. pe_zs has
/// no function input and writes. Defect (conservative direction is not
/// guaranteed: a write that etnaviv would not do).
#[test]
#[ignore = "pe_zs writes depth for GL_EQUAL with glDepthMask on; st_atom_depth.c:88-91 drops the mask for EQUAL"]
fn equal_has_no_write() {
    let g = gl(true, 2, true, false, false);
    assert_eq!(pe_word(g), etnaviv_word(g));
}

/// etnaviv_zsa.c:68-86: stencil on with func ALWAYS and all-KEEP operations
/// is not `stencil_enabled`, so with no depth use DISABLE_ZS is set. pe_zs
/// has no stencil function or ops input, takes stencil_test as "needs ZS" and
/// leaves DISABLE_ZS clear (a missed saving, not wrong output).
#[test]
#[ignore = "pe_zs cannot see stencil func/ops: stencil ALWAYS+KEEP keeps ZS on (etnaviv_zsa.c:68-86)"]
fn stencil_always_keep_is_no_stencil() {
    let want = etnaviv_word(gl(false, F_ALWAYS, false, false, false));
    let o = pe_zs(ZsInput {
        stencil_test: true,
        has_depth_attachment: true,
        has_stencil_attachment: true,
        ..Default::default()
    });
    assert_eq!(o.disable_zs, want & GC_PE_DEPTH_DISABLE_ZS != 0);
}

/// st_atom_depth.c:100 enables stencil only with stencil bits; a D16 target
/// has none. pe_zs counts `stencil_test` with no stencil attachment.
#[test]
#[ignore = "pe_zs honours stencil_test on a D16 (no stencil) target; st_atom_depth.c:100 does not"]
fn stencil_on_d16_is_off() {
    let o = pe_zs(ZsInput {
        stencil_test: true,
        has_depth_attachment: true,
        has_stencil_attachment: false,
        ..Default::default()
    });
    assert!(o.disable_zs);
}
