// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A test-only reference model of docs/notes/pvr-int-mixed-targets.md
//! (sections 2.1-2.3 and 3), written from the note and not from
//! `src/shaderc`: an encoder from a per-target class list to the mixed key
//! of section 2.2, and the register rule of section 3. It is checked
//! against the note's worked keys, against `pvr::mrt_layout::layout` (the
//! merged placement) for the worked mixes and every 1-3 target list over the
//! note's classes, and against the existing key shapes of section 2.3 for
//! collisions. Where the note and `mrt_layout` disagree the test asserts
//! mrt_layout's placement. Host only, no GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mixed_key_spec
//! ```
#![cfg(all(feature = "gpu-powervr", not(feature = "report-es2")))]

use yakogl::pvr::mrt_layout::{layout, LayoutError, Place};

/// A per-target class of section 2.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Rgba8,
    Rgba16f,
    R16f,
    R32f,
    /// An integer layout: channels 1, 2 or 4; bits 8, 16, 32 or 10
    /// (RGB10_A2UI); signed.
    Int(u32, u32, bool),
}

use Class::*;

const R32UI: Class = Int(1, 32, false);
const R32I: Class = Int(1, 32, true);
const RG32UI: Class = Int(2, 32, false);
const RG32I: Class = Int(2, 32, true);
const RGBA32UI: Class = Int(4, 32, false);

/// The note's 19 integer formats plus the four float/unorm classes.
fn classes() -> Vec<Class> {
    let mut v = vec![Rgba8, Rgba16f, R16f, R32f];
    for ch in [1, 2, 4] {
        for bits in [8, 16, 32] {
            for s in [false, true] {
                v.push(Int(ch, bits, s));
            }
        }
    }
    v.push(Int(4, 10, false));
    v
}

fn is_int(c: Class) -> bool {
    matches!(c, Int(..))
}

/// Section 2.2's slot code.
fn slot_code(c: Class) -> u32 {
    match c {
        Rgba8 => 0x10,
        Rgba16f => 0x14,
        R16f => 0x18,
        R32f => 0x1C,
        Int(ch, bits, s) => {
            let chc = match ch {
                1 => 1,
                2 => 2,
                _ => 3,
            };
            let w = match bits {
                8 => 0,
                16 => 1,
                32 => 2,
                _ => 3,
            };
            chc | w << 2 | (s as u32) << 4
        }
    }
}

/// Output registers (`surf_out_dwords`, section 2.1's table).
fn dwords(c: Class) -> u8 {
    match c {
        Rgba8 | R16f | R32f => 1,
        Rgba16f => 2,
        Int(_, 10, _) => 1,
        Int(ch, bits, _) => (ch * bits).div_ceil(32) as u8,
    }
}

/// Section 3's rule: each target's first register and the span, or `None`.
fn model_layout(t: &[Class]) -> Option<(Vec<u8>, u8)> {
    let mut next = 0u8;
    let mut start = vec![];
    for &c in t {
        let d = dwords(c);
        let mut s = next;
        if d >= 2 && s & 1 == 1 {
            s += 1;
        }
        if s + d > 4 {
            return None;
        }
        start.push(s);
        next = s + d;
    }
    Some((start, next))
}

/// Section 2.2's mixed key, or `None` when the list is not a mix
/// (conditions 3-5 of the note).
fn encode(t: &[Class]) -> Option<u32> {
    if !(2..=3).contains(&t.len()) || !t.iter().any(|&c| is_int(c)) {
        return None;
    }
    if t.iter().all(|&c| c == t[0]) {
        return None; // one layout everywhere keeps today's key (2.4)
    }
    model_layout(t)?;
    let mut key = 1u32 << 25;
    for (i, &c) in t.iter().enumerate() {
        key |= slot_code(c) << (10 + 5 * i);
    }
    Some(key)
}

#[test]
fn worked_keys_of_section_2_2() {
    assert_eq!(encode(&[Rgba8, R32UI]), Some(0x0204_C000));
    assert_eq!(encode(&[R32UI, R32I]), Some(0x020C_A400));
    assert_eq!(encode(&[Rgba8, R32I, Rgba16f]), Some(0x034C_C000));
    assert_eq!(encode(&[Rgba8, RG32UI]), Some(0x0205_4000));
}

#[test]
fn worked_registers_of_section_3() {
    type Case<'a> = (&'a [Class], Option<(&'a [u8], u8)>);
    let cases: [Case; 9] = [
        (&[Rgba8, R32UI], Some((&[0, 1], 2))),
        (&[R32UI, R32I], Some((&[0, 1], 2))),
        (&[Rgba8, RG32UI], Some((&[0, 2], 4))),
        (&[RG32UI, Rgba8], Some((&[0, 2], 3))),
        (&[R32UI, Rgba16f], Some((&[0, 2], 4))),
        (&[Rgba8, R32I, Rgba16f], Some((&[0, 1, 2], 4))),
        (&[Rgba16f, R32UI, Rgba8], Some((&[0, 2, 3], 4))),
        (&[Rgba8, RGBA32UI], None),
        (&[RGBA32UI, Int(1, 8, false)], None),
    ];
    for (t, want) in cases {
        let got = model_layout(t);
        assert_eq!(got, want.map(|(s, n)| (s.to_vec(), n)), "{t:?}");
        assert_eq!(encode(t).is_some(), want.is_some(), "{t:?}");
    }
}

#[test]
fn none_where_the_note_says_none() {
    assert_eq!(encode(&[R32UI]), None, "one target");
    assert_eq!(encode(&[Rgba8, Rgba16f, R32f]), None, "no integer target");
    assert_eq!(encode(&[Rgba8, Rgba8]), None, "no integer target");
    assert_eq!(encode(&[R32UI, R32UI]), None, "one layout everywhere");
    assert_eq!(encode(&[RG32I, RG32I, RG32I]), None, "one layout everywhere");
    assert_eq!(encode(&[Rgba8, RGBA32UI]), None, "five registers");
    assert_eq!(encode(&[]), None);
    assert_eq!(encode(&[R32UI, Rgba8, Rgba8, Rgba8]), None, "four targets");
}

/// Every list of 1..=max classes over the note's classes.
fn lists(max: usize) -> Vec<Vec<Class>> {
    let cs = classes();
    let mut out: Vec<Vec<Class>> = cs.iter().map(|&c| vec![c]).collect();
    let mut prev = out.clone();
    for _ in 1..max {
        let mut next = vec![];
        for p in &prev {
            for &c in &cs {
                let mut l = p.clone();
                l.push(c);
                next.push(l);
            }
        }
        out.extend(next.iter().cloned());
        prev = next;
    }
    out
}

/// The model's register table against `mrt_layout::layout`, for every 1-3
/// target list over the note's classes. A list the model fits in four
/// registers must lay out with no tile buffer at the same starts and span;
/// one it refuses must spill (or have no program).
#[test]
fn registers_match_mrt_layout_for_every_list() {
    let mut n = 0;
    for t in lists(3) {
        let d: Vec<u8> = t.iter().map(|&c| dwords(c)).collect();
        let l = layout(&d);
        match model_layout(&t) {
            Some((start, span)) => {
                let l = l.unwrap_or_else(|e| panic!("{t:?}: model fits, layout says {e:?}"));
                assert_eq!(l.tile_buffers, 0, "{t:?}");
                assert_eq!(l.output_regs, span, "{t:?}");
                let got: Vec<u8> = l.place[..t.len()]
                    .iter()
                    .map(|p| match p {
                        Place::Register { first } => *first,
                        Place::TileBuffer { .. } => panic!("{t:?}: spilled"),
                    })
                    .collect();
                assert_eq!(got, start, "{t:?}");
            }
            None => match l {
                Ok(l) => assert!(l.tile_buffers > 0, "{t:?}: model refuses, layout has no spill"),
                Err(LayoutError::NoProgram { .. }) => {}
                Err(e) => panic!("{t:?}: {e:?}"),
            },
        }
        n += 1;
    }
    assert_eq!(n, 23 + 23 * 23 + 23 * 23 * 23);
}

/// The existing key shapes of section 2.3 never have a mixed key's bits.
#[test]
fn mixed_keys_collide_with_no_existing_tag() {
    const SRGB_ENCODED: u32 = 0xA000_0000;
    let mut keys = vec![];
    for t in lists(3) {
        let Some(k) = encode(&t) else { continue };
        keys.push(k);
        // a blend key and the GC7000's flip key: src_rgb [0:4] is never 0
        assert_eq!(k & 0x1f, 0, "{t:?}");
        // an integer key: [5:6] (channel count) is never 0; a mixed key has [0:9] zero
        assert_eq!(k & 0x3ff, 0, "{t:?}");
        // the sRGB tag (bit 29 with [30:31] >= 2) and the encoded key
        assert_eq!(k >> 26, 0, "{t:?}");
        assert_ne!(k, SRGB_ENCODED, "{t:?}");
        // the Mali tag (bit 31) and V3D's u32::MAX
        assert_eq!(k >> 31, 0, "{t:?}");
        assert_ne!(k, u32::MAX, "{t:?}");
        // never 0 (no variant), and bit 25 set
        assert_eq!(k & 1 << 25, 1 << 25, "{t:?}");
    }
    let count = keys.len();
    assert!(count > 100, "{count} mixed lists");
    // the encoding is injective: distinct lists give distinct keys
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), count);
    // an integer slot code never equals a float one (low two bits)
    for c in classes() {
        assert_eq!(slot_code(c) & 3 != 0, is_int(c), "{c:?}");
    }
}
