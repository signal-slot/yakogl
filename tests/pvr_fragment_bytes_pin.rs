// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Pins the bytes the online PowerVR compiler builds for single-class
//! fragment programs, so that the per-target mixed variant
//! (docs/notes/pvr-int-mixed-targets.md section 3.2) provably leaves them
//! alone: key 0, an ordinary blend key, the F16X4 / F16 / F32 render target
//! classes, the one-layout integer variants (R32UI, RG32I, RGBA8UI), the
//! two-output integer executor shape of `common.modf`
//! (tests/es3_pvr_modf.rs) and `SHC_BLEND_KEY_SRGB_ENCODED`.
//!
//! Each row pins an FNV-1a 64 hash of the program bytes with the record's
//! `blend_key`, `num_outputs` and `uses_fbfetch`. The expected values were
//! taken on the tip that added this file. HOST-ONLY: nothing is run.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_fragment_bytes_pin
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{
    shc_blend_key, shc_blend_key_int_rt, shc_blend_key_rt, ShcIntRt, SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA,
    SHC_BLEND_ADD, SHC_BLEND_KEY_REPLACE, SHC_BLEND_KEY_SRGB_ENCODED, SHC_RT_F16, SHC_RT_F16X4, SHC_RT_F32,
};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// (FNV-1a 64 of the program bytes, blend_key, num_outputs, uses_fbfetch)
type Pin = (u64, u32, u8, u8);

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// One fragment shader through the chain under `key`.
fn compile_for(src: &str, key: u32) -> Pin {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l =
            lower::compile_to_ir(a, Stage::Fragment, text).unwrap_or_else(|e| panic!("{src}: front end: {}", e.msg));
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key)
            .unwrap_or_else(|e| panic!("{src}: key {key:#x}: chain: {e:?}"));
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let bytes: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        (fnv1a64(&bytes), rec.blend_key, rec.num_outputs, rec.uses_fbfetch)
    })
    .unwrap()
}

/// A fragment shader of one varying `v` and the outputs `outs` (type, name)
/// at locations 0, 1, ...
fn shader(vty: &str, outs: &[(&str, &str)], body: &str) -> String {
    let mut s = format!(
        "#version 300 es\nprecision highp float;\n{}in highp {vty} v;\n",
        if vty.contains('i') || vty.contains('u') { "flat " } else { "" }
    );
    for (i, (ty, name)) in outs.iter().enumerate() {
        s += &format!("layout(location = {i}) out highp {ty} {name};\n");
    }
    s + &format!("void main() {{ {body} }}\n")
}

const PIN_MOVED: &str = "the pinned bytes of a single-class fragment program moved: the mixed per-target variant \
     (docs/notes/pvr-int-mixed-targets.md 3.2) must keep them. Only a deliberate change regenerates this pin";

fn vec4_src() -> String {
    shader("vec4", &[("vec4", "o")], "o = v * 0.5 + vec4(0.25);")
}

fn int_key(channels: u8, bits: u8, signed: bool) -> u32 {
    shc_blend_key_int_rt(ShcIntRt { channels, bits, signed })
}

/// Expected values, in the order of the rows of `pinned_programs`.
const PINS: [Pin; 10] = [
    (0x6bbd2c2c0b1705a3, 0x0, 1, 0),
    (0x598511f7c007c6c2, 0x98e63, 1, 1),
    (0x7934c7a00e0d42e3, 0x40088621, 2, 0),
    (0xcf4a59b48ed7d1d5, 0x80088621, 1, 0),
    (0x090eb59d26529c64, 0xc0088621, 1, 0),
    (0xbd29d2ed1c61aeab, 0x120, 1, 0),
    (0xd31b2beb66769de6, 0x340, 2, 0),
    (0xdbe30a921efd304e, 0x60, 1, 0),
    (0x89c0d4ea66b6c5e4, 0x140, 4, 0),
    (0x1a1f8b15661bdd38, 0xa0000000, 1, 0),
];

#[test]
fn pinned_programs() {
    let blend = shc_blend_key(
        SHC_BF_SRC_ALPHA,
        SHC_BF_INV_SRC_ALPHA,
        SHC_BF_SRC_ALPHA,
        SHC_BF_INV_SRC_ALPHA,
        SHC_BLEND_ADD,
        SHC_BLEND_ADD,
    );
    let modf = shader(
        "vec2",
        &[("uvec2", "o0"), ("uvec2", "o1")],
        "vec2 i; vec2 f = modf(v, i); o0 = floatBitsToUint(f); o1 = floatBitsToUint(i);",
    );
    let rows: [(&str, String, u32); 10] = [
        ("key 0, vec4", vec4_src(), 0),
        ("ordinary blend key", vec4_src(), blend),
        ("F16X4", vec4_src(), SHC_BLEND_KEY_REPLACE | shc_blend_key_rt(SHC_RT_F16X4)),
        ("F16", vec4_src(), SHC_BLEND_KEY_REPLACE | shc_blend_key_rt(SHC_RT_F16)),
        ("F32", vec4_src(), SHC_BLEND_KEY_REPLACE | shc_blend_key_rt(SHC_RT_F32)),
        ("R32UI", shader("uint", &[("uint", "o")], "o = v * 3u + 1u;"), int_key(1, 32, false)),
        ("RG32I", shader("ivec2", &[("ivec2", "o")], "o = v + ivec2(1, -2);"), int_key(2, 32, true)),
        ("RGBA8UI", shader("uvec4", &[("uvec4", "o")], "o = v + uvec4(1u);"), int_key(4, 8, false)),
        ("modf executor, two RG32UI outputs", modf, int_key(2, 32, false)),
        ("sRGB encoded", vec4_src(), SHC_BLEND_KEY_SRGB_ENCODED),
    ];
    let mut moved = Vec::new();
    for ((name, src, key), want) in rows.into_iter().zip(PINS) {
        let got = compile_for(&src, key);
        if got != want {
            moved.push(format!("{name} (key {key:#x}): pinned {want:x?}, now {got:x?}"));
        }
    }
    assert!(moved.is_empty(), "{PIN_MOVED}:\n{}", moved.join("\n"));
}
