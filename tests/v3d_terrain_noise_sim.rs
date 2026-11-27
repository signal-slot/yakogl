// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2 interpreter pins what glmark2 terrain's noise and draw
//! programs compute, on a fixed pixel grid (plan #419, item noise-sim-pin;
//! `docs/notes/v3d-terrain-shaders.md` sections 1 and 5: "the sim
//! interpreter can show that values are unchanged").
//!
//! A change to the scheduler's word packing (`src/shaderc/online/v3d/sched.rs`)
//! moves every program's word count but must not move a value. Each program
//! pair is compiled through the V3D 4.2 chain (`compile_program`, as
//! `v3d_varyings_compile.rs` does), then run on `sim::run`:
//!
//! * the fragment program on a 16 x 16 grid (16 runs of 16 lanes; lane = x,
//!   run = y) with every varying a fixed function of its input slot and the
//!   pixel, the uniforms fixed by their dword offset, and a set of small
//!   textures for the TMU look-ups; its TLB words and discard flags are
//!   hashed (FNV-1a, 64 bit) and the words of two pixels pinned;
//! * the vertex program on 16 lanes of attributes fixed by location and
//!   component; its VPM output rows are hashed.
//!
//! The pairs: glmark2 terrain's noise (`terrain-texture.vert` +
//! `terrain-noise.frag`) and draw (`terrain.vert` + `terrain.frag`), built
//! from a glmark2 checkout (`GLMARK2_SRC`, its `data/shaders`) as glmark2's
//! `ShaderSource` hands them over (the same text as
//! `gc_terrain_shader_count.rs`); GPL-3.0, never committed. With no
//! `GLMARK2_SRC` that half prints a SKIP line. Two noise-like shaders of
//! this repository's own (hash, fract, floor, mix, dot) always run.
//!
//! The sim's SFU ops are deterministic in the host interpreter, so the hashes
//! are bit for bit. Re-pin only for a change of the values, never of the
//! word count.
//!
//! ```text
//! GLMARK2_SRC=... cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_terrain_noise_sim -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::Uniform;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Texture, Varying, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const TARGET: Target = Target { vpm_size: 65536 };
const GRID: usize = 16;

/// What one program pair computed.
struct Digest {
    fs_hash: u64,
    /// The TLB words (every write, as hex of the lane's value) of pixel (0, 0)
    /// and pixel (5, 9).
    fs_samples: [Vec<u32>; 2],
    vs_hash: u64,
    /// The last VPM row, lane 3.
    vs_sample: u32,
}

fn fnv(h: &mut u64, w: Option<u32>) {
    for b in w.unwrap_or(0xdead_beef).to_le_bytes() {
        *h = (*h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// A fixed value in [0.21, 1.0) from a small integer: the uniform at dword `d`.
fn unif(d: u32) -> f32 {
    0.21 + 0.79 * ((d.wrapping_mul(7919) % 997) as f32 / 997.0)
}

/// The uniform stream of `uniforms` for a block laid out by dword: contents
/// as `v3d_varyings_compile.rs` resolves them, `fixed` (dword, value) over the
/// pattern; texture sizes 1 x 1 x 1.
fn stream(uniforms: &[Uniform], fixed: &[(u32, f32)]) -> Vec<u32> {
    uniforms
        .iter()
        .map(|u| match u.contents {
            0 | 40 | 41 => u.data,
            1 => fixed.iter().find(|f| f.0 == u.data).map_or_else(|| unif(u.data), |f| f.1).to_bits(),
            2..=5 => 1.0f32.to_bits(),
            44 | 45 => 1,
            46 | 47 => 1,
            0x1_0000 => (-1.0f32).to_bits(),
            c => panic!("uniform contents {c} not modelled by this test"),
        })
        .collect()
}

/// Eight textures of one texel each, in [0.6, 1.0) (so a normal map reads
/// as pointing out): the interpreter refuses a NEAREST coordinate within
/// 1e-3 texels of a texel edge, which a 256-pixel grid with many look-ups
/// reaches in any larger texture.
fn textures() -> Vec<Texture> {
    (0..8u32)
        .map(|t| {
            let texels = (0..1u32)
                .map(|i| core::array::from_fn(|c| 0.6 + 0.4 * ((i * 5 + t * 3 + c as u32 * 7) % 17) as f32 / 17.0))
                .collect();
            Texture::flat(1, 1, texels)
        })
        .collect()
}

/// Compile `vs` + `fs` and run both on the fixed inputs.
fn digest(vs: &str, fs: &str) -> Digest {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 32 << 20];
    let a = Arena::new(&mut mem);
    let vs_ = a.alloc_str(vs).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs_).unwrap_or_else(|e| panic!("the VS lowers: {}", e.msg));
    let fr =
        lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).unwrap_or_else(|e| panic!("the FS lowers: {}", e.msg));
    let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, &a).expect("the program compiles");

    /* fragment: 16 runs, run y holds pixels (0..16, y) */
    /* a point light's range is 50, not about 0.5: past it the light's weight is 0 and
     * the draw's colour would not depend on the pixel */
    let fixed: Vec<(u32, f32)> = fr
        .reflection
        .uniforms
        .iter()
        .filter(|r| r.name.starts_with("pointLightDistance"))
        .map(|r| (u32::from(r.dword_off), 50.0))
        .collect();
    let fu = stream(p.fs.code.uniforms, &fixed);
    let mut fs_hash = 0xcbf2_9ce4_8422_2325u64;
    let mut fs_samples = [Vec::new(), Vec::new()];
    for y in 0..GRID {
        let varyings =
            p.fs.input_slots
                .iter()
                .enumerate()
                .map(|(i, &(slot, comp))| {
                    let k = f32::from(slot - 32) * 4.0 + f32::from(comp);
                    let vary = core::array::from_fn(|x| {
                        let (u, w) = ((x as f32 + 0.5) / GRID as f32, (y as f32 + 0.5) / GRID as f32);
                        (0.5 * (u + 0.2) * (((k as u32 * 5 + 1) % 7) as f32 - 2.5) / 2.5
                            + 0.7 * w * (((k as u32 * 3 + 2) % 7) as f32 - 2.5) / 2.5
                            + 0.0137 * (i as f32 + 1.0))
                            .to_bits()
                    });
                    Varying { vary, c: [0; LANES] }
                })
                .collect();
        let inputs = FsInputs {
            w: [1.0f32.to_bits(); LANES],
            w_centroid: [1.0f32.to_bits(); LANES],
            z: [0.5f32.to_bits(); LANES],
            x: core::array::from_fn(|x| (x as f32 + 0.5).to_bits()),
            y: [(y as f32 + 0.5).to_bits(); LANES],
            back_facing: [false; LANES],
            varyings,
        };
        let env = Env { threads: p.fs.threads, textures: textures(), ..Env::default() };
        let o = sim::run(p.fs.code.words, &fu, &Inputs::Fragment(Box::new(inputs)), &env)
            .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
        for lane in 0..LANES {
            for w in &o.tlb {
                fnv(&mut fs_hash, w.value[lane]);
            }
            fnv(&mut fs_hash, Some(o.msf[lane]));
            for (s, at) in [(0, (0, 0)), (1, (5, 9))] {
                if (lane, y) == at {
                    fs_samples[s] = o.tlb.iter().map(|w| w.value[lane].unwrap_or(0xdead_beef)).collect();
                }
            }
        }
    }

    /* vertex: 16 lanes, each attribute record's rows by location and component */
    let vu = stream(p.vs.code.uniforms, &[]);
    let mut rows: Vec<[u32; LANES]> = Vec::new();
    for &(loc, rec) in p.attributes {
        let Some(rec) = rec else { continue };
        for c in 0..usize::from(p.vs.vattr_sizes[usize::from(rec)]) {
            rows.push(core::array::from_fn(|lane| {
                let r = (lane * 3 + usize::from(loc) * 5 + c * 7) % 11;
                (r as f32 / 11.0 - 0.3 + 0.01 * lane as f32).to_bits()
            }));
        }
    }
    let env = Env { threads: p.vs.threads, textures: textures(), ..Env::default() };
    let o = sim::run(p.vs.code.words, &vu, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
        .unwrap_or_else(|e| panic!("the vertex program runs: {e}"));
    let mut vs_hash = 0xcbf2_9ce4_8422_2325u64;
    for r in &o.vpm_out {
        for l in 0..LANES {
            fnv(&mut vs_hash, r.map(|r| r[l]));
        }
    }
    let vs_sample = o.vpm_out.last().copied().flatten().map_or(0xdead_beef, |r| r[3]);
    Digest { fs_hash, fs_samples, vs_hash, vs_sample }
}

fn show(name: &str, d: &Digest) {
    println!(
        "{name}: fs {:#018x} {:08x?} {:08x?}; vs {:#018x} {:#010x}",
        d.fs_hash, d.fs_samples[0], d.fs_samples[1], d.vs_hash, d.vs_sample
    );
}

/// Pinned values of one pair.
struct Pin {
    fs_hash: u64,
    fs_samples: [&'static [u32]; 2],
    vs_hash: u64,
    vs_sample: u32,
}

fn check(name: &str, d: &Digest, pin: &Pin) {
    show(name, d);
    assert_eq!(d.fs_hash, pin.fs_hash, "{name}: the fragment outputs moved");
    assert_eq!(d.fs_samples[0], pin.fs_samples[0], "{name}: pixel (0, 0)");
    assert_eq!(d.fs_samples[1], pin.fs_samples[1], "{name}: pixel (5, 9)");
    assert_eq!(d.vs_hash, pin.vs_hash, "{name}: the vertex outputs moved");
    assert_eq!(d.vs_sample, pin.vs_sample, "{name}: last vertex row, lane 3");
}

// ---- this repository's own noise-like shaders ------------------------------------

const OWN_VS: &str = "attribute vec3 position;\nvarying vec2 vUv;\n\
    void main() { vUv = position.xy * 0.5 + 0.5; gl_Position = vec4(position, 1.0); }\n";

/// Value noise: a fract-based hash of the lattice corners, smoothed, mixed.
const OWN_VALUE_NOISE: &str = "precision mediump float;\nvarying vec2 vUv;\nuniform float time;\n\
    float hash(vec2 p) {\n  vec3 q = fract(vec3(p.xyx) * 0.1031);\n  q += dot(q, q.yzx + 33.33);\n\
      return fract((q.x + q.y) * q.z);\n}\n\
    float vnoise(vec2 p) {\n  vec2 i = floor(p);\n  vec2 f = fract(p);\n  vec2 u = f * f * (3.0 - 2.0 * f);\n\
      return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),\n\
                 mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);\n}\n\
    void main() {\n  float n = vnoise(vUv * 8.0 + time);\n  gl_FragColor = vec4(n, vnoise(vUv * 16.0), n * n, 1.0);\n}\n";

/// A three-octave sum of |dot|-hashed gradients, glmark2 noise's shape (abs,
/// octave scale, step, mod).
const OWN_FBM: &str = "precision mediump float;\nvarying vec2 vUv;\nuniform float time;\n\
    float g(vec2 p) {\n  vec2 i = floor(p);\n  vec2 d = fract(p) - 0.5;\n\
      vec2 h = fract(i * vec2(0.3183, 0.3678) + 0.1);\n\
      h = h * 2.0 - 1.0;\n  return dot(h, d) * step(0.1, h.x + 0.5) + mod(dot(i, vec2(1.7, 2.3)), 3.0) * 0.1;\n}\n\
    void main() {\n  float n = 0.0;\n  float a = 1.0;\n  vec2 p = vUv * 4.0 + vec2(0.0, time);\n\
      for (int k = 0; k < 3; k++) {\n    n += a * abs(g(p));\n    p *= 2.0;\n    a *= 0.5;\n  }\n\
      gl_FragColor = vec4(vec3(n), 1.0);\n}\n";

#[test]
fn own_value_noise_is_pinned() {
    check(
        "own value noise",
        &digest(OWN_VS, OWN_VALUE_NOISE),
        &Pin {
            fs_hash: 0xf1d7_0cae_bc0c_3d22,
            fs_samples: [&[0x339f_3a89, 0x3c00_3956], &[0x3a55_307a, 0x3c00_2503]],
            vs_hash: 0x2691_7bd8_45ad_702d,
            vs_sample: 0x3f17_9f30,
        },
    );
}

#[test]
fn own_fbm_is_pinned() {
    check(
        "own fbm",
        &digest(OWN_VS, OWN_FBM),
        &Pin {
            fs_hash: 0x29d3_b265_9a71_2297,
            fs_samples: [&[0x34ab_34ab, 0x3c00_34ab], &[0x2fce_2fce, 0x3c00_2fce]],
            vs_hash: 0x2691_7bd8_45ad_702d,
            vs_sample: 0x3f17_9f30,
        },
    );
}

// ---- glmark2 terrain ---------------------------------------------------------------

/// `libmatrix/shader-source.cc`'s two precision macros.
const MACROS: &str =
    "#if defined(GL_ES) && defined(GL_FRAGMENT_PRECISION_HIGH)\n#define HIGHP_OR_DEFAULT highp\n#else\n\
#define HIGHP_OR_DEFAULT\n#endif\n#if defined(GL_ES)\n#define MEDIUMP_OR_DEFAULT mediump\n#else\n\
#define MEDIUMP_OR_DEFAULT\n#endif\n";
/// The default fragment precision `ShaderSource` adds.
const FPREC: &str = "#ifdef GL_ES\nprecision mediump float;\n#endif\n";

#[test]
fn glmark2_terrain_noise_and_draw_are_pinned() {
    let Some(root) = std::env::var_os("GLMARK2_SRC") else {
        eprintln!("SKIP v3d_terrain_noise_sim glmark2 half: GLMARK2_SRC (a glmark2 checkout) is not set");
        return;
    };
    let root = std::path::Path::new(&root);
    let rd = |n: &str| {
        std::fs::read_to_string(root.join("data/shaders").join(n))
            .unwrap_or_else(|e| panic!("{n} in {}: {e}", root.display()))
    };
    let noise = digest(
        &format!("{MACROS}{}", rd("terrain-texture.vert")),
        &format!("{MACROS}{FPREC}{}", rd("terrain-noise.frag")),
    );
    let draw = digest(&format!("{MACROS}{}", rd("terrain.vert")), &format!("{MACROS}{FPREC}{}", rd("terrain.frag")));
    check("terrain noise", &noise, &NOISE_PIN);
    check("terrain draw", &draw, &DRAW_PIN);
}

const NOISE_PIN: Pin = Pin {
    fs_hash: 0x438e_6c98_1f7c_bbd3,
    fs_samples: [&[0x3b44_3b44, 0x3c00_3b44], &[0x3a59_3a59, 0x3c00_3a59]],
    vs_hash: 0xb73e_5b70_344f_0b72,
    vs_sample: 0x3fbb_c1f4,
};
const DRAW_PIN: Pin = Pin {
    fs_hash: 0xdbb3_20c8_7fc9_1b97,
    fs_samples: [&[0x3cfa_392c, 0x3545_3d06], &[0x3c66_3829, 0x3545_3c73]],
    vs_hash: 0x33a0_0fea_d419_96c6,
    vs_sample: 0xbf67_15ee,
};
