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
use crate::shaderc::{shc_blend_key_int_rt, shc_blend_key_rt, ShcIntRt, SHC_BLEND_KEY_REPLACE, SHC_RT_F16X4};

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

/// Bit 63: the tag of a 64-bit multi key (docs/notes/pvr-blend-multi.md, 3.2).
pub const SHC_BLEND_KEY_MULTI_TAG: u64 = 1 << 63;
/// Bit 62 of a multi key: reserved, must be 0.
pub const SHC_BLEND_KEY_MULTI_RESERVED: u64 = 1 << 62;
/// The draw buffers a multi key carries.
pub const SHC_MULTI_SLOTS: usize = 4;
/// Bits 0-25 of a multi key: the blend word (the positions of `shc_blend_key`).
const MULTI_BLEND_MASK: u32 = (1 << 26) - 1;

/// The channels (bit 0 = R .. 3 = A) a slot code's class has; 0 for no target
/// or a code that is not valid.
const fn slot_channels(code: u32) -> u32 {
    if code & 3 != 0 {
        return match ShcIntRt::of_key(code << 5) {
            Some(rt) => (1 << rt.channels) - 1,
            None => 0,
        };
    }
    match code {
        0x10 | 0x14 => 0xf,
        0x18 | 0x1C => 1,
        _ => 0,
    }
}

/// A slot value: [0:4] the slot code, [5:8] the disabled-channel mask. Returns
/// the code and the mask reduced to the class's channels; `None` when it is
/// not a valid code (0 = no target is valid; the sRGB codes 0x04, 0x08 and
/// 0x0C are reserved and refused) or carries bits past 8. With `canonical`,
/// the mask must already keep only the channels the class has.
const fn multi_slot_ok(slot: u32, canonical: bool) -> Option<(u32, u32)> {
    if slot >> 9 != 0 {
        return None;
    }
    let code = slot & 0x1f;
    let mask = slot >> 5;
    if code != 0 && slot_key(code).is_none() {
        return None;
    }
    let has = slot_channels(code);
    if canonical && mask & !has != 0 {
        return None;
    }
    Some((code, mask & has))
}

/// The multi key of a draw (section 3.2): `blend` is the shared blend word
/// (bits 0-25, as `shc_blend_key` writes it; the replace state is written as
/// 0, section 3.3) and `slots[i]` draw buffer `i`'s slot value, the 5-bit slot
/// code with the glColorMask-disabled channels in bits 5-8. A zero slot is a
/// gap. Mask bits of channels the class lacks are dropped. `None` for a blend
/// word past bit 25, a slot past 9 bits, a reserved (sRGB) or invalid slot
/// code.
pub const fn shc_blend_key_multi(blend: u32, slots: [u32; SHC_MULTI_SLOTS]) -> Option<u64> {
    if blend & !MULTI_BLEND_MASK != 0 {
        return None;
    }
    let blend = if blend == SHC_BLEND_KEY_REPLACE { 0 } else { blend };
    let mut key = SHC_BLEND_KEY_MULTI_TAG | blend as u64;
    let mut i = 0;
    while i < SHC_MULTI_SLOTS {
        let (code, mask) = match multi_slot_ok(slots[i], false) {
            Some(v) => v,
            None => return None,
        };
        key |= ((code | (mask << 5)) as u64) << (26 + 9 * i);
        i += 1;
    }
    Some(key)
}

/// The blend word and slot values of a canonical multi key; `None` for any
/// other key: no tag, bit 62 set, a replace-state blend word (written as 0),
/// a reserved or invalid slot code, or a mask bit of a channel the class
/// lacks. `shc_blend_key_multi(blend, slots) == Some(key)` for the result.
pub const fn shc_blend_key_multi_slots(key: u64) -> Option<(u32, [u32; SHC_MULTI_SLOTS])> {
    if key & SHC_BLEND_KEY_MULTI_TAG == 0 || key & SHC_BLEND_KEY_MULTI_RESERVED != 0 {
        return None;
    }
    let blend = (key as u32) & MULTI_BLEND_MASK;
    if blend == SHC_BLEND_KEY_REPLACE {
        return None;
    }
    let mut slots = [0u32; SHC_MULTI_SLOTS];
    let mut i = 0;
    while i < SHC_MULTI_SLOTS {
        let slot = ((key >> (26 + 9 * i)) & 0x1ff) as u32;
        if multi_slot_ok(slot, true).is_none() {
            return None;
        }
        slots[i] = slot;
        i += 1;
    }
    Some((blend, slots))
}

/// The canonical rule of section 3.3: does a draw of `blend` and `slots` (as
/// for [`shc_blend_key_multi`]) need a multi key, or does it keep its legacy
/// key? `None` when the arguments are not valid for the constructor.
///
/// A multi key is used exactly when more than one target is present and no
/// legacy key expresses the draw: a non-replace blend with a present
/// non-integer target, any disabled channel on a present target, or no
/// blend and no mask with classes that are neither one class (the one-class
/// replace key) nor a dense blend-free mix of up to
/// `SHC_MIXED_MAX_TARGETS` targets (the bit-25 mixed key).
pub const fn shc_blend_key_needs_multi(blend: u32, slots: [u32; SHC_MULTI_SLOTS]) -> Option<bool> {
    if blend & !MULTI_BLEND_MASK != 0 {
        return None;
    }
    let mut present = 0;
    let mut any_float = false;
    let mut any_mask = false;
    let mut same_class = true;
    let mut first = 0;
    let mut dense = true;
    let mut gap = false;
    let mut keys = [0u32; SHC_MULTI_SLOTS];
    let mut i = 0;
    while i < SHC_MULTI_SLOTS {
        let (code, mask) = match multi_slot_ok(slots[i], false) {
            Some(v) => v,
            None => return None,
        };
        if code == 0 {
            gap = true;
        } else {
            if gap {
                dense = false;
            }
            if present == 0 {
                first = code;
            } else if code != first {
                same_class = false;
            }
            if code & 3 == 0 {
                any_float = true;
            }
            if mask != 0 {
                any_mask = true;
            }
            keys[present] = match slot_key(code) {
                Some(k) => k,
                None => return None,
            };
            present += 1;
        }
        i += 1;
    }
    if present < 2 {
        return Some(false);
    }
    if any_mask || (blend != 0 && blend != SHC_BLEND_KEY_REPLACE && any_float) {
        return Some(true);
    }
    if same_class {
        return Some(false);
    }
    let legacy = dense
        && match present {
            2 => shc_blend_key_mixed(&[keys[0], keys[1]]).is_some(),
            3 => shc_blend_key_mixed(&[keys[0], keys[1], keys[2]]).is_some(),
            _ => false,
        };
    Some(!legacy)
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

    /// Every valid slot code: 0 plus the four float classes and each
    /// integer layout `slot_key` accepts.
    fn valid_codes() -> Vec<u32> {
        let mut v = vec![0u32];
        for c in 1..0x20u32 {
            if slot_key(c).is_some() {
                v.push(c);
            }
        }
        for c in [0x10, 0x14, 0x18, 0x1C] {
            assert!(v.contains(&c));
        }
        v
    }

    #[test]
    fn multi_round_trip() {
        let codes = valid_codes();
        assert!(codes.len() > 5);
        let blends = [
            0,
            shc_blend_key(SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, SHC_BF_ONE, SHC_BF_ONE, SHC_BLEND_ADD, SHC_BLEND_MAX),
            shc_blend_key(SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, SHC_BLEND_ADD, SHC_BLEND_ADD),
        ];
        let mut n = 0;
        for &b in &blends {
            for &c0 in &codes {
                for &c3 in &codes {
                    for mask in 0..16u32 {
                        let slots = [c0 | (mask << 5), 0x10 | (mask << 5), c3 | (mask << 5), c0];
                        let key = shc_blend_key_multi(b, slots).unwrap();
                        assert_eq!(key >> 63, 1);
                        assert_eq!(key & SHC_BLEND_KEY_MULTI_RESERVED, 0);
                        let (db, ds) = shc_blend_key_multi_slots(key).unwrap();
                        assert_eq!(db, b);
                        assert_eq!(shc_blend_key_multi(db, ds), Some(key));
                        for i in 0..4 {
                            // Canonical: only the class's channels stay.
                            let code = slots[i] & 0x1f;
                            assert_eq!(ds[i] & 0x1f, code);
                            assert_eq!(ds[i] >> 5, (slots[i] >> 5) & slot_channels(code));
                        }
                        n += 1;
                    }
                }
            }
        }
        assert!(n > 1000);
        // Bit positions: slot i at 26 + 9 i, a zero slot in the middle.
        let k = shc_blend_key_multi(0, [0x10, 0, 0x1C | (1 << 5), 0x18]).unwrap();
        assert_eq!(k, SHC_BLEND_KEY_MULTI_TAG | (0x10 << 26) | ((0x1C | (1 << 5)) << 44) | (0x18 << 53));
        assert_eq!(shc_blend_key_multi(0, [0; 4]), Some(SHC_BLEND_KEY_MULTI_TAG));
        // The replace state is written as 0.
        assert_eq!(shc_blend_key_multi(SHC_BLEND_KEY_REPLACE, [0x10; 4]), shc_blend_key_multi(0, [0x10; 4]));
        // A mask bit of a channel the class lacks is dropped (F32 has R only).
        assert_eq!(
            shc_blend_key_multi(0, [0x1C | (0xf << 5), 0, 0, 0]),
            shc_blend_key_multi(0, [0x1C | (1 << 5), 0, 0, 0])
        );
    }

    #[test]
    fn multi_refusals() {
        let ok = shc_blend_key_multi(0, [0x10, 0x14, 0, 0]).unwrap();
        assert!(shc_blend_key_multi_slots(ok).is_some());
        // Tag clear, reserved bit 62 set.
        assert_eq!(shc_blend_key_multi_slots(ok & !SHC_BLEND_KEY_MULTI_TAG), None);
        assert_eq!(shc_blend_key_multi_slots(ok | SHC_BLEND_KEY_MULTI_RESERVED), None);
        // Reserved sRGB codes and every other invalid code, in every slot.
        for code in 0..0x20u32 {
            let valid = code == 0 || slot_key(code).is_some();
            for i in 0..4 {
                let mut s = [0x10u32; 4];
                s[i] = code;
                assert_eq!(shc_blend_key_multi(0, s).is_some(), valid, "code {code}");
                let raw = SHC_BLEND_KEY_MULTI_TAG | ((code as u64) << (26 + 9 * i));
                assert_eq!(shc_blend_key_multi_slots(raw).is_some(), valid, "code {code}");
            }
        }
        for code in [0x04u64, 0x08, 0x0C] {
            assert_eq!(shc_blend_key_multi(0, [0x10, code as u32, 0, 0]), None);
            assert_eq!(shc_blend_key_multi_slots(SHC_BLEND_KEY_MULTI_TAG | (code << 35)), None);
        }
        // Slot past 9 bits, blend word past bit 25.
        assert_eq!(shc_blend_key_multi(0, [1 << 9, 0, 0, 0]), None);
        assert_eq!(shc_blend_key_multi(1 << 26, [0; 4]), None);
        // Non-canonical decode: a mask bit the class lacks, an empty slot with
        // a mask, a replace blend word.
        assert_eq!(shc_blend_key_multi_slots(SHC_BLEND_KEY_MULTI_TAG | ((0x1C | (2 << 5)) << 26)), None);
        assert_eq!(shc_blend_key_multi_slots(SHC_BLEND_KEY_MULTI_TAG | (1 << 31)), None);
        assert_eq!(shc_blend_key_multi_slots(SHC_BLEND_KEY_MULTI_TAG | SHC_BLEND_KEY_REPLACE as u64), None);
    }

    #[test]
    fn legacy_keys_unchanged_and_untagged() {
        // The one-target keys.
        for cls in 0..4 {
            for mask in 0..16 {
                let k = shc_blend_key_rt(cls) | shc_blend_key_mask_off(mask);
                assert_eq!(shc_blend_key_multi_slots(k as u64), None);
                assert_eq!((k as u64) >> 63, 0);
            }
        }
        assert_eq!(shc_blend_key_rt(SHC_RT_F32), 0xC000_0000);
        assert_eq!(SHC_BLEND_KEY_MIXED_TAG, 1 << 25);
        // The 3-slot mixed keys.
        let u = tk(GLES_SURF_R32UI);
        let mixes: [&[u32]; 3] = [&[u, RGBA8], &[RGBA8, u, F16X4], &[F16X4, RGBA8, u]];
        for m in mixes {
            let k = shc_blend_key_mixed(m).unwrap();
            assert_ne!(k & SHC_BLEND_KEY_MIXED_TAG, 0);
            assert_eq!((k as u64) >> 63, 0);
            assert_eq!(shc_blend_key_multi_slots(k as u64), None);
            // Decoded: the same slots, zero blend word and zero masks.
            let (t, n) = shc_blend_key_mixed_targets(k).unwrap();
            assert_eq!(n, m.len());
            let mut slots = [0u32; 4];
            for i in 0..n {
                slots[i] = slot_code(t[i]).unwrap();
            }
            let mk = shc_blend_key_multi(0, slots).unwrap();
            let (b, ds) = shc_blend_key_multi_slots(mk).unwrap();
            assert_eq!((b, ds), (0, slots));
            assert!(ds.iter().all(|s| s >> 5 == 0));
            assert_eq!(shc_blend_key_needs_multi(0, slots), Some(false));
        }
    }

    #[test]
    fn canonical_rule() {
        let u8c = slot_code(tk(GLES_SURF_R8UI)).unwrap();
        let u32c = slot_code(tk(GLES_SURF_R32UI)).unwrap();
        let f = 0x10;
        let h = 0x14;
        let bl =
            shc_blend_key(SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, SHC_BF_ONE, SHC_BF_ONE, SHC_BLEND_ADD, SHC_BLEND_ADD);
        let r = |b, s| shc_blend_key_needs_multi(b, s).unwrap();
        // One target, or none: never.
        assert!(!r(bl, [f, 0, 0, 0]));
        assert!(!r(bl, [0, 0, 0, f | (3 << 5)]));
        assert!(!r(0, [0; 4]));
        // Same class, replace: legacy. Replace-state word equals 0.
        assert!(!r(0, [f; 4]));
        assert!(!r(SHC_BLEND_KEY_REPLACE, [f, f, 0, f]));
        // Blend or a mask beside several targets: multi.
        assert!(r(bl, [f, f, 0, 0]));
        assert!(r(0, [f, f | (1 << 5), 0, 0]));
        assert!(r(bl, [f, u32c, 0, 0]));
        // Blend where every present target is integer is ignored.
        assert!(!r(bl, [u32c, u32c, 0, 0]));
        // Blend-free mixes: the bit-25 key up to 3 dense slots; else multi.
        assert!(!r(0, [u32c, f, 0, 0]));
        assert!(!r(0, [f, u32c, h, 0]));
        assert!(r(0, [f, u32c, h, f]));
        assert!(r(0, [f, 0, u32c, 0]));
        assert!(r(0, [u8c, u32c, f, h]));
        // Invalid arguments.
        assert_eq!(shc_blend_key_needs_multi(0, [0x04, 0, 0, 0]), None);
    }
}
