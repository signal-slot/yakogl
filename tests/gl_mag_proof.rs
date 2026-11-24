// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The per-draw magnification proof of `src/gl/mag_proof.rs` (rule M of
//! docs/notes/lazy-mip-chain.md) on glmark2 terrain's five mip-sampled
//! draws, and on negative cases.
//!
//! The module is not declared in any `mod.rs` yet; this test reaches it
//! through `#[path]` and uses nothing of the crate. Any feature set that
//! builds the library runs it; the command below uses the PowerVR host set,
//! as tests/es3_float_pack.rs does.
//!
//! Each program is encoded by hand from glmark2's shader text at the commit
//! the harness pins, `22c527cb0556f3a1ac4445aaa52cc532760928d5`
//! (tools/hostharness/glmark2/README.md, "GLMARK2_SRC"). Citations are
//! `GLMARK2_SRC`-relative paths and lines.
//!
//! The draws (lazy-mip-chain.md, "The per-chain table"), all with the
//! fullscreen quad of `Mesh::make_grid(1, 1, 2.0, 2.0, 0)`
//! (src/scene-terrain/texture-renderer.cpp:38,
//! src/scene-terrain/overlay-renderer.cpp:100; vertices src/mesh.cpp:612-631),
//! `uvOffset` (0, 0) and `uvScale` (1, 1)
//! (src/scene-terrain/blur-renderer.cpp:67-68, overlay-renderer.cpp:125-126):
//!
//! | chain | texture, level 0 | reader | viewport | expected |
//! |---|---|---|---|---|
//! | [1] | terrain target 800x600 | bloom H blur | 256x256 | Unknown |
//! | [2] | bloom H 256x256 | bloom V blur | 256x256 | Unknown |
//! | [3] | bloom V 256x256 | overlay | 800x600 | **ProvenMagnified** |
//! | [4] | terrain target 800x600 | tilt-shift H blur | 800x600 | Unknown |
//! | [5] | tilt-shift H 800x600 | tilt-shift V blur | 800x600 | Unknown |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test gl_mag_proof
//! ```

#[path = "../src/gl/mag_proof.rs"]
mod mag_proof;

use mag_proof::*;
use std::collections::HashMap;

// ---- expression helpers ----

fn attr(name: &str, width: usize) -> Expr {
    Expr::Attribute(name.into(), width)
}
fn vary(name: &str) -> Expr {
    Expr::Varying(name.into())
}
fn uni(name: &str) -> Expr {
    Expr::Uniform(name.into())
}
fn k(v: &[f64]) -> Expr {
    Expr::Const(v.to_vec())
}
fn add(a: Expr, b: Expr) -> Expr {
    Expr::Add(Box::new(a), Box::new(b))
}
fn mul(a: Expr, b: Expr) -> Expr {
    Expr::Mul(Box::new(a), Box::new(b))
}
fn matvec(m: Expr, v: Expr) -> Expr {
    Expr::MatVec(Box::new(m), Box::new(v))
}
fn swz(a: Expr, idx: &[usize]) -> Expr {
    Expr::Swizzle(Box::new(a), idx.to_vec())
}
fn vec_of(parts: Vec<Expr>) -> Expr {
    Expr::Construct(parts)
}
fn tex(sampler: &str, coord: Expr) -> Expr {
    Expr::Texture2D { sampler: sampler.into(), coord: Box::new(coord), bias: None, lod: None }
}
fn other(label: &str, args: Vec<Expr>) -> Expr {
    Expr::Other(label.into(), args)
}

// ---- glmark2's terrain programs ----

/// data/shaders/terrain-texture.vert:10-11
/// `vUv = uvScale * (position.xy * 0.5 + 0.5) + uvOffset;`
/// `gl_Position = vec4( position, 1.0);` (`attribute vec3 position`, :4).
fn terrain_texture_vert() -> VertexStage {
    let pos = attr("position", 3);
    VertexStage {
        varyings: vec![(
            "vUv".into(),
            add(mul(uni("uvScale"), add(mul(swz(pos.clone(), &[0, 1]), k(&[0.5])), k(&[0.5]))), uni("uvOffset")),
        )],
        position: vec_of(vec![pos, k(&[1.0])]),
    }
}

/// data/shaders/terrain-overlay.frag:34-35
/// `vec4 texel = texture2D(tDiffuse, vUv);`
/// `gl_FragColor = opacity * texel;`
fn overlay_frag_with(fetch: Expr) -> FragmentStage {
    FragmentStage { outputs: vec![mul(uni("opacity"), fetch)] }
}

fn overlay_program() -> Program {
    Program { vertex: terrain_texture_vert(), fragment: overlay_frag_with(tex("tDiffuse", vary("vUv"))) }
}

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    H,
    V,
}

/// The Gaussian kernel of src/scene-terrain/blur-renderer.cpp:83-104
/// (`Kernel0..KernelR`, `add_const`).
fn kernel(radius: usize, sigma: f64) -> Vec<f64> {
    let sigma = sigma.max(1.0);
    let s2 = 2.0 * sigma * sigma;
    let values: Vec<f64> =
        (0..=radius).map(|i| 1.0 / (std::f64::consts::PI * s2).sqrt() * (-((i * i) as f64) / s2).exp()).collect();
    let mut sum: f64 = values.iter().sum();
    sum += sum - values[0];
    values.iter().map(|v| v / sum).collect()
}

/// data/shaders/terrain-blur.frag:19-24 with the `$CONVOLUTION$` text of
/// src/scene-terrain/blur-renderer.cpp:108-147.
///
/// `tilt_shift == 1.0` gives `const float stepX = TextureStepX;` (:114,
/// :122); any other value gives
/// `float stepX = TextureStepX * abs(TiltShift - TextureCoord.y) / abs(1.0 - TiltShift);`
/// (:116, :125). terrain uses 0.0 for bloom and 0.5 for tilt-shift
/// (src/scene-terrain.cpp:88-121), so its four blurs all take the second
/// form. `abs(...)` is an [`Expr::Other`]; the division by the constant
/// `abs(1.0 - TiltShift)` is folded into a constant factor by hand.
///
/// Each tap (:133-135, :142-144) is
/// `texture2D(Texture0, TextureCoord + vec2(<o>.0 * stepX, 0.0)) * Kernel|o|`
/// (or `vec2(0.0, <o>.0 * stepY)`), summed with a final `0.0`, and
/// `gl_FragColor = vec4(result.xyz, 1.0)` (:24). `TextureCoord = vUv` (:20).
fn blur_frag(radius: usize, sigma: f64, dir: Dir, step: f64, tilt_shift: f64) -> FragmentStage {
    let coord = vary("vUv");
    let step_expr = if tilt_shift == 1.0 {
        k(&[step])
    } else {
        mul(
            k(&[step]),
            mul(
                other("abs", vec![add(k(&[tilt_shift]), mul(k(&[-1.0]), swz(coord.clone(), &[1])))]),
                k(&[1.0 / (1.0 - tilt_shift).abs()]),
            ),
        )
    };
    let kern = kernel(radius, sigma);
    let mut result = k(&[0.0]);
    for i in 0..(2 * radius + 1) {
        let o = i as f64 - radius as f64;
        let off = mul(k(&[o]), step_expr.clone());
        let delta = match dir {
            Dir::H => vec_of(vec![off, k(&[0.0])]),
            Dir::V => vec_of(vec![k(&[0.0]), off]),
        };
        let tap = mul(tex("Texture0", add(coord.clone(), delta)), k(&[kern[(o.abs()) as usize]]));
        result = add(tap, result);
    }
    FragmentStage { outputs: vec![vec_of(vec![swz(result, &[0, 1, 2]), k(&[1.0])])] }
}

fn blur_program(radius: usize, sigma: f64, dir: Dir, step: f64, tilt_shift: f64) -> Program {
    Program { vertex: terrain_texture_vert(), fragment: blur_frag(radius, sigma, dir, step, tilt_shift) }
}

/// The quad of src/mesh.cpp:612-631 for `make_grid(1, 1, 2.0, 2.0, 0)`:
/// a = (-1, 1, 0), b = (-1, -1, 0), c = (1, 1, 0), d = (1, -1, 0), drawn as
/// ul, ll, ur, ll, lr, ur.
fn quad() -> AttribSource {
    let (a, b, c, d) = (vec![-1.0, 1.0, 0.0], vec![-1.0, -1.0, 0.0], vec![1.0, 1.0, 0.0], vec![1.0, -1.0, 0.0]);
    AttribSource::Vertices(vec![a, b.clone(), c.clone(), b, d, c])
}

fn draw(viewport: (u32, u32), texture: (u32, u32)) -> DrawState {
    let mut attributes = HashMap::new();
    attributes.insert("position".to_string(), quad());
    let mut uniforms = HashMap::new();
    uniforms.insert("uvOffset".to_string(), UniformValue::Vec(vec![0.0, 0.0]));
    uniforms.insert("uvScale".to_string(), UniformValue::Vec(vec![1.0, 1.0]));
    // src/scene-terrain.cpp:103 (OverlayRenderer opacity 0.6),
    // src/scene-terrain/overlay-renderer.cpp:124.
    uniforms.insert("opacity".to_string(), UniformValue::Vec(vec![0.6]));
    DrawState {
        viewport: Viewport { width: viewport.0, height: viewport.1 },
        attributes,
        uniforms,
        texture: TextureSize { width: texture.0, height: texture.1 },
    }
}

fn unknown(v: MagVerdict) -> Unknown {
    match v {
        MagVerdict::Unknown(u) => u,
        MagVerdict::ProvenMagnified { rho_bound } => {
            panic!("expected Unknown, got ProvenMagnified (rho {rho_bound})")
        }
    }
}

fn not_magnified_rho(v: MagVerdict) -> f64 {
    match unknown(v) {
        Unknown::NotMagnified { rho_bound } => rho_bound,
        u => panic!("expected NotMagnified, got {u:?}"),
    }
}

// Screen and bloom sizes: src/scene-terrain.cpp:62-64 (screen_res is the
// canvas, 800x600 in the harness and on the boards).
const SCREEN: (u32, u32) = (800, 600);
const BLOOM: (u32, u32) = (256, 256);

// Blur parameters: src/scene-terrain.cpp:88-121.
// bloom: radius 2, sigma 4.0, tilt_shift 0.0; H step 1/screen_res.x, V step
// 1/bloom_res.y. tilt-shift: radius 4, sigma 2.7, tilt_shift 0.5, steps
// 1/screen_res.
fn bloom_h(tilt: f64) -> Program {
    blur_program(2, 4.0, Dir::H, 1.0 / SCREEN.0 as f64, tilt)
}
fn bloom_v(tilt: f64) -> Program {
    blur_program(2, 4.0, Dir::V, 1.0 / BLOOM.1 as f64, tilt)
}
fn tilt_h(tilt: f64) -> Program {
    blur_program(4, 2.7, Dir::H, 1.0 / SCREEN.0 as f64, tilt)
}
fn tilt_v(tilt: f64) -> Program {
    blur_program(4, 2.7, Dir::V, 1.0 / SCREEN.1 as f64, tilt)
}

// ---- chain [3]: the overlay draw, the one proven magnified ----

#[test]
fn chain3_overlay_samples_bloom_v_magnified() {
    // overlay-renderer.cpp:74-83: the terrain target's fb (800x600), unit 0
    // holds bloom V's 256x256 output (`input_texture_`).
    let v = prove_magnified(&overlay_program(), "tDiffuse", &draw(SCREEN, BLOOM));
    let MagVerdict::ProvenMagnified { rho_bound } = v else {
        panic!("overlay draw not proven: {v:?}");
    };
    // rho = max(256/800, 256/600) = 0.4267 (lazy-mip-chain.md, chain [3]).
    assert!((rho_bound - 256.0 / 600.0).abs() < 1e-9, "rho {rho_bound}");
}

// ---- chains [1], [2], [4], [5]: needed ----

#[test]
fn chain1_bloom_h_reads_terrain_target_unknown() {
    let d = draw(BLOOM, SCREEN);
    assert_eq!(unknown(prove_magnified(&bloom_h(0.0), "Texture0", &d)), Unknown::CoordNotAffine);
    // Without the abs() step it would still be minified: 800 texels over
    // 256 pixels (lambda 1.64).
    let rho = not_magnified_rho(prove_magnified(&bloom_h(1.0), "Texture0", &d));
    assert!((rho - 800.0 / 256.0).abs() < 1e-9, "rho {rho}");
}

#[test]
fn chain2_bloom_v_reads_bloom_h_unknown() {
    let d = draw(BLOOM, BLOOM);
    assert_eq!(unknown(prove_magnified(&bloom_v(0.0), "Texture0", &d)), Unknown::CoordNotAffine);
    // 1:1 at best: rho 1 is not below the limit.
    let rho = not_magnified_rho(prove_magnified(&bloom_v(1.0), "Texture0", &d));
    assert!((rho - 1.0).abs() < 1e-9, "rho {rho}");
}

#[test]
fn chain4_tilt_h_reads_terrain_target_unknown() {
    let d = draw(SCREEN, SCREEN);
    assert_eq!(unknown(prove_magnified(&tilt_h(0.5), "Texture0", &d)), Unknown::CoordNotAffine);
    let rho = not_magnified_rho(prove_magnified(&tilt_h(1.0), "Texture0", &d));
    assert!((rho - 1.0).abs() < 1e-9, "rho {rho}");
}

#[test]
fn chain5_tilt_v_reads_tilt_h_unknown() {
    // tilt_v_renderer->setup_onscreen(canvas): the window, 800x600.
    let d = draw(SCREEN, SCREEN);
    assert_eq!(unknown(prove_magnified(&tilt_v(0.5), "Texture0", &d)), Unknown::CoordNotAffine);
    let rho = not_magnified_rho(prove_magnified(&tilt_v(1.0), "Texture0", &d));
    assert!((rho - 1.0).abs() < 1e-9, "rho {rho}");
}

#[test]
fn exactly_one_of_terrains_five_chains_is_proven() {
    let cases = [
        (bloom_h(0.0), "Texture0", draw(BLOOM, SCREEN)),
        (bloom_v(0.0), "Texture0", draw(BLOOM, BLOOM)),
        (overlay_program(), "tDiffuse", draw(SCREEN, BLOOM)),
        (tilt_h(0.5), "Texture0", draw(SCREEN, SCREEN)),
        (tilt_v(0.5), "Texture0", draw(SCREEN, SCREEN)),
    ];
    let proven: Vec<usize> = cases
        .iter()
        .enumerate()
        .filter(|(_, (p, s, d))| prove_magnified(p, s, d).is_proven())
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(proven, vec![3]);
}

// ---- negative cases on the overlay program ----

#[test]
fn bias_is_unknown() {
    let fetch = Expr::Texture2D {
        sampler: "tDiffuse".into(),
        coord: Box::new(vary("vUv")),
        bias: Some(Box::new(k(&[-4.0]))),
        lod: None,
    };
    let p = Program { vertex: terrain_texture_vert(), fragment: overlay_frag_with(fetch) };
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &draw(SCREEN, BLOOM))), Unknown::Bias);
}

#[test]
fn explicit_lod_is_unknown() {
    // texture2DLodEXT in the fragment stage.
    let fetch = Expr::Texture2D {
        sampler: "tDiffuse".into(),
        coord: Box::new(vary("vUv")),
        bias: None,
        lod: Some(Box::new(k(&[0.0]))),
    };
    let p = Program { vertex: terrain_texture_vert(), fragment: overlay_frag_with(fetch) };
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &draw(SCREEN, BLOOM))), Unknown::Lod);
}

#[test]
fn vertex_stage_fetch_is_unknown() {
    // texture2DLod in the vertex stage, as terrain.vert:40 reads
    // tDisplacement.
    let mut p = overlay_program();
    p.vertex.varyings.push((
        "vExtra".into(),
        Expr::Texture2D {
            sampler: "tDiffuse".into(),
            coord: Box::new(swz(attr("position", 3), &[0, 1])),
            bias: None,
            lod: Some(Box::new(k(&[0.0]))),
        },
    ));
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &draw(SCREEN, BLOOM))), Unknown::VertexStageFetch);
}

#[test]
fn dependent_read_is_unknown() {
    let coord = swz(tex("tOffset", vary("vUv")), &[0, 1]);
    let p = Program { vertex: terrain_texture_vert(), fragment: overlay_frag_with(tex("tDiffuse", coord)) };
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &draw(SCREEN, BLOOM))), Unknown::DependentRead);
    // An affine coordinate plus a fetched offset is a dependent read too.
    let coord = add(vary("vUv"), mul(k(&[0.001]), swz(tex("tOffset", vary("vUv")), &[0, 1])));
    let p = Program { vertex: terrain_texture_vert(), fragment: overlay_frag_with(tex("tDiffuse", coord)) };
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &draw(SCREEN, BLOOM))), Unknown::DependentRead);
}

#[test]
fn non_affine_coordinate_is_unknown() {
    let p = Program {
        vertex: terrain_texture_vert(),
        fragment: overlay_frag_with(tex("tDiffuse", mul(vary("vUv"), vary("vUv")))),
    };
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &draw(SCREEN, BLOOM))), Unknown::CoordNotAffine);
    // A non-affine varying (computed in the vertex stage) too.
    let mut p = overlay_program();
    let pos = attr("position", 3);
    p.vertex.varyings[0].1 = mul(swz(pos.clone(), &[0, 1]), swz(pos, &[0, 1]));
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &draw(SCREEN, BLOOM))), Unknown::CoordNotAffine);
}

#[test]
fn minification_is_unknown() {
    // The overlay program into a 128x128 viewport: 2 texels a pixel.
    let rho = not_magnified_rho(prove_magnified(&overlay_program(), "tDiffuse", &draw((128, 128), BLOOM)));
    assert!((rho - 2.0).abs() < 1e-9, "rho {rho}");
    // uvScale 4 on the 800x600 target: 4 * 256 / 600 = 1.71.
    let mut d = draw(SCREEN, BLOOM);
    d.uniforms.insert("uvScale".into(), UniformValue::Vec(vec![4.0, 4.0]));
    let rho = not_magnified_rho(prove_magnified(&overlay_program(), "tDiffuse", &d));
    assert!((rho - 4.0 * 256.0 / 600.0).abs() < 1e-9, "rho {rho}");
}

#[test]
fn rotated_coordinate_uses_the_sum_bound() {
    // vUv = the quad's 0.5 * position.xy rotated 45 degrees, into a 600x600
    // viewport (x_win = 300 * x): du/dx = dv/dx = s / 300 a pixel with
    // s = 0.5 / sqrt(2), times T texels. The exact rho is T / 600; the
    // bound |du/dx| + |dv/dx| is T / (300 * sqrt(2)), sqrt(2) times more.
    let mut p = overlay_program();
    let pos = attr("position", 3);
    let (x, y) = (swz(pos.clone(), &[0]), swz(pos, &[1]));
    let s = 0.5 * std::f64::consts::FRAC_1_SQRT_2;
    p.vertex.varyings[0].1 =
        vec_of(vec![add(mul(k(&[s]), x.clone()), mul(k(&[-s]), y.clone())), add(mul(k(&[s]), x), mul(k(&[s]), y))]);
    // T = 300: exact 0.5, bound 0.707.
    let v = prove_magnified(&p, "tDiffuse", &draw((600, 600), (300, 300)));
    let MagVerdict::ProvenMagnified { rho_bound } = v else {
        panic!("{v:?}");
    };
    assert!((rho_bound - 0.5 * 2f64.sqrt()).abs() < 1e-9, "rho {rho_bound}");
    // 400x400 into 600x600: exact 0.667, bound 0.943, proven; 450x450:
    // exact 0.75, bound 1.06, Unknown.
    assert!(prove_magnified(&p, "tDiffuse", &draw((600, 600), (400, 400))).is_proven());
    let rho = not_magnified_rho(prove_magnified(&p, "tDiffuse", &draw((600, 600), (450, 450))));
    assert!(rho > 1.0);
}

#[test]
fn rho_at_the_limit_is_unknown() {
    // Exactly 1:1 (rho 1) and just under it, within the margin.
    assert!(!prove_magnified(&overlay_program(), "tDiffuse", &draw((256, 256), BLOOM)).is_proven());
    assert!(!prove_magnified(&overlay_program(), "tDiffuse", &draw((256, 256), (255, 255))).is_proven());
    assert!(prove_magnified(&overlay_program(), "tDiffuse", &draw((256, 256), (254, 254))).is_proven());
}

#[test]
fn perspective_position_is_unknown() {
    // gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0)
    // (data/shaders/terrain.vert:27,43 without the displacement): a
    // perspective projection makes w depend on the position.
    let mut p = overlay_program();
    p.vertex.position =
        matvec(uni("projectionMatrix"), matvec(uni("modelViewMatrix"), vec_of(vec![attr("position", 3), k(&[1.0])])));
    let mut d = draw(SCREEN, BLOOM);
    // terrain.vert's other attributes (normal, uv) for the displacement
    // case below.
    d.attributes.insert("normal".into(), AttribSource::Range { min: vec![-1.0, -1.0, -1.0], max: vec![1.0, 1.0, 1.0] });
    d.attributes.insert("uv".into(), AttribSource::Range { min: vec![0.0, 0.0], max: vec![1.0, 1.0] });
    // A z range so the w row sees the position.
    d.attributes
        .insert("position".into(), AttribSource::Range { min: vec![-1.0, -1.0, -1.0], max: vec![1.0, 1.0, 1.0] });
    #[rustfmt::skip]
    let ident = vec![1.0, 0.0, 0.0, 0.0,  0.0, 1.0, 0.0, 0.0,  0.0, 0.0, 1.0, 0.0,  0.0, 0.0, -5.0, 1.0];
    // A glFrustum-like column-major matrix: w = -z_eye.
    #[rustfmt::skip]
    let persp = vec![1.0, 0.0, 0.0, 0.0,  0.0, 1.0, 0.0, 0.0,  0.0, 0.0, -1.2, -1.0,  0.0, 0.0, -2.2, 0.0];
    d.uniforms.insert("modelViewMatrix".into(), UniformValue::Mat { n: 4, data: ident.clone() });
    d.uniforms.insert("projectionMatrix".into(), UniformValue::Mat { n: 4, data: persp });
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &d)), Unknown::WNotConstant);
    // The real terrain.vert:42-43 adds a displacement read from a texture:
    // gl_Position itself is then a dependent read.
    let mut p2 = p.clone();
    p2.vertex.position = matvec(
        uni("projectionMatrix"),
        add(
            vec_of(vec![mul(attr("normal", 3), swz(tex("tDisplacement", attr("uv", 2)), &[0])), k(&[0.0])]),
            matvec(uni("modelViewMatrix"), vec_of(vec![attr("position", 3), k(&[1.0])])),
        ),
    );
    assert_eq!(unknown(prove_magnified(&p2, "tDiffuse", &d)), Unknown::PositionNotAffine);
    // An orthographic projection (w row 0 0 0 1) through the same matrices
    // is affine, and 2 units of x span the 800 pixels again: proven.
    let mut ortho = ident;
    ortho[14] = 0.0;
    d.uniforms.insert("modelViewMatrix".into(), UniformValue::Mat { n: 4, data: ortho.clone() });
    d.uniforms.insert("projectionMatrix".into(), UniformValue::Mat { n: 4, data: ortho });
    assert!(prove_magnified(&p, "tDiffuse", &d).is_proven());
}

#[test]
fn coordinate_from_an_unseen_attribute_direction_is_unknown() {
    // vUv reads position.z, which the window position does not see: its
    // gradient depends on each triangle's slope in z.
    let mut p = overlay_program();
    let pos = attr("position", 3);
    p.vertex.varyings[0].1 = vec_of(vec![swz(pos.clone(), &[0]), swz(pos, &[2])]);
    let mut d = draw(SCREEN, BLOOM);
    d.attributes
        .insert("position".into(), AttribSource::Range { min: vec![-1.0, -1.0, -1.0], max: vec![1.0, 1.0, 1.0] });
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &d)), Unknown::DependsOnPrimitive);
    // terrain's quad has z = 0 at every vertex: z is a constant, the
    // coordinate's v is constant, and the draw is magnified.
    let d = draw(SCREEN, BLOOM);
    assert!(prove_magnified(&p, "tDiffuse", &d).is_proven());
}

#[test]
fn missing_inputs_are_unknown() {
    let mut d = draw(SCREEN, BLOOM);
    d.uniforms.remove("uvScale");
    assert!(matches!(unknown(prove_magnified(&overlay_program(), "tDiffuse", &d)), Unknown::BadInput(_)));
    let mut d = draw(SCREEN, BLOOM);
    d.attributes.clear();
    assert!(matches!(unknown(prove_magnified(&overlay_program(), "tDiffuse", &d)), Unknown::BadInput(_)));
    // A sampler the program does not fetch.
    assert_eq!(unknown(prove_magnified(&overlay_program(), "tOther", &draw(SCREEN, BLOOM))), Unknown::NoFetch);
}

#[test]
fn other_node_is_never_seen_through() {
    // A function call around an otherwise affine coordinate.
    let p = Program {
        vertex: terrain_texture_vert(),
        fragment: overlay_frag_with(tex("tDiffuse", other("fract", vec![vary("vUv")]))),
    };
    assert_eq!(unknown(prove_magnified(&p, "tDiffuse", &draw(SCREEN, BLOOM))), Unknown::CoordNotAffine);
}
