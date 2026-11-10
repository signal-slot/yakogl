// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A colour mask on an integer render target on the PowerVR online back end
//! (RC1 partial masks, dEQP-GLES3 `fragment_out` under `glColorMask`): the
//! integer variant (`shc_blend_key_int_rt`) with mask bits on the target's
//! channels compiled with "a colour mask on an integer render target"
//! refused. Now the program reads the pixel's packed dwords from the pixel
//! output registers and keeps the masked fields' bits
//! (`chain::pack_int_output`), as the blend variants keep a masked float
//! channel.
//!
//! Checked for RGBA8UI, RGBA16I and RGBA32UI, every mask that keeps one, two
//! or three channels and the mask of all four: the program compiles, its
//! record reads the pixel (`uses_fbfetch`), every read of a pixel output
//! register carries `olchk`, and the dwords it writes -- run through the
//! host byte model (tests/usc_exec `run_binary`) with a known pixel -- hold
//! the pixel's bits in the masked fields and the shader's in the others.
//!
//! HOST-ONLY: no GPU arithmetic and no read-back of an integer target; the
//! board check is plan #325's board-rt-pcf.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_int_mask
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::operands::srcs;
use yakogl::shaderc::online::powervr::sel::Bank;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, shc_blend_key_mask_off, ShcIntRt};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the chain recorded for one fragment shader.
#[derive(Debug)]
struct Rec {
    num_outputs: u8,
    uses_fbfetch: u8,
    image: Vec<u8>,
}

/// One fragment shader through the chain under the variant `key`, or the
/// refusal.
fn compile_for(src: &str, key: u32) -> Result<Rec, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok(Rec {
            num_outputs: rec.num_outputs,
            uses_fbfetch: rec.uses_fbfetch,
            image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
        })
    })
    .unwrap()
}

/// The three targets: (name, layout, the output's type, the flat input's
/// bits). The input plus (0, 1, 2, 3) per channel: values past an 8-bit
/// field (0x1f0..), and negative ones whose sign bits would spill into the
/// next 16-bit field were the field not masked to its width.
const TARGETS: [(&str, ShcIntRt, &str, u32); 3] = [
    ("RGBA8UI", ShcIntRt { channels: 4, bits: 8, signed: false }, "uvec4", 0x1f0),
    ("RGBA16I", ShcIntRt { channels: 4, bits: 16, signed: true }, "ivec4", (-2i32) as u32),
    ("RGBA32UI", ShcIntRt { channels: 4, bits: 32, signed: false }, "uvec4", 0x8000_0000),
];

/// The pixel already in the target: pixel output registers 0..3.
const OLD: [u32; 4] = [0xdead_beef, 0x0bad_f00d, 0x1357_9bdf, 0x2468_ace0];

/// dEQP's fragment_out shape: one flat integer input, one output.
fn fragment(ty: &str) -> String {
    let c = if ty == "ivec4" { "ivec4(0, 1, 2, 3)" } else { "uvec4(0u, 1u, 2u, 3u)" };
    format!(
        "#version 300 es\nflat in highp {ty} v;\nlayout(location = 0) out highp {ty} o;\n\
         void main() {{ o = v + {c}; }}\n"
    )
}

/// Channel `c` of `rt`: (dword, bit offset, width), as the target packs it.
fn field(rt: ShcIntRt, c: u32) -> (u32, u32, u32) {
    match rt.bits {
        32 => (c, 0, 32),
        16 => (c / 2, 16 * (c % 2), 16),
        _ => (0, 8 * c, 8),
    }
}

/// The dwords GL leaves in the pixel: the masked channels' fields from
/// `old`, the others from the shader's `values`, each cut to its width.
fn expected(rt: ShcIntRt, masked: u32, values: [u32; 4], old: [u32; 4]) -> Vec<u32> {
    let mut out = vec![0u32; rt.dwords() as usize];
    for c in 0..4 {
        let (d, off, w) = field(rt, c);
        let bits = if w == 32 { u32::MAX } else { ((1u32 << w) - 1) << off };
        let v = if masked & (1 << c) != 0 { old[d as usize] } else { values[c as usize] << off };
        out[d as usize] |= v & bits;
    }
    out
}

/// `src` under `key`, run once with every coefficient of the flat input
/// `bits` and the pixel `OLD`: the dwords in pixel outputs 0.., and the
/// record.
fn run(src: &str, key: u32, bits: u32) -> (Vec<u32>, Rec) {
    let rec = compile_for(src, key).unwrap_or_else(|e| panic!("key {key:#x}: {e}"));
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff.fill(bits);
    inp.pixout = OLD[0];
    inp.pixout1 = OLD[1];
    usc_exec::set_pixout_hi([OLD[2], OLD[3]]);
    let out = usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("key {key:#x}: byte model: {e}"));
    let dwords = (0..u32::from(rec.num_outputs))
        .map(|k| *out.get(&k).unwrap_or_else(|| panic!("key {key:#x}: nothing in pixel output {k}: {out:?}")))
        .collect();
    (dwords, rec)
}

/// Every group that reads a pixel output register (special 32..35) waits
/// for the output lock (`olchk`), as pco's pixel reads do; how many there
/// are.
fn pixout_reads_carry_olchk(image: &[u8]) -> usize {
    let special = Bank::Special.hw();
    let (mut off, mut reads) = (0usize, 0usize);
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        let lower = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
        if lower.iter().flatten().any(|o| o.bank == special && (32..36).contains(&o.index)) {
            assert_eq!(g.hdr.get("olchk"), Some(1), "the pixel read at +{off:#x} has no olchk");
            reads += 1;
        }
        off += g.total;
    }
    reads
}

/// The channel masks that keep 1, 2 and 3 of the four channels (bit k set:
/// channel k not written), and the mask of all four.
fn masks() -> impl Iterator<Item = u32> {
    1u32..=15
}

/// (a) and (c): every partial mask and the full mask compile on the three
/// targets, read the pixel, and mark each pixel read with `olchk`; (b) the
/// dwords written keep the masked fields' old bits and take the unmasked
/// ones from the shader.
#[test]
fn masked_channels_keep_the_pixel() {
    let mut checked = 0;
    for (name, rt, ty, bits) in TARGETS {
        let src = fragment(ty);
        let values: [u32; 4] = core::array::from_fn(|c| bits.wrapping_add(c as u32));
        /* the unmasked variant: the shader's own packing, no pixel read */
        let (plain, rec) = run(&src, shc_blend_key_int_rt(rt), bits);
        assert_eq!(plain, expected(rt, 0, values, OLD), "{name}: no mask");
        assert_eq!(rec.uses_fbfetch, 0, "{name}: no mask reads no pixel");
        assert_eq!(pixout_reads_carry_olchk(&rec.image), 0, "{name}: no mask reads no pixel");
        for masked in masks() {
            let key = shc_blend_key_int_rt(rt) | shc_blend_key_mask_off(masked);
            let (got, rec) = run(&src, key, bits);
            let want = expected(rt, masked, values, OLD);
            assert_eq!(got, want, "{name} mask {masked:04b}: {got:08x?}, want {want:08x?}");
            assert_eq!(u32::from(rec.num_outputs), rt.dwords(), "{name} mask {masked:04b}: the target's dwords");
            assert_eq!(rec.uses_fbfetch, 1, "{name} mask {masked:04b}: the record reads the pixel");
            assert!(pixout_reads_carry_olchk(&rec.image) > 0, "{name} mask {masked:04b}: no pixel read");
            checked += 1;
        }
    }
    assert_eq!(checked, 3 * 15);
}

/// The whole target masked writes the pixel back as it was, whatever the
/// shader computes.
#[test]
fn a_full_mask_writes_the_pixel_back() {
    for (name, rt, ty, bits) in TARGETS {
        let key = shc_blend_key_int_rt(rt) | shc_blend_key_mask_off(0xf);
        let (got, _) = run(&fragment(ty), key, bits);
        assert_eq!(got, OLD[..rt.dwords() as usize], "{name}");
    }
}

/// A mask bit on a channel the target lacks is no mask: RG16UI with blue
/// and alpha off is the unmasked program, which reads no pixel.
#[test]
fn a_mask_on_missing_channels_reads_nothing() {
    let rt = ShcIntRt { channels: 2, bits: 16, signed: false };
    let src = "#version 300 es\nflat in highp uvec2 v;\nlayout(location = 0) out highp uvec2 o;\n\
               void main() { o = v; }\n";
    let plain = compile_for(src, shc_blend_key_int_rt(rt)).unwrap();
    let masked = compile_for(src, shc_blend_key_int_rt(rt) | shc_blend_key_mask_off(0b1100)).unwrap();
    assert_eq!(masked.uses_fbfetch, 0);
    assert_eq!(masked.image, plain.image, "the same program");
}
