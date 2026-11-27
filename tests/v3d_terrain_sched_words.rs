// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2 online compiler's word counts of glmark2 terrain's noise and
//! draw programs, and of short shaders of this repository's own, pinned as
//! they are today (plan #419, item terrain-words-test;
//! `docs/notes/v3d-terrain-shaders.md` sections 1, 2 and 5,
//! `docs/notes/pi4-slow-terrain.md` "What is left, ranked", row 1). Per stage
//! it pins the words, the threads and the number of words whose add half and
//! whose mul half are not `nop`. A change of the scheduler
//! (`src/shaderc/online/v3d/sched.rs`) moves these pins; re-pin them with the
//! values printed under `--nocapture`.
//!
//! Targets, not pins (the note's section 5 floors: a perfect pairing of the
//! add and mul halves, data dependences allowing): the noise FS about 872 to
//! 900 words (1495 today), the terrain VS about 115 (172), the terrain FS
//! about 125 (160).
//!
//! The programs are compiled as the note's section 1 and
//! `tests/v3d_varyings_compile.rs` do: `lower::compile_to_ir` per stage, then
//! `chain::compile_program(vs, fs, Prim::Triangles, Target { vpm_size: 65536 })`
//! with the default sampler key.
//!
//! The glmark2 texts are GPL-3.0 and never committed: they are built from a
//! glmark2 checkout (`GLMARK2_SRC`, its `data/shaders`) as
//! `tests/gc_terrain_shader_count.rs` builds them (`ShaderSource`'s two
//! precision macros and the default fragment precision). The noise FS is
//! paired with `terrain-texture.vert`, terrain's draw with `terrain.vert` and
//! `terrain.frag`. With no `GLMARK2_SRC` that half prints a SKIP line. The
//! shaders of this repository always run.
//!
//! ```text
//! GLMARK2_SRC=... cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online --test v3d_terrain_sched_words -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::path::Path;
use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Code, Prim, Target};
use yakogl::shaderc::online::v3d::isa::{AddOp, Instr, MulOp};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/// One stage: words, threads, words with an add half, words with a mul half.
type Stage = (usize, u32, usize, usize);

fn stage_of(code: &Code<'_>, threads: u32) -> Stage {
    let (mut add, mut mul) = (0, 0);
    for i in code.instrs {
        if let Instr::Alu(a) = i {
            add += usize::from(a.add.op != AddOp::Nop);
            mul += usize::from(a.mul.op != MulOp::Nop);
        }
    }
    (code.words.len(), threads, add, mul)
}

/// The (vs, cs, fs) stages of the program `vs` + `fs`.
fn compile(vs: &str, fs: &str) -> (Stage, Stage, Stage) {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs_ = a.alloc_str(vs).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v =
        lower::compile_to_ir(&a, sema::Stage::Vertex, vs_).unwrap_or_else(|e| panic!("the VS lowers: {}\n{vs}", e.msg));
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_)
        .unwrap_or_else(|e| panic!("the FS lowers: {}\n{fs}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    (stage_of(&p.vs.code, p.vs.threads), stage_of(&p.cs.code, p.cs.threads), stage_of(&p.fs.code, p.fs.threads))
}

/// `libmatrix/shader-source.cc`'s two precision macros.
const MACROS: &str =
    "#if defined(GL_ES) && defined(GL_FRAGMENT_PRECISION_HIGH)\n#define HIGHP_OR_DEFAULT highp\n#else\n\
#define HIGHP_OR_DEFAULT\n#endif\n#if defined(GL_ES)\n#define MEDIUMP_OR_DEFAULT mediump\n#else\n\
#define MEDIUMP_OR_DEFAULT\n#endif\n";
/// The default fragment precision `ShaderSource` adds.
const FPREC: &str = "#ifdef GL_ES\nprecision mediump float;\n#endif\n";

/// Pinned: (name, VS, CS, FS), each (words, threads, add words, mul words).
/// Only the stage named in `which` is pinned; the others are the paired
/// program's.
struct Pin {
    name: &'static str,
    /// 0 vs, 1 cs, 2 fs
    which: usize,
    want: Stage,
}

fn check(wrong: &mut Vec<String>, pins: &[Pin], got: (Stage, Stage, Stage)) {
    for p in pins {
        let g = [got.0, got.1, got.2][p.which];
        println!("v3d {}: {} words, {} threads, add {}, mul {} (pinned {:?})", p.name, g.0, g.1, g.2, g.3, p.want);
        if g != p.want {
            wrong.push(format!("{}: {g:?}, pinned {:?}", p.name, p.want));
        }
    }
}

const COMMON_VS: &str = "attribute vec4 p; varying vec4 v; void main() { v = p; gl_Position = p; }\n";

/// Short shaders of this repository's own, always run.
fn own(wrong: &mut Vec<String>) {
    // An independent add chain beside an independent mul chain: a perfect
    // pairing issues one add and one mul a word.
    let chains = format!(
        "precision mediump float;\nvarying vec4 v;\nvoid main() {{\n\
         float a = v.x; float m = v.y;\n{}\
         gl_FragColor = vec4(a, m, a + m, 1.0);\n}}\n",
        (0..24).map(|i| format!("a = a + v.z * 0.{}1; m = m * (v.w + 0.{}3);\n", i % 9, i % 7)).collect::<String>()
    );
    let (_, _, f) = compile(COMMON_VS, &chains);
    check(wrong, &[Pin { name: "own chains fs", which: 2, want: CHAINS }], (f, f, f));

    // A noise-like hash of fract, floor and mix.
    let hash = "precision mediump float;\nvarying vec4 v;\n\
        float h(float x) { return fract(sin(x) * 43758.5453); }\n\
        float n(vec2 p) { vec2 i = floor(p); vec2 f = fract(p); f = f * f * (3.0 - 2.0 * f);\n\
          return mix(mix(h(i.x + i.y * 57.0), h(i.x + 1.0 + i.y * 57.0), f.x),\n\
                     mix(h(i.x + (i.y + 1.0) * 57.0), h(i.x + 1.0 + (i.y + 1.0) * 57.0), f.x), f.y); }\n\
        void main() { float s = 0.0; float a = 0.5; vec2 p = v.xy * 8.0;\n\
          s += a * n(p); a *= 0.5; p = p * 2.0 + 17.0;\n\
          s += a * n(p); a *= 0.5; p = p * 2.0 + 17.0;\n\
          s += a * n(p);\n\
          gl_FragColor = vec4(vec3(s), 1.0); }\n";
    let (_, _, f) = compile(COMMON_VS, hash);
    check(wrong, &[Pin { name: "own hash fs", which: 2, want: HASH }], (f, f, f));
}

/// 24 add and 24 mul steps in two independent chains; a perfect pairing is
/// about 30 words (target, not a pin).
const CHAINS: Stage = (54, 4, 38, 37);
const HASH: Stage = (181, 4, 166, 68);

#[test]
fn terrain_sched_words() {
    let mut wrong = Vec::new();
    own(&mut wrong);

    match std::env::var_os("GLMARK2_SRC") {
        None => eprintln!("SKIP v3d_terrain_sched_words glmark2 half: GLMARK2_SRC (a glmark2 checkout) is not set"),
        Some(root) => {
            let rd = |n: &str| {
                std::fs::read_to_string(Path::new(&root).join("data/shaders").join(n))
                    .unwrap_or_else(|e| panic!("{n} in {}: {e}", Path::new(&root).display()))
            };
            let tex_vs = format!("{MACROS}{}", rd("terrain-texture.vert"));
            let noise = format!("{MACROS}{FPREC}{}", rd("terrain-noise.frag"));
            check(&mut wrong, &[Pin { name: "noise fs", which: 2, want: NOISE_FS }], compile(&tex_vs, &noise));

            let t_vs = format!("{MACROS}{}", rd("terrain.vert"));
            let t_fs = format!("{MACROS}{FPREC}{}", rd("terrain.frag"));
            check(
                &mut wrong,
                &[
                    Pin { name: "terrain vs", which: 0, want: TERRAIN_VS },
                    Pin { name: "terrain cs", which: 1, want: TERRAIN_CS },
                    Pin { name: "terrain fs", which: 2, want: TERRAIN_FS },
                ],
                compile(&t_vs, &t_fs),
            );
        }
    }
    assert!(wrong.is_empty(), "counts moved; re-pin:\n{}", wrong.join("\n"));
}

// glmark2's programs (words, threads, add words, mul words); floors in the header.
const NOISE_FS: Stage = (1495, 2, 872, 830);
const TERRAIN_VS: Stage = (172, 2, 109, 98);
const TERRAIN_CS: Stage = (97, 4, 60, 54);
const TERRAIN_FS: Stage = (160, 4, 89, 119);
