// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Colour output arrays narrower than a vec4 on the PowerVR online back end:
//! the shape of dEQP-GLES3 `fragment_out.array.*` (`layout(location = 0) out
//! T out0[N]; ... out0[k] = var0_k;`). The front end holds an output array in
//! a shadow local, one slot per element, and stores each element once at the
//! end of `main` as `LoadLocal { comps < 4 }` into `StoreOutput` at
//! `location + k` (lower.rs `store_shadows`). The slot's lanes at or past the element's
//! width are never stored, so selection's `src` hands back the slot's home
//! register for them, which nothing writes; `StoreOutput` replaces those
//! lanes with lane 0 (`colour_components` of location + k: one declaration
//! per element) before anything reads them. So each element writes its own
//! lanes, and nothing of an unwritten register or of another element.
//!
//! What is checked, per shape: the build succeeds, the IR does take a narrow
//! shadow load into the colour output, and the built USC bytes, run through
//! the host byte model (tests/usc_exec `run_binary`), leave in pixel output k
//! element k's own value in its declared lanes and lane 0 in the rest. The
//! inputs are marked: every coefficient holds its own index (a float input
//! as index / 255, which the RGBA8 variant packs back as the byte), and the
//! iterator lays the varyings out one after another, component c of a
//! varying 4 coefficients after component c - 1. Variants: RGBA8 (the
//! default, dEQP's `fixed`), RGBA16F (`SHC_RT_F16X4`, `float`) and the
//! integer ones (`shc_blend_key_int_rt`). Several elements are checked under
//! the RGBA8 variant only: the per-target layouts of more colour targets are
//! RC14's. HOST-ONLY: no GPU arithmetic and no GL read-back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_fragout_array
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::ir;
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, shc_blend_key_rt, ShcIntRt, SHC_BLEND_KEY_REPLACE, SHC_RT_F16X4};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What one fragment shader built to.
struct Rec {
    /// Pixel output dwords.
    num_outputs: u8,
    image: Vec<u8>,
    /// The width of each shadow load (`LoadLocal`) stored to a colour
    /// output, in program order.
    shadow_loads: Vec<u8>,
}

/// One fragment shader through the chain under the variant `key`, or the
/// refusal.
fn compile_for(src: &str, key: u32) -> Result<Rec, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let shadow_loads = l
            .module
            .instrs
            .iter()
            .filter_map(|op| match *op {
                ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(_), src } => match l.module.def(src.val) {
                    Some(&ir::Op::LoadLocal { comps, .. }) => Some(comps),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok(Rec {
            num_outputs: rec.num_outputs,
            image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
            shadow_loads,
        })
    })
    .unwrap()
}

/// `src` under `key`, run with coefficient i holding `coeff(i)`: the record
/// and the pixel output dwords, in order.
fn run(src: &str, key: u32, coeff: impl Fn(usize) -> u32) -> (Rec, Vec<u32>) {
    let rec = compile_for(src, key).unwrap_or_else(|e| panic!("{src}: {e}"));
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    for (i, c) in inp.coeff.iter_mut().enumerate() {
        *c = coeff(i);
    }
    let out = usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"));
    let px = (0..u32::from(rec.num_outputs))
        .map(|k| *out.get(&k).unwrap_or_else(|| panic!("{src}: nothing in pixel output {k}: {out:?}")))
        .collect();
    (rec, px)
}

/// A float input marked with its coefficient's index: index / 255.
fn float_mark(i: usize) -> u32 {
    ((i % 256) as f32 / 255.0).to_bits()
}

/// An integer input marked with its coefficient's index.
fn int_mark(i: usize) -> u32 {
    100 + i as u32
}

/// dEQP's fragment shader of an array case (`es3fFragmentOutputTests.cpp`
/// `createProgram`): one input `var0_k` per element, `out0[k] = var0_k`.
fn array_case(prec: &str, ty: &str, n: usize) -> String {
    let float = !ty.starts_with('i') && !ty.starts_with('u');
    let (interp, in_prec) = if float { ("smooth", "highp") } else { ("flat", prec) };
    let mut s = String::from("#version 300 es\n");
    for k in 0..n {
        s += &format!("{interp} in {in_prec} {ty} var0_{k};\n");
    }
    s += &format!("layout(location = 0) out {prec} {ty} out0[{n}];\n\nvoid main()\n{{\n");
    for k in 0..n {
        s += &format!("\tout0[{k}] = var0_{k};\n");
    }
    s + "}\n"
}

/// The components of a GLSL type.
fn width(ty: &str) -> usize {
    ty.chars().last().and_then(|c| c.to_digit(10)).map_or(1, |d| d as usize)
}

/// The four RGBA8 lanes of a pixel output dword.
fn bytes(d: u32) -> [u32; 4] {
    d.to_le_bytes().map(u32::from)
}

/// The lanes element `k` of an array of `w`-wide elements must leave when
/// its first input's first coefficient is `base`: its own coefficients
/// `base + 4 * (w * k + c)` in the declared lanes, lane 0 in the others.
fn want_lanes(base: u32, w: usize, k: usize) -> [u32; 4] {
    let own = |c: usize| base + 4 * (w * k + c) as u32;
    core::array::from_fn(|c| own(if c < w { c } else { 0 }))
}

/// dEQP's `array.fixed` shape, one and three elements of `float` to `vec4`
/// under the RGBA8 variant: element k in pixel output k, its own lanes, lane
/// 0 past its width -- never the unwritten lanes of its slot nor the next
/// element's input.
#[test]
fn fixed_arrays_write_each_element_in_its_own_lanes() {
    for ty in ["float", "vec2", "vec3", "vec4"] {
        for n in [1, 3] {
            let src = array_case("mediump", ty, n);
            let (rec, px) = run(&src, 0, float_mark);
            let w = width(ty);
            assert_eq!(rec.shadow_loads, vec![w as u8; n], "{ty}[{n}]: each element a shadow load of its width");
            assert_eq!(px.len(), n, "{ty}[{n}]: one RGBA8 dword per element");
            let base = bytes(px[0])[0];
            for (k, &d) in px.iter().enumerate() {
                assert_eq!(bytes(d), want_lanes(base, w, k), "{ty}[{n}] element {k}: {src}");
            }
        }
    }
}

/// dEQP's `array.float` shape, one element under the RGBA16F variant:
/// halves, R and G in dword 0, B and A in dword 1.
#[test]
fn half_float_array_writes_its_own_lanes() {
    let key = SHC_BLEND_KEY_REPLACE | shc_blend_key_rt(SHC_RT_F16X4);
    for ty in ["float", "vec2", "vec3"] {
        let src = array_case("mediump", ty, 1);
        let (rec, px) = run(&src, key, float_mark);
        assert_eq!(rec.shadow_loads, vec![width(ty) as u8], "{ty}[1]: a narrow shadow load");
        assert_eq!(px.len(), 2, "{ty}[1]: two dwords of halves");
        let lanes: Vec<u32> = px
            .iter()
            .flat_map(|d| [d & 0xffff, d >> 16])
            .map(|h| (usc_exec::f16_to_f32(h as u16) * 255.0).round() as u32)
            .collect();
        assert_eq!(lanes, want_lanes(lanes[0], width(ty), 0), "{ty}[1]: {src}");
    }
}

/// dEQP's `array.int` / `array.uint` shapes, one element: the integer
/// variant of the element's width at 32 bits (a `uvec3` renders to a
/// four-channel target), and an `ivec2` at 16 and 8 bits a channel.
#[test]
fn integer_arrays_write_their_own_lanes() {
    for (ty, signed) in [("int", true), ("ivec2", true), ("uint", false), ("uvec2", false), ("uvec3", false)] {
        let w = width(ty);
        let channels = if w == 3 { 4 } else { w };
        let src = array_case("highp", ty, 1);
        let key = shc_blend_key_int_rt(ShcIntRt { channels: channels as u8, bits: 32, signed });
        let (rec, px) = run(&src, key, int_mark);
        assert_eq!(rec.shadow_loads, vec![w as u8], "{ty}[1]: a narrow shadow load");
        assert_eq!(px, want_lanes(px[0], w, 0)[..channels], "{ty}[1] at 32 bits: {src}");
    }
    for (prec, bits) in [("mediump", 16u32), ("lowp", 8)] {
        let src = array_case(prec, "ivec2", 1);
        let key = shc_blend_key_int_rt(ShcIntRt { channels: 2, bits: bits as u8, signed: true });
        let (_, px) = run(&src, key, int_mark);
        assert_eq!(px.len(), 1, "ivec2[1] at {bits} bits: one dword");
        let (r, g) = (px[0] & ((1 << bits) - 1), px[0] >> bits);
        assert_eq!(g, r + 4, "ivec2[1] at {bits} bits: G is the input after R's: {src}");
    }
}

/// The shadow slot written other ways than whole: a conditional store, a
/// component at a time, read back and modified, a scalar of an ALU result,
/// a loop index. Each still writes the element's own lanes and lane 0 past
/// its width. The one input `a` is dEQP's `var0_0`: its x at coefficient 4
/// (4 / 255 by the mark), its y at 8.
#[test]
fn shadowed_writes_other_than_whole_keep_their_lanes() {
    let (_, p) = run(&array_case("mediump", "vec2", 1), 0, float_mark);
    assert_eq!(bytes(p[0])[..2], [4, 8], "the first input's coefficients");
    let head = "#version 300 es\nprecision highp float;\nin highp vec2 a;\n";
    let cases: [(&str, &str, [u32; 4]); 4] = [
        /* a.x is 4 / 255, so the else arm runs */
        (
            "layout(location = 0) out mediump vec2 o[1];\n\
             void main() { if (a.x > 0.5) o[0] = a; else o[0] = a.yx; }",
            "the conditional store",
            [8, 4, 8, 8],
        ),
        (
            "layout(location = 0) out mediump vec2 o[1];\nvoid main() { o[0].x = a.y; o[0].y = a.x; }",
            "a component at a time",
            [8, 4, 8, 8],
        ),
        (
            "layout(location = 0) out mediump vec2 o[1];\nvoid main() { o[0] = a; o[0].y = o[0].y * 2.0; }",
            "read back and modified",
            [4, 16, 4, 4],
        ),
        (
            "layout(location = 0) out mediump float o[1];\nvoid main() { o[0] = a.y * 2.0; }",
            "a scalar of an ALU result",
            [16, 16, 16, 16],
        ),
    ];
    for (body, what, want) in cases {
        let src = format!("{head}{body}\n");
        let (rec, px) = run(&src, 0, float_mark);
        assert!(
            !rec.shadow_loads.is_empty() && rec.shadow_loads.iter().all(|&c| c < 4),
            "{what}: a narrow shadow load: {:?}",
            rec.shadow_loads
        );
        assert_eq!(bytes(px[0]), want, "{what}: {src}");
    }
    let looped = format!(
        "{head}layout(location = 0) out mediump vec3 o[2];\n\
         void main() {{ for (int i = 0; i < 2; i++) o[i] = vec3(a, a.x) * float(i + 1); }}\n"
    );
    let (rec, px) = run(&looped, 0, float_mark);
    assert_eq!(rec.shadow_loads, [3, 3], "a loop index: two vec3 shadow loads");
    assert_eq!(px.iter().map(|&d| bytes(d)).collect::<Vec<_>>(), [[4, 8, 4, 4], [8, 16, 8, 8]], "a loop index");
}
