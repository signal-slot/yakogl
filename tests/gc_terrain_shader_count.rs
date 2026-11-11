// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 terrain's shaders other than the noise shader, and the
//! one-statement shaders that isolate the causes of their length, through the
//! GC7000 online compiler: the instruction and temporary counts pinned
//! (`docs/notes/gc-terrain-shaders-rest.md`, plan #328 item
//! terrain-compiler-fix). Mesa etnaviv's `etna_shc` counts for the GC7000UL
//! are the comments; they are the target, not a pin. The pins include
//! `gc-shadow-refract-shaders.md`'s "The patch" (a copy fold and a scalar lane
//! merge, `src/shaderc/online/vivante/peephole.rs`) and the GC7000's
//! constant-loop unroll (`select.rs` `unrolled`, the note's rank 1); the
//! comments give the counts before them.
//!
//! The glmark2 texts are GPL-3.0 and never committed: they are built here from
//! a glmark2 checkout (`GLMARK2_SRC`, its `data/shaders`) as glmark2's
//! `ShaderSource` and `blur-renderer.cpp` hand them to the driver (the note's
//! `gen_terrain.py`, the same text). With no `GLMARK2_SRC` the glmark2 half
//! prints a SKIP line. The micro-shaders are this repository's own and always
//! run.
//!
//! ```text
//! GLMARK2_SRC=... cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_terrain_shader_count -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use std::path::Path;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// Instructions and temporaries of one compile.
fn count(stage: u32, src: &str) -> (u32, u32) {
    yakogl::shaderc::online::backend::forget_compiled();
    let (mut sh, mut log) = (ShcShader::ZERO, [0 as c_char; 1024]);
    let text = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, text.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) };
    let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "compile failed: {msg}");
    (sh.usc_dw / 4, sh.temps.into())
}

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

/// (name, stage, ours: instructions and temps); etna_shc's and the count before the patch in the comment
const GLMARK2: [(&str, u32, u32, u32); 10] = [
    ("terrain.vert", SHC_STAGE_VERTEX, 48, 9),     // etna 39, 9; before 56, 9
    ("terrain.frag", SHC_STAGE_FRAGMENT, 81, 12),  // etna 72, 11; before 127, 16 (sr patch), 148, 19
    ("tilt_h.frag", SHC_STAGE_FRAGMENT, 57, 12),   // etna 46, 5; before 66, 12; 62 before rank 4
    ("tilt_v.frag", SHC_STAGE_FRAGMENT, 57, 12),   // etna 46, 5; before 75, 12; 71 before rank T, 62 before rank 4
    ("bloom_h.frag", SHC_STAGE_FRAGMENT, 32, 8),   // etna 24, 5; before 42, 8; 38 before rank 4
    ("bloom_v.frag", SHC_STAGE_FRAGMENT, 32, 8),   // etna 24, 5; before 47, 8; 43 before rank T, 38 before rank 4
    ("normalmap.frag", SHC_STAGE_FRAGMENT, 20, 6), // etna 21, 4; before 25, 5; 23, 5 before rank T, 21 before rank 4
    ("overlay.frag", SHC_STAGE_FRAGMENT, 2, 2),    // etna 2, 2; before 2, 2
    ("quad.vert", SHC_STAGE_VERTEX, 7, 3),         // etna 4, 2; before 7, 3
    ("luminance.frag", SHC_STAGE_FRAGMENT, 7, 4),  // etna 3, 3; before 7, 4
];
/// The ten's instructions: etna_shc 279; 366 before rank 4 (float constant
/// folding), 382 before rank T (the retarget past a constant lane), 428
/// before the unroll, 475 before the patch.
const GLMARK2_TOTAL: u32 = 343;

/// The ten texts from a glmark2 checkout, in [`GLMARK2`]'s order.
fn glmark2_texts(root: &Path) -> Vec<String> {
    let rd = |n: &str| {
        std::fs::read_to_string(root.join("data/shaders").join(n))
            .unwrap_or_else(|e| panic!("{n} in {}: {e}", root.display()))
    };
    let b = rd("terrain-blur.frag");
    vec![
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

const HEAD: &str = "precision mediump float;\nvarying vec4 a;\nvarying vec4 b;\nuniform sampler2D T;\n";

/// The note's causes, one statement each (fragment stage), the counts ours. Not
/// compiled with `etna_shc`: the comment says what etnaviv emits for the same
/// statement in the note's glmark2 listings.
fn micro() -> Vec<(&'static str, u32, String, u32)> {
    let fs = |decl: &str, body: &str| format!("{HEAD}{decl}void main() {{ {body} }}\n");
    vec![
        // t1, rank 1: a one-iteration loop over a uniform array; etnaviv unrolls it
        // and reads the constant file, as select.rs `unrolled` now does; 19
        // before the unroll, 23 before the patch
        (
            "t1",
            SHC_STAGE_FRAGMENT,
            fs("uniform vec4 P[1];\n", "vec4 c = a; for (int i = 0; i < 1; i++) { c += P[i]; } gl_FragColor = c;"),
            1,
        ),
        // t3, rank 3: one blur tap's `* Kernel +` (texld, mul, add); etnaviv texld, mad; before the patch 3
        ("t3", SHC_STAGE_FRAGMENT, fs("", "gl_FragColor = texture2D(T, a.xy) * 0.25 + b;"), 3),
        // t4, rank 4: tilt-shift's constant divisor; etnaviv folds it (add, mul), as select.rs
        // `fold_constants` now does; before the patch 9, 5 before rank 4
        (
            "t4",
            SHC_STAGE_FRAGMENT,
            fs("const float S = 0.5;\n", "gl_FragColor = vec4(abs(S - a.y) / abs(1.0 - S));"),
            2,
        ),
        // t5, rank 5: two taps' offsets from one scalar; etnaviv one vec2 mul for both; before the patch 8,
        // 8 before rank T (the first sum's store into the vec4 is retargeted past the second tap)
        (
            "t5",
            SHC_STAGE_FRAGMENT,
            fs("", "gl_FragColor = vec4(a.xy + vec2(-1.0 * b.x, 0.0), a.xy + vec2(1.0 * b.x, 0.0));"),
            7,
        ),
        // t5v, rank T: t5 with the lanes of each vec2 swapped, tilt_v's shape; the product is stored
        // after the constant lane; 10 before rank T (`docs/notes/gc-compiler-ranks-3-5.md`)
        (
            "t5v",
            SHC_STAGE_FRAGMENT,
            fs("", "gl_FragColor = vec4(a.xy + vec2(0.0, -1.0 * b.x), a.xy + vec2(0.0, 1.0 * b.x));"),
            7,
        ),
    ]
}

#[test]
fn terrain_shaders_count() {
    let mut wrong = Vec::new();
    for (name, stage, src, want) in micro() {
        let (n, t) = count(stage, &src);
        println!("gc7000 micro {name}: {n} instructions, {t} temps (pinned {want})");
        if n != want {
            wrong.push(format!("{name}: {n} instructions, pinned {want}"));
        }
    }

    match std::env::var_os("GLMARK2_SRC") {
        None => eprintln!("SKIP gc_terrain_shader_count glmark2 half: GLMARK2_SRC (a glmark2 checkout) is not set"),
        Some(root) => {
            let mut total = 0;
            for ((name, stage, want_n, want_t), src) in GLMARK2.iter().zip(glmark2_texts(Path::new(&root))) {
                let (n, t) = count(*stage, &src);
                println!("gc7000 {name}: {n} instructions, {t} temps (pinned {want_n}, {want_t})");
                if (n, t) != (*want_n, *want_t) {
                    wrong.push(format!("{name}: {n} instructions {t} temps, pinned {want_n} {want_t}"));
                }
                total += n;
            }
            println!("gc7000 terrain: {total} instructions in the ten (pinned {GLMARK2_TOTAL}; etna_shc 279)");
            if total != GLMARK2_TOTAL {
                wrong.push(format!("total {total}, pinned {GLMARK2_TOTAL}"));
            }
        }
    }
    assert!(wrong.is_empty(), "counts moved; re-pin with the note's table:\n{}", wrong.join("\n"));
}
