// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The merged mixed-key functions of `src/target_key.rs` against
//! `pvr::mrt_layout::layout` (the placement the end-of-tile emit uses), for
//! every colour format the key covers and every ordered list of 1 to
//! `SHC_MIXED_MAX_TARGETS` of them (docs/notes/pvr-int-mixed-targets.md,
//! sections 2 and 3: the compiler's output starts must equal mrt_layout's).
//! `pvr_mixed_key_spec` checks a hand-written model; this one checks the real
//! functions.
//!
//! For each list, with `t[i] = gles_surf_target_key(fmt[i])`:
//!
//! - (a) when `shc_mixed_out_layout(t)` is `Some`, `layout` over the same
//!   dword counts puts every target in output registers at the same starts
//!   and spans the same registers;
//! - (b) when `shc_blend_key_mixed(t)` is `Some(k)`,
//!   `shc_blend_key_mixed_targets(k)` gives the list back and
//!   `shc_blend_key_is_mixed(k)` holds;
//! - (c) when `shc_mixed_out_layout(t)` is `None` for a real mix (an integer
//!   target and a target of another class), `layout` spills or errors: the
//!   compiler never refuses a list the emit places in registers;
//! - (d) `shc_blend_key_mixed` is `Some` exactly when the list has two or
//!   more targets, an integer target, is not one integer layout repeated, and
//!   has a register layout.
//!
//! HOST-ONLY: pure arithmetic, no GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mixed_layout_cross
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::pvr::mrt_layout::{layout, Place};
use yakogl::shaderc::ShcIntRt;
use yakogl::target_key::{
    gles_surf_target_key, shc_blend_key_is_mixed, shc_blend_key_mixed, shc_blend_key_mixed_targets,
    shc_mixed_out_layout, shc_target_key_dwords, SHC_MIXED_MAX_TARGETS,
};

#[derive(Default)]
struct Seen {
    lists: usize,
    placed: usize,
    mixed_keys: usize,
    refused_mixes: usize,
}

fn check(fmts: &[u8], keys: &[u32], seen: &mut Seen) {
    let ctx = format!("formats {fmts:?}, keys {keys:x?}");
    let dwords: Vec<u8> = keys.iter().map(|&k| shc_target_key_dwords(k).expect("valid key") as u8).collect();
    let lay = layout(&dwords);
    let shc = shc_mixed_out_layout(keys);
    let is_int = |k: u32| ShcIntRt::of_key(k).is_some();
    let any_int = keys.iter().any(|&k| is_int(k));
    let any_other = keys.iter().any(|&k| !is_int(k));
    let same_int = keys.iter().all(|&k| k == keys[0] && is_int(k));
    seen.lists += 1;

    // (a)
    if let Some((start, total)) = shc {
        seen.placed += 1;
        let l = lay.unwrap_or_else(|e| panic!("{ctx}: shc places, mrt_layout errors {e:?}"));
        assert_eq!(l.tile_buffers, 0, "{ctx}: mrt_layout spills what shc places");
        assert_eq!(l.output_regs, total, "{ctx}: register total");
        for (i, &s) in start.iter().enumerate().take(keys.len()) {
            assert_eq!(l.place[i], Place::Register { first: s }, "{ctx}: target {i}");
        }
        for &s in &start[keys.len()..] {
            assert_eq!(s, 0, "{ctx}: unused start");
        }
    }

    // (b)
    let mixed = shc_blend_key_mixed(keys);
    if let Some(k) = mixed {
        seen.mixed_keys += 1;
        let (back, n) = shc_blend_key_mixed_targets(k).unwrap_or_else(|| panic!("{ctx}: key {k:#x} does not decode"));
        assert_eq!(&back[..n], keys, "{ctx}: round trip");
        assert!(shc_blend_key_is_mixed(k), "{ctx}: is_mixed");
    }

    // (c)
    if shc.is_none() && any_int && any_other {
        seen.refused_mixes += 1;
        if let Ok(l) = lay {
            assert!(l.tile_buffers > 0, "{ctx}: shc refuses a mix mrt_layout keeps in registers");
        }
    }

    // (d)
    let expect = keys.len() >= 2 && any_int && !same_int && shc.is_some();
    assert_eq!(mixed.is_some(), expect, "{ctx}: mixed key presence");
}

fn walk(formats: &[(u8, u32)], fmts: &mut Vec<u8>, keys: &mut Vec<u32>, seen: &mut Seen) {
    if !fmts.is_empty() {
        check(fmts, keys, seen);
    }
    if fmts.len() == SHC_MIXED_MAX_TARGETS {
        return;
    }
    for &(f, k) in formats {
        fmts.push(f);
        keys.push(k);
        walk(formats, fmts, keys, seen);
        fmts.pop();
        keys.pop();
    }
}

#[test]
fn mixed_functions_agree_with_mrt_layout_for_every_format_list() {
    let formats: Vec<(u8, u32)> = (0..=255u8).filter_map(|f| gles_surf_target_key(f).map(|k| (f, k))).collect();
    assert!(formats.len() >= 10, "only {} formats have a target key", formats.len());
    let mut seen = Seen::default();
    walk(&formats, &mut Vec::new(), &mut Vec::new(), &mut seen);
    let n = formats.len();
    assert_eq!(seen.lists, n + n * n + n * n * n);
    // Every branch ran: some lists place, some become mixed keys, some real
    // mixes pass the four registers and are refused.
    assert!(seen.placed > 0, "no list placed");
    assert!(seen.mixed_keys > 0, "no mixed key");
    assert!(seen.refused_mixes > 0, "no refused mix");
}
