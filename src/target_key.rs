// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Per-target and mixed integer/float colour-target keys.
//!
//! A draw into an integer colour attachment beside attachments of another
//! class is compiled as one fragment variant keyed on the list of its
//! targets (docs/notes/pvr-int-mixed-targets.md, sections 2 and 3). This
//! module holds the per-format target key and the mixed key's constructor,
//! decoder and register layout. It is pure const code, used by the GL core,
//! the PowerVR back end and the online compiler, and it is a module of its
//! own so that the key does not wait on `shaderc/mod.rs`.

use crate::backend::{gles_surf_int_layout, gles_surf_is_srgb, gles_surf_rt_class, GLES_SURF_RGB10_A2};
use crate::shaderc::{shc_blend_key_int_rt, shc_blend_key_rt, ShcIntRt, SHC_RT_F16X4};

/// The most draw buffers a mixed key carries (the PowerVR's
/// `backend::DRAW_TARGETS`, RC14).
pub const SHC_MIXED_MAX_TARGETS: usize = 3;

/// Bit 25: the tag of a mixed key ([0:9] and [26:31] clear).
pub const SHC_BLEND_KEY_MIXED_TAG: u32 = 1 << 25;

/// Pixel output registers of a pixel (`PVR_BACKEND_MAX_OUT_REGS`).
const MAX_OUT_REGS: u32 = 4;

/// Slot code of a valid per-target key; `None` for any other value.
const fn slot_code(t: u32) -> Option<u32> {
    if let Some(rt) = ShcIntRt::of_key(t) {
        // `of_key` ignores the mask bits and [10:25]: only the exact key is valid.
        if shc_blend_key_int_rt(rt) != t {
            return None;
        }
        return Some((t >> 5) & 0x1f);
    }
    match t {
        0 | 0x4000_0000 | 0x8000_0000 | 0xC000_0000 => Some(0x10 | ((t >> 30) << 2)),
        _ => None,
    }
}

/// The per-target key of a slot code; `None` for no target (0), a reserved
/// or an invalid code.
const fn slot_key(code: u32) -> Option<u32> {
    if code >= 0x20 {
        return None;
    }
    if code & 3 != 0 {
        let t = code << 5;
        return match ShcIntRt::of_key(t) {
            Some(_) => Some(t),
            None => None,
        };
    }
    match code {
        0x10 | 0x14 | 0x18 | 0x1C => Some(((code >> 2) & 3) << 30),
        _ => None,
    }
}

/// The per-target key of a colour surface format (`GLES_SURF_*`), section
/// 2.1: the one-target key the GL core gives the attachment alone with no
/// blend and no colour mask. `None` for the sRGB surface (refused in a mix)
/// and for any format that is not a colour surface.
pub const fn gles_surf_target_key(fmt: u8) -> Option<u32> {
    if gles_surf_is_srgb(fmt) || fmt > GLES_SURF_RGB10_A2 {
        return None;
    }
    if let Some(l) = gles_surf_int_layout(fmt) {
        return Some(shc_blend_key_int_rt(ShcIntRt { channels: l.channels, bits: l.bits, signed: l.signed }));
    }
    Some(shc_blend_key_rt(gles_surf_rt_class(fmt)))
}

/// The pixel output registers one per-target key's colour takes (section
/// 2.1), or `None` when `t` is not a valid per-target key.
pub const fn shc_target_key_dwords(t: u32) -> Option<u32> {
    if slot_code(t).is_none() {
        return None;
    }
    if let Some(rt) = ShcIntRt::of_key(t) {
        return Some(rt.dwords());
    }
    Some(if t == shc_blend_key_rt(SHC_RT_F16X4) { 2 } else { 1 })
}

/// The first pixel output register of each target and the registers the
/// outputs span (section 3), or `None` when they pass the pixel's four
/// registers, `targets` is empty or longer than SHC_MIXED_MAX_TARGETS, or a
/// per-target key is not valid. `start[i]` for i >= targets.len() is 0.
pub const fn shc_mixed_out_layout(targets: &[u32]) -> Option<([u8; SHC_MIXED_MAX_TARGETS], u8)> {
    if targets.is_empty() || targets.len() > SHC_MIXED_MAX_TARGETS {
        return None;
    }
    let mut start = [0u8; SHC_MIXED_MAX_TARGETS];
    let mut next = 0u32;
    let mut i = 0;
    while i < targets.len() {
        let d = match shc_target_key_dwords(targets[i]) {
            Some(d) => d,
            None => return None,
        };
        let mut s = next;
        if d >= 2 && s & 1 != 0 {
            s += 1;
        }
        if s + d > MAX_OUT_REGS {
            return None;
        }
        start[i] = s as u8;
        next = s + d;
        i += 1;
    }
    Some((start, next as u8))
}

/// The mixed key of draw buffers 0.. (one per-target key each, dense), or
/// `None` when the list is not a mix (section 2.2, conditions 2-5).
pub const fn shc_blend_key_mixed(targets: &[u32]) -> Option<u32> {
    if targets.len() < 2 || targets.len() > SHC_MIXED_MAX_TARGETS {
        return None;
    }
    let mut key = SHC_BLEND_KEY_MIXED_TAG;
    let mut any_int = false;
    let mut all_same_int = true;
    let mut i = 0;
    while i < targets.len() {
        let code = match slot_code(targets[i]) {
            Some(c) => c,
            None => return None,
        };
        if ShcIntRt::of_key(targets[i]).is_some() {
            any_int = true;
        }
        if ShcIntRt::of_key(targets[i]).is_none() || targets[i] != targets[0] {
            all_same_int = false;
        }
        key |= code << (10 + 5 * i);
        i += 1;
    }
    if !any_int || all_same_int || shc_mixed_out_layout(targets).is_none() {
        return None;
    }
    Some(key)
}

/// The per-target keys a mixed key carries and how many; `None` for any
/// key that is not a canonical mixed key (every other key, section 2.3).
pub const fn shc_blend_key_mixed_targets(key: u32) -> Option<([u32; SHC_MIXED_MAX_TARGETS], usize)> {
    if key & 0x3ff != 0 || key >> 26 != 0 || key & SHC_BLEND_KEY_MIXED_TAG == 0 {
        return None;
    }
    let mut out = [0u32; SHC_MIXED_MAX_TARGETS];
    let mut n = 0;
    while n < SHC_MIXED_MAX_TARGETS {
        let code = (key >> (10 + 5 * n)) & 0x1f;
        if code == 0 {
            break;
        }
        match slot_key(code) {
            Some(t) => out[n] = t,
            None => return None,
        }
        n += 1;
    }
    // Dense: nothing present after the first empty slot.
    let mut j = n + 1;
    while j < SHC_MIXED_MAX_TARGETS {
        if (key >> (10 + 5 * j)) & 0x1f != 0 {
            return None;
        }
        j += 1;
    }
    // Canonical: the constructor builds exactly this key from the list.
    let rebuilt = match n {
        2 => shc_blend_key_mixed(&[out[0], out[1]]),
        3 => shc_blend_key_mixed(&[out[0], out[1], out[2]]),
        _ => None,
    };
    match rebuilt {
        Some(k) if k == key => Some((out, n)),
        _ => None,
    }
}

/// `shc_blend_key_mixed_targets(key).is_some()`.
pub const fn shc_blend_key_is_mixed(key: u32) -> bool {
    shc_blend_key_mixed_targets(key).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::*;
    use crate::shaderc::*;

    fn tk(fmt: u8) -> u32 {
        gles_surf_target_key(fmt).unwrap()
    }
    const RGBA8: u32 = 0;
    const F16X4: u32 = 0x4000_0000;

    #[test]
    fn worked_keys() {
        let m = shc_blend_key_mixed;
        assert_eq!(m(&[RGBA8, tk(GLES_SURF_R32UI)]), Some(0x0204_C000));
        assert_eq!(m(&[tk(GLES_SURF_R32UI), tk(GLES_SURF_R32I)]), Some(0x020C_A400));
        assert_eq!(m(&[RGBA8, tk(GLES_SURF_R32I), F16X4]), Some(0x034C_C000));
        assert_eq!(m(&[RGBA8, tk(GLES_SURF_RG32UI)]), Some(0x0205_4000));
    }

    #[test]
    fn round_trip() {
        let mut pool = vec![RGBA8, F16X4, 0x8000_0000, 0xC000_0000];
        for f in GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST {
            pool.push(tk(f));
        }
        let mut built = 0;
        for &a in &pool {
            for &b in &pool {
                if let Some(k) = shc_blend_key_mixed(&[a, b]) {
                    built += 1;
                    assert_eq!(shc_blend_key_mixed_targets(k), Some(([a, b, 0], 2)));
                    assert!(shc_blend_key_is_mixed(k));
                }
                for &c in &pool {
                    if let Some(k) = shc_blend_key_mixed(&[a, b, c]) {
                        built += 1;
                        assert_eq!(shc_blend_key_mixed_targets(k), Some(([a, b, c], 3)));
                    }
                }
            }
        }
        assert!(built > 100);
        // every key the decoder accepts rebuilds to itself (canonical only)
        for v in 0u32..1 << 15 {
            let k = (v << 10) | SHC_BLEND_KEY_MIXED_TAG;
            if let Some((o, n)) = shc_blend_key_mixed_targets(k) {
                assert_eq!(shc_blend_key_mixed(&o[..n]), Some(k));
            }
        }
    }

    #[test]
    fn none_cases() {
        let u = tk(GLES_SURF_R32UI);
        assert_eq!(shc_blend_key_mixed(&[]), None);
        assert_eq!(shc_blend_key_mixed(&[u]), None);
        assert_eq!(shc_blend_key_mixed(&[RGBA8, F16X4]), None);
        assert_eq!(shc_blend_key_mixed(&[u, u]), None);
        assert_eq!(shc_blend_key_mixed(&[u, u, u]), None);
        assert_eq!(shc_blend_key_mixed(&[u, u, RGBA8, u]), None);
        // a masked integer key, a blend key, the sRGB class
        assert_eq!(shc_blend_key_mixed(&[RGBA8, u | shc_blend_key_mask_off(1)]), None);
        assert_eq!(shc_blend_key_mixed(&[RGBA8, shc_blend_key(1, 1, 1, 1, 0, 0)]), None);
        assert_eq!(shc_blend_key_mixed(&[u, SHC_BLEND_KEY_SRGB_ENCODED]), None);
        assert_eq!(shc_blend_key_mixed(&[u, RGBA8 | shc_blend_key_mask_off(1)]), None);
        // past four registers
        assert_eq!(shc_blend_key_mixed(&[RGBA8, tk(GLES_SURF_RGBA32UI)]), None);
        assert_eq!(shc_blend_key_mixed(&[tk(GLES_SURF_RGBA32UI), tk(GLES_SURF_R8UI)]), None);
        // non-canonical keys
        let tag = SHC_BLEND_KEY_MIXED_TAG;
        assert!(!shc_blend_key_is_mixed(tag));
        assert!(!shc_blend_key_is_mixed(tag | 0x10 << 10));
        assert!(!shc_blend_key_is_mixed(tag | 0x10 << 15 | 0x09 << 20));
        assert!(!shc_blend_key_is_mixed(0x0204_C000 | 1));
        assert!(!shc_blend_key_is_mixed(0x0204_C000 | 1 << 26));
        assert!(!shc_blend_key_is_mixed(tag | 0x10 << 10 | 0x04 << 15));
        assert!(!shc_blend_key_is_mixed(tag | 0x10 << 10 | 0x10 << 15));
    }

    #[test]
    fn existing_keys_are_not_mixed() {
        for src in [1u32, 2, 8, 0x11, 0x18] {
            for dst in [1u32, 2, 8, 0x11] {
                for eq in 0..5 {
                    let b = shc_blend_key(src, dst, src, dst, eq, eq);
                    for mask in 0..16 {
                        for cls in 0..4 {
                            assert!(!shc_blend_key_is_mixed(b | shc_blend_key_mask_off(mask) | shc_blend_key_rt(cls)));
                            assert!(!shc_blend_key_is_mixed(b | shc_blend_key_target(cls, mask)));
                        }
                        assert!(!shc_blend_key_is_mixed(b | shc_blend_key_target(SHC_RT_SRGB, mask)));
                    }
                }
            }
        }
        for f in GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST {
            for mask in 0..16 {
                assert!(!shc_blend_key_is_mixed(tk(f) | shc_blend_key_mask_off(mask)));
            }
        }
        for k in [SHC_BLEND_KEY_SRGB_ENCODED, SHC_BLEND_KEY_REPLACE, SHC_BLEND_KEY_KEEP, SHC_VIV_KEY_FRONT_FLIPPED, 0] {
            assert!(!shc_blend_key_is_mixed(k));
        }
    }

    #[test]
    fn layouts() {
        let l = shc_mixed_out_layout;
        let u = tk(GLES_SURF_R32UI);
        let i = tk(GLES_SURF_R32I);
        let rg = tk(GLES_SURF_RG32UI);
        assert_eq!(l(&[RGBA8, u]), Some(([0, 1, 0], 2)));
        assert_eq!(l(&[u, i]), Some(([0, 1, 0], 2)));
        assert_eq!(l(&[RGBA8, rg]), Some(([0, 2, 0], 4)));
        assert_eq!(l(&[rg, RGBA8]), Some(([0, 2, 0], 3)));
        assert_eq!(l(&[u, F16X4]), Some(([0, 2, 0], 4)));
        assert_eq!(l(&[RGBA8, i, F16X4]), Some(([0, 1, 2], 4)));
        assert_eq!(l(&[F16X4, u, RGBA8]), Some(([0, 2, 3], 4)));
        assert_eq!(l(&[RGBA8, tk(GLES_SURF_RGBA32UI)]), None);
        assert_eq!(l(&[tk(GLES_SURF_RGBA32UI), tk(GLES_SURF_R8UI)]), None);
        assert_eq!(l(&[]), None);
        assert_eq!(l(&[RGBA8; 4]), None);
        assert_eq!(l(&[1]), None);
    }

    #[test]
    fn per_format_keys() {
        assert_eq!(gles_surf_target_key(GLES_SURF_SRGB8_A8), None);
        assert_eq!(gles_surf_target_key(GLES_SURF_RGB10_A2 + 1), None);
        assert_eq!(gles_surf_target_key(255), None);
        for f in 0..=GLES_SURF_RGB10_A2 {
            if f == GLES_SURF_SRGB8_A8 {
                continue;
            }
            let k = gles_surf_target_key(f).unwrap();
            let want = if let Some(l) = gles_surf_int_layout(f) {
                let rt = ShcIntRt { channels: l.channels, bits: l.bits, signed: l.signed };
                assert_eq!(k, shc_blend_key_int_rt(rt));
                rt.dwords()
            } else {
                assert_eq!(k, shc_blend_key_rt(gles_surf_rt_class(f)));
                if f == GLES_SURF_RGBA16F {
                    2
                } else {
                    1
                }
            };
            assert_eq!(shc_target_key_dwords(k), Some(want), "format {f}");
        }
        assert_eq!(gles_surf_target_key(GLES_SURF_RGBA8888), Some(shc_blend_key_rt(SHC_RT_RGBA8)));
        assert_eq!(gles_surf_target_key(GLES_SURF_R16F), Some(shc_blend_key_rt(SHC_RT_F16)));
        assert_eq!(gles_surf_target_key(GLES_SURF_R32F), Some(shc_blend_key_rt(SHC_RT_F32)));
        assert_eq!(shc_target_key_dwords(1), None);
    }
}
