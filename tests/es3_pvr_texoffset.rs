// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Constant texel offsets on the PowerVR online back end beyond the 2D
//! image (dEQP-GLES3 shaders.texture_functions: textureOffset,
//! textureLodOffset, textureProjOffset, textureProjLodOffset and
//! texelFetchOffset): on a 2D array kept as the stacked image
//! (`pvr_native_array` off) and as the TPU's own (on), and on a 3D texture
//! kept as the grid of slices (`pvr_native_3d` off) and as the TPU's own
//! (on), in a vertex and a fragment shader.
//!
//! As Mesa's pco builds them (pco_nir_tex.c:306-322): a sample's offset is
//! the lookup word at the end of the data block, `soo` set, u in bits 0..5,
//! v in 6..11 and, on a native 3D texture, w in 12..15. The grid of slices
//! takes w as whole slices in the shader; a texelFetchOffset adds its
//! offset to the integer coordinate (texelFetch(P + offset), ES 3.00 8.8)
//! and has no lookup word.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_texoffset
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::{BTreeMap, BTreeSet};
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::{dsts, srcs};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How a sampler is kept: the two switches the look-up arms read.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Layout {
    Stacked,
    Native,
}

/// The two switches for the closure, and back at their defaults after it,
/// panic or not.
fn with_layout<T>(layout: Layout, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _back = Back;
    let on = i32::from(layout == Layout::Native);
    yakogl::shaderc::pvr_set_native_array(on);
    yakogl::shaderc::pvr_set_native_3d(on);
    f()
}

/// What one compiled program says: its `smp`s' fields, the data block of
/// its first `smp`, and every 32-bit immediate it moves.
#[derive(Debug)]
struct Prog {
    smps: Vec<BTreeMap<&'static str, u32>>,
    block: Vec<Option<u32>>,
    imms: BTreeSet<u32>,
    ops: BTreeMap<&'static str, usize>,
}

/// One shader through the online PowerVR chain, or the refusal.
fn compile(stage: Stage, layout: Layout, src: &str) -> Result<Prog, String> {
    with_layout(layout, || {
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
            let rec = if stage == Stage::Fragment {
                compile_fragment(&l.module, &l.reflection, a)
            } else {
                compile_vertex(&l.module, &l.reflection, a)
            }
            .map_err(|e| format!("chain: {e:?}"))?;
            // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            Ok(decode(&image))
        })
        .unwrap()
    })
}

/// The program's samples, the first sample's data block -- for each of its
/// temporaries (from the one `smp` names as its data operand), the last
/// write before the sample, `Some(v)` for an immediate move of `v`, `None`
/// for a computed value -- and the immediates it moves.
fn decode(image: &[u8]) -> Prog {
    const NAMES: [&str; 6] = ["dmn", "lodm", "pplod", "nncoords", "soo", "tao"];
    let mut off = 0usize;
    let mut writes: Vec<(u32, Option<u32>)> = Vec::new();
    let mut prog = Prog { smps: Vec::new(), block: Vec::new(), imms: BTreeSet::new(), ops: BTreeMap::new() };
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            let names: Vec<&'static str> = ops_for(r, ph).map(|o| o.name).collect();
            if let Some(n) = names.first() {
                *prog.ops.entry(n).or_default() += 1;
            }
            if names.contains(&"smp") {
                if prog.smps.is_empty() {
                    let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
                    let data = lo[1].expect("the smp's data operand");
                    for k in 0..16u32 {
                        let Some((_, w)) = writes.iter().rev().find(|(t, _)| *t == data.index + k) else { break };
                        prog.block.push(*w);
                    }
                }
                prog.smps.push(NAMES.iter().map(|n| (*n, r.get(n).unwrap_or(0))).collect());
                continue;
            }
            let imm = if names.contains(&"bbyp0bm_imm32") { r.get("imm32") } else { None };
            if let Some(v) = imm {
                prog.imms.insert(v);
            }
            let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
            for o in d.into_iter().flatten() {
                /* temp is bank 1 (operands.rs) */
                if o.bank == 1 {
                    writes.push((o.index, imm));
                }
            }
        }
        off += g.total;
    }
    prog
}

/// `stage`'s shader around `decl` (the sampler) and the look-up `e`, with
/// a coordinate `c`, a projective one `q`, a level `l`, an integer one
/// `p` and an integer level `il` from uniforms.
fn shader(stage: Stage, decl: &str, e: &str) -> String {
    let head = "#version 300 es\nprecision highp float;\nprecision highp int;\n";
    let uniforms = "uniform vec3 c;\nuniform vec4 q;\nuniform float l;\nuniform ivec3 p;\nuniform int il;\n";
    if stage == Stage::Fragment {
        format!("{head}{decl}\n{uniforms}out vec4 o;\nvoid main() {{ o = vec4({e}); }}\n")
    } else {
        format!("{head}{decl}\n{uniforms}in vec4 a;\nout vec4 v;\nvoid main() {{ gl_Position = a; v = vec4({e}); }}\n")
    }
}

/// The lookup word pco packs (pco_nir_tex.c:306-322, rogue_hw_defs.h
/// ROGUE_SMP_LOOKUP_*): u in bits 0..5, v in 6..11, w in 12..15, each its
/// two's complement truncated to the field.
fn word(du: i8, dv: i8, dw: i8) -> u32 {
    (du as u32 & 0x3f) | ((dv as u32 & 0x3f) << 6) | ((dw as u32 & 0xf) << 12)
}

/// A sample with an offset: (sampler, look-up, the offset the lookup word
/// carries -- w only on a native 3D texture -- and the data block's words
/// before the lookup word in a fragment shader, per layout: (stacked,
/// native)). A stacked image's or the grid's sample is a 2D one -- u, v,
/// the level word if any; a native array's adds a level word always (a 0.0
/// bias without one) and the two address words; a native 3D one has w
/// after v.
const SAMPLES: &[(&str, &str, [i8; 3], [usize; 2])] = &[
    ("sampler2DArray", "textureOffset(s, c, ivec2(3, -2))", [3, -2, 0], [2, 5]),
    ("sampler2DArray", "textureLodOffset(s, c, l, ivec2(-8, 7))", [-8, 7, 0], [3, 5]),
    ("isampler2DArray", "textureOffset(s, c, ivec2(1, 5))", [1, 5, 0], [2, 5]),
    ("usampler2DArray", "textureLodOffset(s, c, l, ivec2(-3, 4))", [-3, 4, 0], [3, 5]),
    ("sampler3D", "textureOffset(s, c, ivec3(3, -2, 5))", [3, -2, 5], [2, 3]),
    ("sampler3D", "textureLodOffset(s, c, l, ivec3(-8, 7, -8))", [-8, 7, -8], [3, 4]),
    ("sampler3D", "textureProjOffset(s, q, ivec3(1, 2, -3))", [1, 2, -3], [2, 3]),
    ("sampler3D", "textureProjLodOffset(s, q, l, ivec3(-1, 6, 7))", [-1, 6, 7], [3, 4]),
    ("isampler3D", "textureOffset(s, c, ivec3(2, 2, 1))", [2, 2, 1], [2, 3]),
    ("usampler3D", "textureProjLodOffset(s, q, l, ivec3(-2, 3, 4))", [-2, 3, 4], [3, 4]),
];

/// The bias forms, which ES 3.00 allows in a fragment shader only.
const BIASED: &[(&str, &str, [i8; 3], [usize; 2])] = &[
    ("sampler2DArray", "textureOffset(s, c, ivec2(3, -2), l)", [3, -2, 0], [3, 5]),
    ("sampler3D", "textureOffset(s, c, ivec3(-4, 1, 2), l)", [-4, 1, 2], [3, 4]),
    ("sampler3D", "textureProjOffset(s, q, ivec3(4, -1, -2), l)", [4, -1, -2], [3, 4]),
];

/// texelFetchOffset: (sampler, look-up with an offset, the same without).
const FETCHES: &[(&str, &str, &str)] = &[
    ("sampler2D", "texelFetchOffset(s, p.xy, il, ivec2(5, -3))", "texelFetch(s, p.xy, il)"),
    ("isampler2D", "texelFetchOffset(s, p.xy, il, ivec2(5, -3))", "texelFetch(s, p.xy, il)"),
    ("sampler2DArray", "texelFetchOffset(s, p, il, ivec2(5, -3))", "texelFetch(s, p, il)"),
    ("usampler2DArray", "texelFetchOffset(s, p, il, ivec2(5, -3))", "texelFetch(s, p, il)"),
    ("sampler3D", "texelFetchOffset(s, p, il, ivec3(5, -3, 6))", "texelFetch(s, p, il)"),
    ("isampler3D", "texelFetchOffset(s, p, il, ivec3(5, -3, 6))", "texelFetch(s, p, il)"),
];

fn decl(sampler: &str) -> String {
    format!("uniform highp {sampler} s;")
}

/// Every sample with an offset compiles in both stages and both layouts,
/// each `smp` with `soo` set; in a fragment shader the lookup word is the
/// immediate right after the block's other words, with w on a native 3D
/// texture only; in a vertex shader it is in the block.
#[test]
fn every_offset_sample_ends_its_block_with_the_lookup_word() {
    let rows = SAMPLES.iter().map(|r| (r, true)).chain(BIASED.iter().map(|r| (r, false)));
    for ((sampler, e, [du, dv, dw], before), vertex_too) in rows {
        for layout in [Layout::Stacked, Layout::Native] {
            let vol = sampler.ends_with("3D");
            let want = if vol && layout == Layout::Native { word(*du, *dv, *dw) } else { word(*du, *dv, 0) };
            let at = before[usize::from(layout == Layout::Native)];
            let stages: &[Stage] = if vertex_too { &[Stage::Fragment, Stage::Vertex] } else { &[Stage::Fragment] };
            for &stage in stages {
                let what = format!("{e} on {sampler}, {layout:?}, {stage:?}");
                let p =
                    compile(stage, layout, &shader(stage, &decl(sampler), e)).unwrap_or_else(|m| panic!("{what}: {m}"));
                assert!(!p.smps.is_empty(), "{what}: no smp");
                assert!(p.smps.iter().all(|s| s["soo"] == 1), "{what}: every smp has soo: {:?}", p.smps);
                if stage == Stage::Fragment {
                    assert_eq!(p.block.get(at), Some(&Some(want)), "{what}: word {at} of {:x?}", p.block);
                } else {
                    assert!(p.block.contains(&Some(want)), "{what}: no lookup word {want:#x} in {:x?}", p.block);
                }
            }
        }
    }
}

/// The grid of slices samples two slices of a float texture, each with the
/// same lookup word (u and v); its w moves the slices in the shader, an
/// immediate `dw` the program without the offset does not have.
#[test]
fn the_grid_takes_w_as_whole_slices() {
    for stage in [Stage::Fragment, Stage::Vertex] {
        let with = "textureLodOffset(s, c, l, ivec3(3, -2, 5))";
        let without = "textureLod(s, c, l)";
        let p = compile(stage, Layout::Stacked, &shader(stage, &decl("sampler3D"), with)).unwrap();
        let q = compile(stage, Layout::Stacked, &shader(stage, &decl("sampler3D"), without)).unwrap();
        assert_eq!(p.smps.len(), 2, "{stage:?}: the two slices around c.z");
        assert!(p.smps.iter().all(|s| s["soo"] == 1 && s["tao"] == 0), "{stage:?}: {:?}", p.smps);
        assert!(q.smps.iter().all(|s| s["soo"] == 0), "{stage:?}: {:?}", q.smps);
        let five = 5.0f32.to_bits();
        assert!(p.imms.contains(&five) && !q.imms.contains(&five), "{stage:?}: w = 5 slices: {:x?}", p.imms);
        assert!(p.imms.contains(&word(3, -2, 0)), "{stage:?}: the lookup word has no w: {:x?}", p.imms);
    }
}

/// texelFetchOffset: the fetch's `smp` (`nncoords`, the level replaced),
/// with no lookup word (`soo` clear); each offset lane is one integer add
/// on the coordinate more than the same fetch without the offset has -- in
/// both stages and both layouts.
#[test]
fn texel_fetch_offset_adds_the_offset_to_the_coordinate() {
    for (sampler, with, without) in FETCHES {
        let vol = sampler.ends_with("3D");
        let lanes = if vol { 3 } else { 2 };
        for layout in [Layout::Stacked, Layout::Native] {
            for stage in [Stage::Fragment, Stage::Vertex] {
                let what = format!("{with} on {sampler}, {layout:?}, {stage:?}");
                let p = compile(stage, layout, &shader(stage, &decl(sampler), with))
                    .unwrap_or_else(|m| panic!("{what}: {m}"));
                let q = compile(stage, layout, &shader(stage, &decl(sampler), without))
                    .unwrap_or_else(|m| panic!("{what}: {m}"));
                assert_eq!(p.smps.len(), 1, "{what}: one fetch");
                let s = &p.smps[0];
                assert_eq!((s["soo"], s["nncoords"]), (0, 1), "{what}: {s:?}");
                assert_eq!(s["lodm"], q.smps[0]["lodm"], "{what}: the same level as the fetch without");
                /* one integer add per lane (`imadd32`, a small offset as
                 * its immediate source); -3 does not fit one and is moved */
                let adds = |p: &Prog| p.ops.get("imadd32").copied().unwrap_or(0);
                assert_eq!(adds(&p), adds(&q) + lanes, "{what}: {:?} against {:?}", p.ops, q.ops);
                let minus3 = (-3i32) as u32;
                assert!(p.imms.contains(&minus3) && !q.imms.contains(&minus3), "{what}: {:x?}", p.imms);
            }
        }
    }
}
