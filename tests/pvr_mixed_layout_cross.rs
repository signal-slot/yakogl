// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The merged mixed-target key functions of `src/target_key.rs` against
//! `pvr::mrt_layout::layout`, the placement the PowerVR end-of-tile emit
//! uses (docs/notes/pvr-int-mixed-targets.md, section 3: the compiler's
//! output starts must equal `mrt_layout`'s). `tests/pvr_mixed_key_spec.rs`
//! checks a hand-written model; this file runs the real functions.
//!
//! Every ordered list of 1 to `SHC_MIXED_MAX_TARGETS` colour formats
//! (`fmt` in 0..=255 with `gles_surf_target_key(fmt)` = `Some`) is checked:
//! (a) a `shc_mixed_out_layout` result equals `layout`'s register starts and
//! total; (b) a `shc_blend_key_mixed` key decodes back to the list;
//! (c) a real mix with no compiler layout spills (or errors) in `layout`;
//! (d) the note's plan mixes start where the note says.
//!
//! HOST-ONLY: pure arithmetic, no GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mixed_layout_cross
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::backend::*;
use yakogl::pvr::mrt_layout::{layout, Place, OUTPUT_REGS};
use yakogl::shaderc::ShcIntRt;
use yakogl::target_key::*;

fn formats() -> Vec<u8> {
    (0..=255u8).filter(|&f| gles_surf_target_key(f).is_some()).collect()
}

/// Every ordered list of 1..=SHC_MIXED_MAX_TARGETS formats.
fn lists() -> Vec<Vec<u8>> {
    let f = formats();
    let mut out: Vec<Vec<u8>> = f.iter().map(|&a| vec![a]).collect();
    let mut prev = out.clone();
    for _ in 1..SHC_MIXED_MAX_TARGETS {
        let mut next = Vec::new();
        for l in &prev {
            for &a in &f {
                let mut n = l.clone();
                n.push(a);
                next.push(n);
            }
        }
        out.extend(next.iter().cloned());
        prev = next;
    }
    out
}

fn keys(fmts: &[u8]) -> Vec<u32> {
    fmts.iter().map(|&f| gles_surf_target_key(f).unwrap()).collect()
}

fn dwords(keys: &[u32]) -> Vec<u8> {
    keys.iter().map(|&k| shc_target_key_dwords(k).unwrap() as u8).collect()
}

fn is_int(k: u32) -> bool {
    ShcIntRt::of_key(k).is_some()
}

fn is_real_mix(keys: &[u32]) -> bool {
    keys.iter().any(|&k| is_int(k)) && keys.iter().any(|&k| !is_int(k))
}

#[test]
fn list_space_is_what_the_test_means() {
    let f = formats();
    assert!(f.len() >= 20, "{} formats", f.len());
    assert_eq!(lists().len(), f.len() + f.len().pow(2) + f.len().pow(3));
    // sRGB has no key: it is refused in a mix (note section 5)
    assert_eq!(gles_surf_target_key(GLES_SURF_SRGB8_A8), None);
}

#[test]
fn layout_starts_and_total_match_mrt_layout() {
    let mut some = 0;
    for l in lists() {
        let k = keys(&l);
        let Some((start, span)) = shc_mixed_out_layout(&k) else { continue };
        some += 1;
        let m = layout(&dwords(&k)).unwrap_or_else(|e| panic!("{l:?}: {e:?}"));
        assert_eq!(m.tile_buffers, 0, "{l:?}");
        assert_eq!(m.output_regs, span, "{l:?}");
        for i in 0..k.len() {
            assert_eq!(m.place[i], Place::Register { first: start[i] }, "{l:?} target {i}");
        }
        for s in &start[k.len()..] {
            assert_eq!(*s, 0, "{l:?}: unused start");
        }
    }
    assert!(some > 100, "{some}");
}

#[test]
fn mixed_key_round_trips() {
    let mut mixed = 0;
    for l in lists() {
        let k = keys(&l);
        let Some(key) = shc_blend_key_mixed(&k) else { continue };
        mixed += 1;
        let (t, n) = shc_blend_key_mixed_targets(key).unwrap_or_else(|| panic!("{l:?}: {key:#x}"));
        assert_eq!(&t[..n], &k[..], "{l:?}");
        assert!(t[n..].iter().all(|&x| x == 0), "{l:?}");
        assert!(shc_blend_key_is_mixed(key), "{l:?}");
        // a key is only given where the compiler has a layout
        assert!(shc_mixed_out_layout(&k).is_some(), "{l:?}");
    }
    assert!(mixed > 50, "{mixed}");
}

/// A real mix the compiler has no layout for must not fit `layout`'s
/// registers either: it spills to a tile buffer or has no program.
#[test]
fn real_mix_without_compiler_layout_spills_in_mrt_layout() {
    let mut none = 0;
    for l in lists() {
        let k = keys(&l);
        if !is_real_mix(&k) || shc_mixed_out_layout(&k).is_some() {
            continue;
        }
        none += 1;
        assert_eq!(shc_blend_key_mixed(&k), None, "{l:?}");
        match layout(&dwords(&k)) {
            Err(_) => {}
            Ok(m) => assert!(m.tile_buffers > 0, "{l:?} fits the registers in mrt_layout but not in the compiler"),
        }
    }
    assert!(none > 0);
}

/// The refused classes, by a short literal list each.
#[test]
fn refused_classes_pinned() {
    let refused = |f: &[u8]| {
        let k = keys(f);
        assert!(is_real_mix(&k), "{f:?}");
        assert_eq!(shc_mixed_out_layout(&k), None, "{f:?}");
        assert_eq!(shc_blend_key_mixed(&k), None, "{f:?}");
        match layout(&dwords(&k)) {
            Err(_) => {}
            Ok(m) => assert!(m.tile_buffers > 0, "{f:?}"),
        }
    };
    // RGBA32UI takes all four registers, nothing fits beside it
    refused(&[GLES_SURF_RGBA8888, GLES_SURF_RGBA32UI]);
    refused(&[GLES_SURF_RGBA32UI, GLES_SURF_R16F]);
    // 2 + 2 + 2 registers: the third passes register 4
    refused(&[GLES_SURF_RGBA16F, GLES_SURF_RG32UI, GLES_SURF_RGBA16UI]);
    // 1 + 2 + 2: the second takes r2-r3, the third has none
    refused(&[GLES_SURF_RGBA8888, GLES_SURF_RGBA16F, GLES_SURF_RG32I]);
    // sRGB has no per-target key at all (note section 5)
    assert_eq!(gles_surf_target_key(GLES_SURF_SRGB8_A8), None);
    // two RGBA8 targets are not a mix
    assert_eq!(shc_blend_key_mixed(&[0, 0]), None);
}

fn starts(f: &[u8]) -> (Vec<u8>, u8) {
    let k = keys(f);
    let (s, span) = shc_mixed_out_layout(&k).unwrap_or_else(|| panic!("{f:?} has no layout"));
    let m = layout(&dwords(&k)).unwrap();
    assert_eq!(m.output_regs, span);
    assert!(shc_blend_key_mixed(&k).is_some(), "{f:?}");
    let s = s[..f.len()].to_vec();
    for (i, &st) in s.iter().enumerate() {
        assert_eq!(m.place[i].source_pos(), st);
    }
    (s, span)
}

#[test]
fn plan_mixes_start_where_the_note_says() {
    assert_eq!(OUTPUT_REGS, 4);
    // RGBA8 + R32UI: r0, r1
    assert_eq!(starts(&[GLES_SURF_RGBA8888, GLES_SURF_R32UI]), (vec![0, 1], 2));
    // two integer layouts (the modf pair, R32UI + R32I): r0, r1
    assert_eq!(starts(&[GLES_SURF_R32UI, GLES_SURF_R32I]), (vec![0, 1], 2));
    // two integer layouts of different width: the two-register one on r2
    assert_eq!(starts(&[GLES_SURF_R32I, GLES_SURF_RG32I]), (vec![0, 2], 4));
    // three targets of the note's table
    assert_eq!(starts(&[GLES_SURF_RGBA8888, GLES_SURF_R32I, GLES_SURF_RGBA16F]), (vec![0, 1, 2], 4));
    assert_eq!(starts(&[GLES_SURF_RGBA16F, GLES_SURF_R32UI, GLES_SURF_RGBA8888]), (vec![0, 2, 3], 4));
}
