// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0's sized colour texture formats (src/gl/es3_formats.rs): the
//! accept / refuse matrix per GPU, the CPU conversion checked texel by texel
//! against reference values written out here, and the completeness rule of
//! ES 3.0 3.8.13. What reaches the GL entry points (glTexImage2D and the
//! rest, on the mock back end) is tested next to the code, in
//! src/gl/es3_format_tests.rs: an integration test cannot make a context.
//!
//! The reference values are computed by hand from the format definitions
//! (ES 3.0.6 2.1.5 / EXT_texture_shared_exponent), never by the code under
//! test. Under `report-es2` the module knows no format (`find` is None) and
//! the whole matrix collapses to that one assertion.

use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::es3_formats::*;

const MASKS: [(&str, u32); 3] =
    [("PowerVR", PVR_SAMPLED_TEXFMTS), ("GC7000UL", GC_SAMPLED_TEXFMTS), ("V3D 4.2", V3D_SAMPLED_TEXFMTS)];

/// Every class this module can write, as a back end that samples them all.
fn all_classes() -> u32 {
    (GLES_TEXFMT_R11G11B10F..=GLES_TEXFMT_RGBA32F).fold(PVR_SAMPLED_TEXFMTS, |m, f| m | gles_texfmt_bit(f))
}

fn fmt(internalformat: u32) -> &'static Es3Format {
    find(internalformat).unwrap_or_else(|| panic!("0x{internalformat:x} is not in the table")).1
}

fn le32(v: u32) -> Vec<u8> {
    v.to_ne_bytes().to_vec()
}

fn le16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_ne_bytes()).collect()
}

fn le32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_ne_bytes()).collect()
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_ne_bytes()).collect()
}

/// Convert one texel of (format, type) into the class `texfmt` of `e`.
fn conv(e: &Es3Format, texfmt: u8, src: &[u8], format: u32, type_: u32) -> Vec<u8> {
    let mut dst = vec![0xAAu8; storage_bytes(texfmt)];
    assert!(convert_row(e, texfmt, src, format, type_, 1, &mut dst), "{} -> class {texfmt}", e.name);
    dst
}

#[test]
fn report_es2_has_no_such_formats() {
    if !cfg!(feature = "report-es2") {
        return;
    }
    for e in FORMATS.iter() {
        assert!(find(e.internalformat).is_none(), "{}", e.name);
    }
    assert!(!transfer_pair_valid(GL_RGBA, GL_UNSIGNED_BYTE));
    assert!(!only_for_new_formats(GL_RED_INTEGER, GL_INT));
}

#[test]
fn the_table_is_consistent() {
    if cfg!(feature = "report-es2") {
        return;
    }
    let mut seen = std::collections::HashSet::new();
    for (i, e) in FORMATS.iter().enumerate() {
        assert!(seen.insert(e.internalformat), "{} twice", e.name);
        assert_eq!(find(e.internalformat).map(|(idx, _)| idx as usize), Some(i + 1), "{}", e.name);
        assert_eq!(by_index(i as u8 + 1).map(|b| b.internalformat), Some(e.internalformat));
        assert_eq!(e.is_integer(), is_integer_format(e.base), "{} base {:x}", e.name, e.base);
        assert!(!e.types.is_empty() && !e.storage.is_empty());
        for &t in e.types {
            assert!(transfer_pair_valid(e.base, t), "{}: ({:x}, {:x}) is not a table 3.2 pair", e.name, e.base, t);
            assert!(texel_bytes(e.base, t) > 0, "{}", e.name);
        }
        /* an integer format is held by an integer class only */
        if e.is_integer() {
            assert!(e.storage.iter().all(|&s| s != GLES_TEXFMT_RGBA8 && s != GLES_TEXFMT_RGBA16F), "{}", e.name);
        }
        for &s in e.storage {
            assert!(gles_texfmt_bytes(s) >= 4, "{}", e.name);
        }
    }
    assert_eq!(by_index(0).map(|e| e.name), None);
    assert!(by_index(FORMATS.len() as u8 + 1).is_none());
    assert_eq!(FORMATS.len(), 35);
}

/// The matrix of ES 3.0 table 3.2 (colour rows): which (format, type) each
/// internal format takes.
#[test]
fn the_accept_matrix_is_table_3_2() {
    if cfg!(feature = "report-es2") {
        return;
    }
    let rows: &[(u32, u32, &[u32])] = &[
        (GL_R8_SNORM, GL_RED, &[GL_BYTE]),
        (GL_RG8_SNORM, GL_RG, &[GL_BYTE]),
        (GL_RGB8_SNORM, GL_RGB, &[GL_BYTE]),
        (GL_RGBA8_SNORM, GL_RGBA, &[GL_BYTE]),
        (GL_R11F_G11F_B10F, GL_RGB, &[GL_UNSIGNED_INT_10F_11F_11F_REV, GL_HALF_FLOAT, GL_FLOAT]),
        (GL_RGB9_E5, GL_RGB, &[GL_UNSIGNED_INT_5_9_9_9_REV, GL_HALF_FLOAT, GL_FLOAT]),
        (GL_RGB16F, GL_RGB, &[GL_HALF_FLOAT, GL_FLOAT]),
        (GL_RG32F, GL_RG, &[GL_FLOAT]),
        (GL_RGB32F, GL_RGB, &[GL_FLOAT]),
        (GL_RGB10_A2, GL_RGBA, &[GL_UNSIGNED_INT_2_10_10_10_REV]),
        (GL_RGB10_A2UI, GL_RGBA_INTEGER, &[GL_UNSIGNED_INT_2_10_10_10_REV]),
        (GL_R8UI, GL_RED_INTEGER, &[GL_UNSIGNED_BYTE]),
        (GL_R8I, GL_RED_INTEGER, &[GL_BYTE]),
        (GL_R16UI, GL_RED_INTEGER, &[GL_UNSIGNED_SHORT]),
        (GL_R16I, GL_RED_INTEGER, &[GL_SHORT]),
        (GL_R32UI, GL_RED_INTEGER, &[GL_UNSIGNED_INT]),
        (GL_R32I, GL_RED_INTEGER, &[GL_INT]),
        (GL_RG8UI, GL_RG_INTEGER, &[GL_UNSIGNED_BYTE]),
        (GL_RG8I, GL_RG_INTEGER, &[GL_BYTE]),
        (GL_RG16UI, GL_RG_INTEGER, &[GL_UNSIGNED_SHORT]),
        (GL_RG16I, GL_RG_INTEGER, &[GL_SHORT]),
        (GL_RG32UI, GL_RG_INTEGER, &[GL_UNSIGNED_INT]),
        (GL_RG32I, GL_RG_INTEGER, &[GL_INT]),
        (GL_RGB8UI, GL_RGB_INTEGER, &[GL_UNSIGNED_BYTE]),
        (GL_RGB8I, GL_RGB_INTEGER, &[GL_BYTE]),
        (GL_RGB16UI, GL_RGB_INTEGER, &[GL_UNSIGNED_SHORT]),
        (GL_RGB16I, GL_RGB_INTEGER, &[GL_SHORT]),
        (GL_RGB32UI, GL_RGB_INTEGER, &[GL_UNSIGNED_INT]),
        (GL_RGB32I, GL_RGB_INTEGER, &[GL_INT]),
        (GL_RGBA8UI, GL_RGBA_INTEGER, &[GL_UNSIGNED_BYTE]),
        (GL_RGBA8I, GL_RGBA_INTEGER, &[GL_BYTE]),
        (GL_RGBA16UI, GL_RGBA_INTEGER, &[GL_UNSIGNED_SHORT]),
        (GL_RGBA16I, GL_RGBA_INTEGER, &[GL_SHORT]),
        (GL_RGBA32UI, GL_RGBA_INTEGER, &[GL_UNSIGNED_INT]),
        (GL_RGBA32I, GL_RGBA_INTEGER, &[GL_INT]),
    ];
    assert_eq!(rows.len(), FORMATS.len(), "a format without a reference row");
    let formats = [GL_RED, GL_RG, GL_RGB, GL_RGBA, GL_RED_INTEGER, GL_RG_INTEGER, GL_RGB_INTEGER, GL_RGBA_INTEGER];
    let types = [
        GL_UNSIGNED_BYTE,
        GL_BYTE,
        GL_UNSIGNED_SHORT,
        GL_SHORT,
        GL_UNSIGNED_INT,
        GL_INT,
        GL_HALF_FLOAT,
        GL_FLOAT,
        GL_UNSIGNED_INT_2_10_10_10_REV,
        GL_UNSIGNED_INT_10F_11F_11F_REV,
        GL_UNSIGNED_INT_5_9_9_9_REV,
    ];
    for &(internalformat, base, ok) in rows {
        let e = fmt(internalformat);
        assert_eq!(e.base, base, "{}", e.name);
        for &f in &formats {
            for &t in &types {
                assert_eq!(e.accepts(f, t), f == base && ok.contains(&t), "{} with ({f:x}, {t:x})", e.name);
            }
        }
        assert_eq!(e.types[0], ok[0], "{}: the first type is the format's own", e.name);
    }
}

/// What each GPU's mask turns each format into: the first class it samples,
/// and a named refusal where it samples none.
#[test]
fn storage_per_gpu_and_the_refusals() {
    if cfg!(feature = "report-es2") {
        return;
    }
    for (gpu, mask) in MASKS {
        for e in FORMATS.iter() {
            let got = e.storage_in(mask);
            let want = e.storage.iter().copied().find(|&s| mask & gles_texfmt_bit(s) != 0);
            assert_eq!(got, want, "{gpu} {}", e.name);
            /* an integer format is held natively or refused, never faked:
             * its only class is its own */
            if e.is_integer() {
                assert_eq!(e.storage.len(), 1, "{gpu} {}: an integer format has one class", e.name);
                assert!(got.is_none() || got == Some(e.storage[0]), "{gpu} {}", e.name);
            }
        }
    }
    /* the PowerVR samples every class natively (formats-pvr) but RGB10_A2's,
     * which stays in RGBA8 until it is a render target there */
    let pvr = PVR_SAMPLED_TEXFMTS;
    for e in FORMATS.iter().filter(|e| e.name != "GL_RGB10_A2") {
        assert_eq!(e.storage_in(pvr), Some(e.storage[0]), "PowerVR {}: its own class", e.name);
    }
    assert_eq!(fmt(GL_R11F_G11F_B10F).storage_in(pvr), Some(GLES_TEXFMT_R11G11B10F));
    assert_eq!(fmt(GL_RGB9_E5).storage_in(pvr), Some(GLES_TEXFMT_RGB9E5));
    assert_eq!(fmt(GL_RGB16F).storage_in(pvr), Some(GLES_TEXFMT_RGBA16F));
    assert_eq!(fmt(GL_RGBA8_SNORM).storage_in(pvr), Some(GLES_TEXFMT_RGBA8_SNORM));
    assert_eq!(fmt(GL_RG32F).storage_in(pvr), Some(GLES_TEXFMT_RGBA32F));
    assert_eq!(fmt(GL_RGB10_A2).storage_in(pvr), Some(GLES_TEXFMT_RGBA8));
    assert_eq!(fmt(GL_RGB10_A2UI).storage_in(pvr), Some(GLES_TEXFMT_RGB10A2UI));
    assert_eq!(fmt(GL_RG16UI).storage_in(pvr), Some(GLES_TEXFMT_RGBA16UI));
    /* the fallbacks that exist on the GC7000: the original classes only */
    let gc = GC_SAMPLED_TEXFMTS;
    assert_eq!(fmt(GL_R11F_G11F_B10F).storage_in(gc), Some(GLES_TEXFMT_RGBA16F));
    assert_eq!(fmt(GL_RGB9_E5).storage_in(gc), Some(GLES_TEXFMT_RGBA16F));
    assert_eq!(fmt(GL_RGBA8_SNORM).storage_in(gc), Some(GLES_TEXFMT_RGBA16F));
    assert_eq!(fmt(GL_RG32F).storage_in(gc), Some(GLES_TEXFMT_RGBA16F));
    assert_eq!(fmt(GL_RGB10_A2).storage_in(gc), Some(GLES_TEXFMT_RGBA8));
    assert_eq!(fmt(GL_RGB10_A2UI).storage_in(gc), None);
    assert_eq!(fmt(GL_R8UI).storage_in(gc), None);
    /* the V3D TMU reads no half float yet: those are refused there */
    let v3d = V3D_SAMPLED_TEXFMTS;
    assert_eq!(fmt(GL_R11F_G11F_B10F).storage_in(v3d), None);
    assert_eq!(fmt(GL_RGB10_A2).storage_in(v3d), Some(GLES_TEXFMT_RGBA8));
    /* a back end that samples a native class gets it, ahead of the fallback */
    let native = gc | gles_texfmt_bit(GLES_TEXFMT_R11G11B10F) | gles_texfmt_bit(GLES_TEXFMT_RGBA8UI);
    assert_eq!(fmt(GL_R11F_G11F_B10F).storage_in(native), Some(GLES_TEXFMT_R11G11B10F));
    assert_eq!(fmt(GL_R8UI).storage_in(native), Some(GLES_TEXFMT_RGBA8UI));
    assert_eq!(fmt(GL_RG16UI).storage_in(native), None);
    /* the build's own mask is one of the three */
    assert!(MASKS.iter().any(|&(_, m)| m == sampled_texfmts()));
}

#[test]
fn bytes_per_texel_of_every_pair() {
    if cfg!(feature = "report-es2") {
        return;
    }
    assert_eq!(texel_bytes(GL_RED, GL_UNSIGNED_BYTE), 1);
    assert_eq!(texel_bytes(GL_RGB, GL_BYTE), 3);
    assert_eq!(texel_bytes(GL_RG_INTEGER, GL_SHORT), 4);
    assert_eq!(texel_bytes(GL_RGBA_INTEGER, GL_UNSIGNED_INT), 16);
    assert_eq!(texel_bytes(GL_RGB, GL_HALF_FLOAT), 6);
    assert_eq!(texel_bytes(GL_RGB, GL_FLOAT), 12);
    assert_eq!(texel_bytes(GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV), 4);
    assert_eq!(texel_bytes(GL_RGB, GL_UNSIGNED_INT_5_9_9_9_REV), 4);
    assert_eq!(texel_bytes(GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV), 4);
    assert_eq!(texel_bytes(GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4), 2);
    assert_eq!(texel_bytes(GL_RGBA, 0x1234), 0);
    for (i, e) in FORMATS.iter().enumerate() {
        for &s in e.storage {
            assert_eq!(storage_bytes(s) as u32, gles_texfmt_bytes(s), "{i}");
        }
    }
}

#[test]
fn half_and_small_float_conversions() {
    /* binary16 */
    assert_eq!(f32_to_half(1.0), 0x3C00);
    assert_eq!(f32_to_half(-2.0), 0xC000);
    assert_eq!(f32_to_half(65504.0), 0x7BFF);
    assert_eq!(f32_to_half(65520.0), 0x7C00, "rounds up to infinity");
    assert_eq!(f32_to_half(1.0 / 16384.0 / 1024.0), 0x0001, "smallest subnormal 2^-24");
    assert_eq!(f32_to_half(1.0 + 1.0 / 2048.0), 0x3C00, "tie to even");
    assert_eq!(f32_to_half(1.0 + 3.0 / 2048.0), 0x3C02, "tie to even, upward");
    assert_eq!(half_to_f32(0x0001), 2.0f32.powi(-24));
    assert_eq!(half_to_f32(0x3555), 1365.0 / 4096.0);
    assert!(half_to_f32(0x7E00).is_nan());
    for h in (0u16..0x7C00).step_by(7) {
        assert_eq!(f32_to_half(half_to_f32(h)), h, "round trip of 0x{h:04x}");
    }
    /* the 11- and 10-bit unsigned floats */
    assert_eq!(f32_to_small_float(1.0, 6), 0x3C0);
    assert_eq!(f32_to_small_float(0.5, 6), 0x380);
    assert_eq!(f32_to_small_float(2.0, 5), 0x200);
    assert_eq!(f32_to_small_float(-1.0, 6), 0, "negatives clamp to 0");
    assert_eq!(f32_to_small_float(f32::NAN, 6), 0);
    assert_eq!(f32_to_small_float(1.0e9, 6), 0x7BF, "overflow clamps to 65024");
    assert_eq!(small_float_to_f32(0x7BF, 6), 65024.0);
    assert_eq!(small_float_to_f32(1, 6), 2.0f32.powi(-20), "smallest 11-bit subnormal");
    assert_eq!(small_float_to_f32(1, 5), 2.0f32.powi(-19), "smallest 10-bit subnormal");
    for v in 0..0x7C0u32 {
        assert_eq!(f32_to_small_float(small_float_to_f32(v, 6), 6), v, "11-bit 0x{v:x}");
    }
    for v in 0..0x3E0u32 {
        assert_eq!(f32_to_small_float(small_float_to_f32(v, 5), 5), v, "10-bit 0x{v:x}");
    }
}

#[test]
fn shared_exponent_reference_values() {
    /* max 2.0: shared exponent 1 + 15 + 1 = 17, mantissas of 2^-7 units */
    let p = pack_rgb9e5(1.0, 0.5, 2.0);
    assert_eq!(p, 128 | (64 << 9) | (256 << 18) | (17 << 27));
    assert_eq!(unpack_rgb9e5(p), [1.0, 0.5, 2.0]);
    assert_eq!(pack_rgb9e5(0.0, 0.0, 0.0), 0);
    /* the largest value: 65408 = 511 * 2^7 */
    let max = pack_rgb9e5(1.0e9, -1.0, f32::NAN);
    assert_eq!(unpack_rgb9e5(max), [65408.0, 0.0, 0.0]);
    assert_eq!(max, 511 | (31 << 27));
    /* a value that rounds the mantissa up to 512 moves the exponent */
    let p = pack_rgb9e5(1023.9, 0.0, 0.0);
    assert_eq!(unpack_rgb9e5(p)[0], 1024.0);
    /* every representable value survives */
    for e in [0u32, 1, 15, 16, 30, 31] {
        for m in [0u32, 1, 255, 511] {
            let w = m | (e << 27);
            let [r, _, _] = unpack_rgb9e5(w);
            assert_eq!(unpack_rgb9e5(pack_rgb9e5(r, 0.0, 0.0))[0], r, "e {e} m {m}");
        }
    }
    let w = pack_r11g11b10f(1.0, 0.5, 2.0);
    assert_eq!(w, 0x3C0 | (0x380 << 11) | (0x200 << 22));
    assert_eq!(unpack_r11g11b10f(w), [1.0, 0.5, 2.0]);
}

#[test]
fn snorm_conversion() {
    if cfg!(feature = "report-es2") {
        return;
    }
    let e = fmt(GL_RGBA8_SNORM);
    /* 127 -> 1.0, -127 and -128 -> -1.0, 0 -> 0, 64 -> 64/127 */
    let src: Vec<u8> = [127i8, -127, -128, 64].iter().map(|&b| b as u8).collect();
    assert_eq!(conv(e, GLES_TEXFMT_RGBA8_SNORM, &src, GL_RGBA, GL_BYTE), vec![127u8, 0x81, 0x81, 64]);
    /* halves: -1.0 = 0xBC00, 64/127 = 0.503937... */
    let halves = conv(e, GLES_TEXFMT_RGBA16F, &src, GL_RGBA, GL_BYTE);
    assert_eq!(halves, le16s(&[0x3C00, 0xBC00, 0xBC00, f32_to_half(64.0 / 127.0)]));
    /* R8_SNORM reads (r, 0, 0, 1) */
    let r = conv(fmt(GL_R8_SNORM), GLES_TEXFMT_RGBA8_SNORM, &[(-64i8) as u8], GL_RED, GL_BYTE);
    assert_eq!(r, vec![0xC0, 0, 0, 127]);
    /* RG8_SNORM */
    let rg = conv(fmt(GL_RG8_SNORM), GLES_TEXFMT_RGBA8_SNORM, &[10, 20], GL_RG, GL_BYTE);
    assert_eq!(rg, vec![10, 20, 0, 127]);
    /* RGB8_SNORM: alpha 1 */
    let rgb = conv(fmt(GL_RGB8_SNORM), GLES_TEXFMT_RGBA8_SNORM, &[1, 2, 3], GL_RGB, GL_BYTE);
    assert_eq!(rgb, vec![1, 2, 3, 127]);
}

#[test]
fn packed_float_conversion() {
    if cfg!(feature = "report-es2") {
        return;
    }
    let e = fmt(GL_R11F_G11F_B10F);
    let floats = f32s(&[1.0, 0.5, 2.0]);
    let word = 0x3C0u32 | (0x380 << 11) | (0x200 << 22);
    /* native: the packed word from floats, from halves, and from itself */
    assert_eq!(conv(e, GLES_TEXFMT_R11G11B10F, &floats, GL_RGB, GL_FLOAT), le32(word));
    assert_eq!(conv(e, GLES_TEXFMT_R11G11B10F, &le16s(&[0x3C00, 0x3800, 0x4000]), GL_RGB, GL_HALF_FLOAT), le32(word));
    assert_eq!(conv(e, GLES_TEXFMT_R11G11B10F, &le32(word), GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV), le32(word));
    /* the fallback: exact halves, alpha 1 */
    let halves = le16s(&[0x3C00, 0x3800, 0x4000, 0x3C00]);
    assert_eq!(conv(e, GLES_TEXFMT_RGBA16F, &floats, GL_RGB, GL_FLOAT), halves);
    assert_eq!(conv(e, GLES_TEXFMT_RGBA16F, &le32(word), GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV), halves);
    /* negative values clamp to 0 in the packed word (ES 3.0 2.1.5) */
    let neg = conv(e, GLES_TEXFMT_R11G11B10F, &f32s(&[-1.0, 1.0, -0.0]), GL_RGB, GL_FLOAT);
    assert_eq!(neg, le32(0x3C0 << 11));

    let e = fmt(GL_RGB9_E5);
    let word = 128u32 | (64 << 9) | (256 << 18) | (17 << 27);
    assert_eq!(conv(e, GLES_TEXFMT_RGB9E5, &floats, GL_RGB, GL_FLOAT), le32(word));
    assert_eq!(conv(e, GLES_TEXFMT_RGB9E5, &le32(word), GL_RGB, GL_UNSIGNED_INT_5_9_9_9_REV), le32(word));
    assert_eq!(conv(e, GLES_TEXFMT_RGBA16F, &le32(word), GL_RGB, GL_UNSIGNED_INT_5_9_9_9_REV), halves);

    /* RGB16F / RGB32F / RG32F: component counts and the fallback */
    let h3 = le16s(&[0x3C00, 0x3800, 0x4000]);
    assert_eq!(conv(fmt(GL_RGB16F), GLES_TEXFMT_RGBA16F, &h3, GL_RGB, GL_HALF_FLOAT), halves);
    assert_eq!(conv(fmt(GL_RGB16F), GLES_TEXFMT_RGBA16F, &floats, GL_RGB, GL_FLOAT), halves);
    assert_eq!(
        conv(fmt(GL_RGB32F), GLES_TEXFMT_RGBA32F, &floats, GL_RGB, GL_FLOAT),
        f32s(&[1.0, 0.5, 2.0, 1.0]).to_vec()
    );
    assert_eq!(
        conv(fmt(GL_RG32F), GLES_TEXFMT_RGBA32F, &f32s(&[3.5, -4.0]), GL_RG, GL_FLOAT),
        f32s(&[3.5, -4.0, 0.0, 1.0]).to_vec()
    );
    assert_eq!(
        conv(fmt(GL_RG32F), GLES_TEXFMT_RGBA16F, &f32s(&[3.5, -4.0]), GL_RG, GL_FLOAT),
        le16s(&[f32_to_half(3.5), f32_to_half(-4.0), 0, 0x3C00])
    );
}

#[test]
fn ten_bit_conversion() {
    if cfg!(feature = "report-es2") {
        return;
    }
    /* r 1023, g 512, b 1, a 2 in UNSIGNED_INT_2_10_10_10_REV (r lowest) */
    let word = 1023u32 | (512 << 10) | (1 << 20) | (2 << 30);
    let e = fmt(GL_RGB10_A2);
    assert_eq!(conv(e, GLES_TEXFMT_RGB10A2, &le32(word), GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV), le32(word));
    /* the 8-bit fallback: 512/1023*255 = 127.6 -> 128; 1/1023*255 = 0.25 -> 0; 2/3 -> 170 */
    assert_eq!(
        conv(e, GLES_TEXFMT_RGBA8, &le32(word), GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV),
        vec![255, 128, 0, 170]
    );
    /* from bytes it is not a pair (table 3.2: RGB10_A2 takes only the word) */
    assert!(!e.accepts(GL_RGBA, GL_UNSIGNED_BYTE));
    /* the unsigned integer twin keeps the integers */
    let e = fmt(GL_RGB10_A2UI);
    assert_eq!(
        conv(e, GLES_TEXFMT_RGB10A2UI, &le32(word), GL_RGBA_INTEGER, GL_UNSIGNED_INT_2_10_10_10_REV),
        le32(word)
    );
}

#[test]
fn integer_conversion() {
    if cfg!(feature = "report-es2") {
        return;
    }
    /* R8I -5: sign-extended to the class's 8 bits; the others (0, 0, 1) */
    assert_eq!(conv(fmt(GL_R8I), GLES_TEXFMT_RGBA8I, &[0xFB], GL_RED_INTEGER, GL_BYTE), vec![0xFB, 0, 0, 1]);
    assert_eq!(
        conv(fmt(GL_RG8UI), GLES_TEXFMT_RGBA8UI, &[200, 7], GL_RG_INTEGER, GL_UNSIGNED_BYTE),
        vec![200, 7, 0, 1]
    );
    assert_eq!(
        conv(fmt(GL_RGB8UI), GLES_TEXFMT_RGBA8UI, &[1, 2, 3], GL_RGB_INTEGER, GL_UNSIGNED_BYTE),
        vec![1, 2, 3, 1]
    );
    assert_eq!(
        conv(fmt(GL_RGBA8I), GLES_TEXFMT_RGBA8I, &[0x80, 0x7F, 0, 0xFF], GL_RGBA_INTEGER, GL_BYTE),
        vec![0x80, 0x7F, 0, 0xFF]
    );
    assert_eq!(
        conv(fmt(GL_R16UI), GLES_TEXFMT_RGBA16UI, &le16s(&[0xBEEF]), GL_RED_INTEGER, GL_UNSIGNED_SHORT),
        le16s(&[0xBEEF, 0, 0, 1])
    );
    assert_eq!(
        conv(fmt(GL_RG16I), GLES_TEXFMT_RGBA16I, &le16s(&[0x8000, 0x7FFF]), GL_RG_INTEGER, GL_SHORT),
        le16s(&[0x8000, 0x7FFF, 0, 1])
    );
    assert_eq!(
        conv(fmt(GL_RGB16UI), GLES_TEXFMT_RGBA16UI, &le16s(&[1, 2, 3]), GL_RGB_INTEGER, GL_UNSIGNED_SHORT),
        le16s(&[1, 2, 3, 1])
    );
    assert_eq!(
        conv(fmt(GL_R32I), GLES_TEXFMT_RGBA32I, &le32s(&[0x8000_0000]), GL_RED_INTEGER, GL_INT),
        le32s(&[0x8000_0000, 0, 0, 1])
    );
    assert_eq!(
        conv(fmt(GL_RG32UI), GLES_TEXFMT_RGBA32UI, &le32s(&[u32::MAX, 5]), GL_RG_INTEGER, GL_UNSIGNED_INT),
        le32s(&[u32::MAX, 5, 0, 1])
    );
    assert_eq!(
        conv(fmt(GL_RGBA32UI), GLES_TEXFMT_RGBA32UI, &le32s(&[1, 2, 3, 4]), GL_RGBA_INTEGER, GL_UNSIGNED_INT),
        le32s(&[1, 2, 3, 4])
    );
    /* a row of several texels */
    let mut dst = vec![0u8; 3 * 4];
    assert!(convert_row(
        fmt(GL_RG8UI),
        GLES_TEXFMT_RGBA8UI,
        &[1, 2, 3, 4, 5, 6],
        GL_RG_INTEGER,
        GL_UNSIGNED_BYTE,
        3,
        &mut dst
    ));
    assert_eq!(dst, vec![1u8, 2, 0, 1, 3, 4, 0, 1, 5, 6, 0, 1]);
}

#[test]
fn a_source_the_format_does_not_name_is_refused_by_the_converter() {
    if cfg!(feature = "report-es2") {
        return;
    }
    let mut dst = [0u8; 16];
    /* too few source bytes, a pair that is not unpacked, a class not written */
    assert!(!convert_row(
        fmt(GL_RGBA32UI),
        GLES_TEXFMT_RGBA32UI,
        &[0; 8],
        GL_RGBA_INTEGER,
        GL_UNSIGNED_INT,
        1,
        &mut dst
    ));
    assert!(!convert_row(fmt(GL_R8UI), GLES_TEXFMT_RGBA8UI, &[0; 4], GL_RED_INTEGER, 0x1234, 1, &mut dst));
    assert!(!convert_row(fmt(GL_R8UI), GLES_TEXFMT_ETC2_RGB, &[0; 4], GL_RED_INTEGER, GL_UNSIGNED_BYTE, 1, &mut dst));
    /* a row longer than the destination */
    assert!(!convert_row(fmt(GL_R8UI), GLES_TEXFMT_RGBA8UI, &[0; 8], GL_RED_INTEGER, GL_UNSIGNED_BYTE, 8, &mut dst));
}

/// ES 3.0 3.8.13: integer textures (and those of 32-bit float texels) are
/// complete only with NEAREST magnification and NEAREST or
/// NEAREST_MIPMAP_NEAREST minification.
#[test]
fn completeness_follows_the_filters() {
    if cfg!(feature = "report-es2") {
        return;
    }
    let mins = [
        GL_NEAREST,
        GL_LINEAR,
        GL_NEAREST_MIPMAP_NEAREST,
        GL_LINEAR_MIPMAP_NEAREST,
        GL_NEAREST_MIPMAP_LINEAR,
        GL_LINEAR_MIPMAP_LINEAR,
    ];
    for e in FORMATS.iter() {
        let unfilterable = e.is_integer() || e.internalformat == GL_RG32F || e.internalformat == GL_RGB32F;
        assert_eq!(e.filterable(), !unfilterable, "{}", e.name);
        for &min in &mins {
            for &mag in &[GL_NEAREST, GL_LINEAR] {
                let want =
                    unfilterable && (mag != GL_NEAREST || !matches!(min, GL_NEAREST | GL_NEAREST_MIPMAP_NEAREST));
                assert_eq!(e.incomplete_with(min, mag), want, "{} min {min:x} mag {mag:x}", e.name);
            }
        }
    }
}

/// A back end that samples every class holds every format natively and the
/// conversion lands in it; the build's own mask is never wider than that.
#[test]
fn every_format_converts_into_every_class_it_may_be_stored_in() {
    if cfg!(feature = "report-es2") {
        return;
    }
    let all = all_classes();
    assert_eq!(sampled_texfmts() & !all, 0);
    for e in FORMATS.iter() {
        let tf = e.storage_in(all).expect(e.name);
        assert_eq!(Some(tf), Some(e.storage[0]), "{}: the first class is the native one", e.name);
        for &s in e.storage {
            let sb = texel_bytes(e.base, e.types[0]);
            let src: Vec<u8> = (0..sb * 3).map(|i| (i * 5 + 1) as u8).collect();
            let mut dst = vec![0u8; storage_bytes(s) * 3];
            /* a random bit pattern is a legal texel of every pair but the
             * floats' NaNs, which only have to survive without a panic */
            assert!(convert_row(e, s, &src, e.base, e.types[0], 3, &mut dst), "{} -> {s}", e.name);
        }
    }
}
