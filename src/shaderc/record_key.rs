// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The record key: how a compiled shader is found, by the in-process
//! compiler's cache and by the precompiled table alike.
//!
//! The source's SHA-256, chained with everything else a compile reads: the
//! varyings a link reads as 0, the attribute locations a link laid a vertex
//! shader out with, the constant W, the sampler-state key and an `if`
//! lowering other than the default. The stage and the blend key are the
//! look-up's other two keys. One definition, because the two look-ups must
//! agree byte for byte: a row the generator made under one key is found at
//! run time only under the same one -- by an image with the compiler
//! (`online::backend`) and by one without it (`precompiled`).

use super::sha256::sha256;
use super::{IfLowering, LodKey, ShcAttribMap, SHC_MAX_DECLARED_ATTRIBS};

/// The bytes of a link's attribute map that decide the compile: each
/// binding's name, location and column count, in order.
pub fn map_key(map: &ShcAttribMap) -> [u8; SHC_MAX_DECLARED_ATTRIBS * 34 + 1] {
    let mut k = [0u8; SHC_MAX_DECLARED_ATTRIBS * 34 + 1];
    k[0] = map.count;
    for (i, b) in map.b[..(map.count as usize).min(map.b.len())].iter().enumerate() {
        let at = 1 + 34 * i;
        for (d, c) in k[at..at + 32].iter_mut().zip(b.name.iter()) {
            *d = *c as u8;
        }
        k[at + 32] = b.location;
        k[at + 33] = b.columns;
    }
    k
}

/// The record key of `glsl` compiled with the zeroed varyings `zero` (their
/// names, each with a NUL; `None` for none), the attribute map `map`, and
/// the facts.
pub fn record_key(
    glsl: &[u8],
    zero: Option<&[u8]>,
    map: Option<&ShcAttribMap>,
    w_constant: bool,
    lod_key: LodKey,
    if_lowering: IfLowering,
) -> [u8; 32] {
    let mut sha = sha256(glsl);
    let mut chain = |extra: &[u8]| {
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&sha);
        both[32..].copy_from_slice(&sha256(extra));
        sha = sha256(&both);
    };
    if let Some(zero) = zero {
        chain(zero);
    }
    if let Some(map) = map {
        chain(&map_key(map));
    }
    if w_constant {
        chain(b"w-constant");
    }
    if !lod_key.is_none() {
        let mut k = *b"lod-key:        ";
        k[8..].copy_from_slice(&lod_key.bits().to_le_bytes());
        chain(&k);
    }
    if if_lowering != IfLowering::Default {
        let mut k = *b"if-lowering:    ";
        k[12..].copy_from_slice(&if_lowering.as_u32().to_le_bytes());
        chain(&k);
    }
    sha
}
