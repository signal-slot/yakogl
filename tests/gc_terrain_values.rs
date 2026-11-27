// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC host interpreter's outputs for glmark2 terrain's shaders, pinned
//! (plan #421, item values-guard). A compiler change that makes the shaders
//! shorter must leave what they compute alone: each text is compiled with the
//! GC7000 online compiler (selection, dead-code removal, definition sinking,
//! copy coalescing, allocation, as `lane_move_equivalence.rs` does), run by
//! `tests/common` `run_mem` on 8 seeded pseudo-random input sets, and a
//! sha256 of every output register's meaningful lanes (and the discard flag)
//! is compared with the constant table below. The table is today's values; a
//! later compiler item keeps it unchanged, and changes it only for a change
//! of value that it can justify.
//!
//! Inputs do not depend on the register layout the compiler picks: varyings
//! and attributes are drawn in declaration order, uniform dwords by their
//! index in the uniform block, and the constant file is filled from the
//! compiler's own slots (a literal as its bits, a uniform from the block).
//! Floats are in [0, 1] for varyings (texture coordinates, a normalised
//! position) and [-2, 2] for attributes and uniforms; an integer or bool
//! uniform is 0..=3. A texture fetch is the interpreter's fixed function of
//! sampler and coordinates, so every shader runs; none is listed as not run.
//!
//! The glmark2 texts are GPL-3.0 and never committed: they are built from a
//! glmark2 checkout (`GLMARK2_SRC`, its `data/shaders`) exactly as
//! `gc_terrain_shader_count.rs` builds them. Without it the test prints a
//! SKIP line. The eleven: noise, terrain.vert, terrain.frag, tilt_h,
//! tilt_v, bloom_h, bloom_v, normalmap, overlay, quad.vert, luminance.
//!
//! ```text
//! GLMARK2_SRC=... cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_terrain_values -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use std::path::Path;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::vivante::alloc::{self, Stage};
use yakogl::shaderc::online::vivante::isa::{Instr, MAX_FIELDS};
use yakogl::shaderc::online::{lower, select, sema};
use yakogl::shaderc::sha256::sha256;
use yakogl::shaderc::{SHC_VIV_CONST_LITERAL, SHC_VIV_CONST_UNIFORM};

mod common;
use common::{run_mem, Rng, Run};

/// `libmatrix/shader-source.cc`'s two precision macros.
const MACROS: &str =
    "#if defined(GL_ES) && defined(GL_FRAGMENT_PRECISION_HIGH)\n#define HIGHP_OR_DEFAULT highp\n#else\n\
#define HIGHP_OR_DEFAULT\n#endif\n#if defined(GL_ES)\n#define MEDIUMP_OR_DEFAULT mediump\n#else\n\
#define MEDIUMP_OR_DEFAULT\n#endif\n";
/// The default fragment precision `ShaderSource` adds.
const FPREC: &str = "#ifdef GL_ES\nprecision mediump float;\n#endif\n";

/// `ShaderSource::add_const` of a float (`%f`).
fn konst(name: &str, v: f64) -> String {
    format!("const float {name} = {v:.6};\n")
}

/// `blur-renderer.cpp`'s `$CONVOLUTION$` text for one pass, in `terrain-blur.frag`.
fn blur(file: &str, radius: i32, sigma: f64, horizontal: bool, step: f64, tilt: f64) -> String {
    let sigma = sigma.max(1.0);
    let s2 = 2.0 * sigma * sigma;
    let vals: Vec<f64> =
        (0..=radius).map(|i| 1.0 / (core::f64::consts::PI * s2).sqrt() * (-f64::from(i * i) / s2).exp()).collect();
    let sum: f64 = 2.0 * vals.iter().sum::<f64>() - vals[0];
    let mut c: String = vals.iter().enumerate().map(|(i, k)| konst(&format!("Kernel{i}"), k / sum)).collect();
    c += &konst("TiltShift", tilt);
    let ax = if horizontal { "X" } else { "Y" };
    c += &konst(&format!("TextureStep{ax}"), step);
    let mut ss = format!(
        "float step{ax} = TextureStep{ax} * abs(TiltShift - TextureCoord.y) / abs(1.0 - TiltShift);\nresult = \n"
    );
    for o in -radius..=radius {
        let v = if horizontal { format!("vec2({o}.0 * stepX, 0.0)") } else { format!("vec2(0.0, {o}.0 * stepY)") };
        ss += &format!("texture2D(Texture0, TextureCoord + {v}) * Kernel{} +\n", o.abs());
    }
    ss += "0.0 ;\n";
    format!("{MACROS}{FPREC}{c}{}", file.replace("$CONVOLUTION$", &ss))
}

/// The eleven texts from a glmark2 checkout: the noise height map
/// (`simplex-noise-renderer.cpp`: `terrain-noise.frag`, no added constants),
/// then the ten of `gc_terrain_shader_count.rs`'s table in its order.
fn glmark2_texts(root: &Path) -> Vec<String> {
    let rd = |n: &str| {
        std::fs::read_to_string(root.join("data/shaders").join(n))
            .unwrap_or_else(|e| panic!("{n} in {}: {e}", root.display()))
    };
    let b = rd("terrain-blur.frag");
    vec![
        format!("{MACROS}{FPREC}{}", rd("terrain-noise.frag")),
        format!("{MACROS}{}", rd("terrain.vert")),
        format!("{MACROS}{FPREC}{}", rd("terrain.frag")),
        // scene-terrain.cpp:84-121: tilt-shift radius 4, sigma 2.7, tilt 0.5; bloom radius 2, sigma 4
        blur(&b, 4, 2.7, true, 1.0 / 800.0, 0.5),
        blur(&b, 4, 2.7, false, 1.0 / 600.0, 0.5),
        blur(&b, 2, 4.0, true, 1.0 / 800.0, 0.0),
        blur(&b, 2, 4.0, false, 1.0 / 256.0, 0.0),
        format!("{MACROS}{FPREC}{}", rd("terrain-normalmap.frag")),
        format!("{MACROS}{FPREC}{}", rd("terrain-overlay.frag")),
        format!("{MACROS}{}", rd("terrain-texture.vert")),
        format!("{MACROS}{FPREC}{}", rd("terrain-luminance.frag")),
    ]
}

/// Input sets per shader.
const SETS: usize = 8;

/// Name, stage and the pinned hash (the first 16 hex digits of the sha256 over
/// the eight runs' outputs) of each text, in `glmark2_texts`' order.
const PINS: [(&str, sema::Stage, &str); 11] = [
    ("noise.frag", sema::Stage::Fragment, "c12413ad22c40a48"),
    ("terrain.vert", sema::Stage::Vertex, "61731d9ebc22d5d0"),
    ("terrain.frag", sema::Stage::Fragment, "320812cb3e6522c0"),
    ("tilt_h.frag", sema::Stage::Fragment, "2db3e638b0b5a805"),
    ("tilt_v.frag", sema::Stage::Fragment, "ce6ef0fb579a3c9f"),
    ("bloom_h.frag", sema::Stage::Fragment, "fc8bfa45912f2144"),
    ("bloom_v.frag", sema::Stage::Fragment, "a21b8f75e6254f8f"),
    ("normalmap.frag", sema::Stage::Fragment, "820aa694a05a707d"),
    ("overlay.frag", sema::Stage::Fragment, "18ab994cb54a556e"),
    ("quad.vert", sema::Stage::Vertex, "39732128aa7b7d3c"),
    ("luminance.frag", sema::Stage::Fragment, "e96104589211db31"),
];

/// A float of `[lo, hi]` from the generator's `[-2, 2]`.
fn scaled(r: &mut Rng, lo: f32, hi: f32) -> f32 {
    lo + (r.float() + 2.0) / 4.0 * (hi - lo)
}

/// The sha256 digest's first 8 bytes in hex, over every set's outputs.
fn outputs_hash(stage: sema::Stage, text: &str) -> String {
    let mut mem = vec![0u8; 8 << 20];
    let a = Arena::new(&mut mem);
    let text = a.alloc_str(text).expect("arena");
    let l = lower::compile_to_ir(&a, stage, text).expect("lowers");
    let sel = select::select(&l.module, &a).expect("selects");
    let mut code = sel.instrs.to_vec();
    let mut back_edges = sel.back_edges.to_vec();
    let dead = alloc::eliminate_dead(&Arena::new(&mut vec![0u8; 1 << 20]), &mut code, &mut back_edges).unwrap();
    code.truncate(code.len() - dead);
    alloc::sink_definitions(&Arena::new(&mut vec![0u8; 1 << 20]), &mut code).unwrap();
    let removed = alloc::coalesce_copies(&Arena::new(&mut vec![0u8; 1 << 20]), &mut code).unwrap();
    code.truncate(code.len() - removed);
    let outputs: Vec<(u8, u8)> = sel
        .io_out
        .iter()
        .map(|io| (io.reg, (1u8 << io.components.min(4)) - 1))
        .chain(sel.vs_pos_out_reg.map(|r| (r, 0xf)))
        .chain(sel.vs_pointsize_out_reg.map(|r| (r, 0x1)))
        .chain(sel.ps_color_out_reg.map(|r| (r, 0xf)))
        .collect();
    let reserved: Vec<u8> = outputs.iter().map(|o| o.0).collect();
    let io_in: Vec<u8> = sel.io_in.iter().map(|io| io.reg).collect();
    let frag = stage == sema::Stage::Fragment;
    let writes = |r: u8| code.iter().any(|s| matches!(s.dst, Some(alloc::SelDst::Fixed { reg, .. }) if reg == r));
    let placeable: Vec<u8> = reserved.iter().copied().filter(|r| writes(*r)).collect();
    let fixed: Vec<u8> = reserved.iter().copied().filter(|r| !writes(*r)).collect();
    let mut out = vec![Instr { op: 0, vals: [0; MAX_FIELDS], dont_care: 0 }; code.len()];
    let mut placed = [alloc::NOT_PLACED; alloc::MAX_TEMPS as usize];
    alloc::allocate_and_build_placed(
        &Arena::new(&mut vec![0u8; 1 << 20]),
        &code,
        &mut out,
        if frag { Stage::Fragment } else { Stage::Vertex },
        fixed,
        io_in.iter().copied(),
        &placeable,
        &mut placed,
    )
    .expect("allocates");
    let outs: Vec<(u8, u8)> = outputs
        .iter()
        .map(|&(reg, lanes)| {
            (placed.get(reg as usize).copied().filter(|x| *x != alloc::NOT_PLACED).unwrap_or(reg), lanes)
        })
        .collect();

    /* Which dwords of the uniform block hold an integer or bool. */
    let mut int_dwords = vec![false; l.module.uniform_dwords as usize];
    for u in l
        .module
        .uniforms
        .iter()
        .filter(|u| matches!(u.gl_type, 0x1404 | 0x1405 | 0x8b53..=0x8b59 | 0x8dc6..=0x8dc8 | 0x8b5e..=0x8b64))
    {
        let (s, e) = (u.dword_off as usize, u.dword_off as usize + u.dwords as usize);
        for d in int_dwords.iter_mut().take(e).skip(s) {
            *d = true;
        }
    }

    let mut bytes = Vec::new();
    for set in 0..SETS {
        let seed = 0x9e37_79b9_7f4a_7c15u64 ^ (set as u64 + 1).wrapping_mul(0x2545_f491_4f6c_dd1d);
        /* A uniform dword's value depends on its index in the block only. */
        let uval = |d: usize| -> u32 {
            let mut r = Rng(seed ^ (d as u64 + 1).wrapping_mul(0xff51_afd7_ed55_8ccd));
            for _ in 0..3 {
                r.next();
            }
            if int_dwords.get(d) == Some(&true) {
                r.next() % 4
            } else {
                r.float().to_bits()
            }
        };
        let mut uniforms = vec![[0u32; 4]; sel.consts.len().div_ceil(4).max(1)];
        for (i, c) in sel.consts.iter().enumerate() {
            let v = match c.contents {
                SHC_VIV_CONST_LITERAL => c.data,
                SHC_VIV_CONST_UNIFORM => uval(c.data as usize),
                k if k == yakogl::shaderc::SHC_VIV_CONST_UNUSED => 0,
                k => panic!("constant kind {k} is not modelled by this test"),
            };
            uniforms[i / 4][i % 4] = v;
        }
        let mut rng = Rng(seed);
        let mut init = vec![[0u32; 4]; 128];
        if frag {
            init[0] = core::array::from_fn(|_| scaled(&mut rng, 0.0, 1.0).to_bits());
        }
        for &r in &io_in {
            init[r as usize] = core::array::from_fn(|_| {
                if frag { scaled(&mut rng, 0.0, 1.0) } else { scaled(&mut rng, -2.0, 2.0) }.to_bits()
            });
        }
        match run_mem(&out, &init, &uniforms, &[]) {
            Run::Done { regs, killed } => {
                bytes.push(killed as u8);
                if !killed {
                    for &(o, lanes) in &outs {
                        for (l, v) in regs[o as usize].iter().enumerate() {
                            if lanes & (1 << l) != 0 {
                                bytes.extend_from_slice(&v.to_le_bytes());
                            }
                        }
                    }
                }
            }
            Run::Unsupported(why) => panic!("set {set}: the interpreter cannot run it: {why}"),
        }
    }
    sha256(&bytes)[..8].iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn terrain_shaders_values_are_pinned() {
    let Some(root) = std::env::var_os("GLMARK2_SRC") else {
        eprintln!("SKIP gc_terrain_values: GLMARK2_SRC (a glmark2 checkout) is not set");
        return;
    };
    let mut wrong = Vec::new();
    for ((name, stage, want), text) in PINS.iter().zip(glmark2_texts(Path::new(&root))) {
        let got = outputs_hash(*stage, &text);
        println!("gc7000 {name}: {SETS} input sets, outputs {got} (pinned {want})");
        if got != *want {
            wrong.push(format!("{name}: {got}, pinned {want}"));
        }
    }
    assert!(wrong.is_empty(), "the interpreter's outputs moved:\n{}", wrong.join("\n"));
}
