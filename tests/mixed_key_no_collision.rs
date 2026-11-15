// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! No mixed key (`shc_blend_key_mixed`, docs/notes/pvr-int-mixed-targets.md
//! section 2) collides with, or decodes as, a key already in use. Every list
//! of one to three colour formats that has a per-target key and makes a
//! mixed key is enumerated; each key round-trips through
//! `shc_blend_key_mixed_targets`, differs from every existing key (the
//! `blend_key` literals of the precompiled rows in src/pvr/internal.rs, the
//! sRGB-encoded key, the V3D sentinel, the single-class and integer-target
//! keys, the pins of tests/gl_blend_key_pin.rs), is not claimed by the
//! integer-target or sRGB decoders, and no existing key reads as mixed.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test mixed_key_no_collision
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeSet;
use yakogl::shaderc::*;
use yakogl::target_key::*;

/// Every list of 1..=3 formats with a per-target key that makes a mixed key.
fn mixes() -> Vec<(Vec<u32>, u32)> {
    // several formats share one per-target key (RGBA8888, XRGB8888, RGB565, ...)
    let fmts: Vec<u32> = (0..=255u8).filter_map(gles_surf_target_key).collect::<BTreeSet<_>>().into_iter().collect();
    assert!(fmts.len() >= 10, "{} formats", fmts.len());
    let mut lists: Vec<Vec<u32>> = fmts.iter().map(|&a| vec![a]).collect();
    for &a in &fmts {
        for &b in &fmts {
            lists.push(vec![a, b]);
            for &c in &fmts {
                lists.push(vec![a, b, c]);
            }
        }
    }
    lists.into_iter().filter_map(|l| shc_blend_key_mixed(&l).map(|k| (l, k))).collect()
}

/// The `blend_key` literals of the precompiled rows.
fn internal_keys() -> BTreeSet<u32> {
    let text = include_str!("../src/pvr/internal.rs");
    let mut keys = BTreeSet::new();
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("blend_key: 0x") else { continue };
        let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        keys.insert(u32::from_str_radix(&hex, 16).unwrap());
    }
    keys
}

/// Every key already in use.
fn existing() -> BTreeSet<u32> {
    let mut s = internal_keys();
    s.insert(0);
    s.insert(0xA000_0000);
    s.insert(SHC_BLEND_KEY_SRGB_ENCODED);
    s.insert(u32::MAX);
    s.insert(SHC_BLEND_KEY_REPLACE);
    s.insert(SHC_BLEND_KEY_KEEP);
    s.insert(SHC_VIV_KEY_FRONT_FLIPPED);
    for cls in [SHC_RT_RGBA8, SHC_RT_F16X4, SHC_RT_F16, SHC_RT_F32] {
        s.insert(shc_blend_key_rt(cls) | SHC_BLEND_KEY_REPLACE);
    }
    // tests/gl_blend_key_pin.rs: the alpha blend, with and without a class and mask
    let alpha = shc_blend_key(
        SHC_BF_SRC_ALPHA,
        SHC_BF_INV_SRC_ALPHA,
        SHC_BF_SRC_ALPHA,
        SHC_BF_INV_SRC_ALPHA,
        SHC_BLEND_ADD,
        SHC_BLEND_ADD,
    );
    for cls in [SHC_RT_RGBA8, SHC_RT_F16X4, SHC_RT_F16, SHC_RT_F32, SHC_RT_SRGB] {
        for mask in 0..16 {
            s.insert(shc_blend_key_target(cls, mask));
            s.insert(alpha | shc_blend_key_target(cls, mask));
            s.insert(SHC_BLEND_KEY_REPLACE | shc_blend_key_target(cls, mask));
            s.insert(SHC_BLEND_KEY_KEEP | shc_blend_key_target(cls, mask));
            s.insert(alpha | shc_blend_key_mask_off(mask));
        }
    }
    // every integer layout, with every colour mask
    for channels in [1u8, 2, 4] {
        for bits in [8u8, 10, 16, 32] {
            for signed in [false, true] {
                if bits == 10 && channels != 4 {
                    continue;
                }
                let k = shc_blend_key_int_rt(ShcIntRt { channels, bits, signed });
                for mask in 0..16 {
                    s.insert(k | shc_blend_key_mask_off(mask));
                }
            }
        }
    }
    s
}

#[test]
fn round_trip() {
    let all = mixes();
    assert!(!all.is_empty());
    for (l, k) in &all {
        let (t, n) = shc_blend_key_mixed_targets(*k).unwrap_or_else(|| panic!("{l:x?}: {k:#x}"));
        assert_eq!(&t[..n], &l[..], "{k:#x}");
        assert!(shc_blend_key_is_mixed(*k), "{k:#x}");
    }
}

#[test]
fn mixed_keys_are_distinct() {
    let all = mixes();
    let set: BTreeSet<u32> = all.iter().map(|(_, k)| *k).collect();
    assert_eq!(set.len(), all.len());
}

#[test]
fn no_mixed_key_equals_an_existing_key() {
    let internal = internal_keys();
    assert!(!internal.is_empty(), "no blend_key found in src/pvr/internal.rs");
    let ex = existing();
    assert!(ex.len() > internal.len());
    for (l, k) in mixes() {
        assert!(!ex.contains(&k), "{l:x?}: {k:#x}");
    }
}

#[test]
fn no_existing_key_is_mixed() {
    for k in existing() {
        assert!(!shc_blend_key_is_mixed(k), "{k:#x}");
        assert_eq!(shc_blend_key_mixed_targets(k), None, "{k:#x}");
    }
}

#[test]
fn other_decoders_do_not_claim_a_mixed_key() {
    for (l, k) in mixes() {
        assert_eq!(ShcIntRt::of_key(k), None, "{l:x?}: {k:#x}");
        // the sRGB decoder and the class/mask fields read a mixed key as RGBA8, unmasked
        assert_ne!(shc_blend_key_class(k), SHC_RT_SRGB, "{k:#x}");
        assert_eq!(shc_blend_key_class(k), SHC_RT_RGBA8, "{k:#x}");
        assert_eq!(shc_blend_key_masked(k), 0, "{k:#x}");
    }
}
