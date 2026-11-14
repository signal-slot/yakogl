// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The shared helper of the V3D 7.1 (Raspberry Pi 5) compiler tests: the four
//! shapes of docs/notes/pre-study-rpi5.md P8 and the runner that judges a
//! 7.1 program of any origin (ours, Mesa's) against a shape.
//!
//! * [`Shape`]: one shape's GLSL (vertex and fragment), its inputs (the
//!   uniform values, the varyings of the 16 lanes, a small texture) and its
//!   expected colour per lane.
//! * [`run71`] / [`run71_stream`]: round-trip every word through codec71
//!   (`decode71`, then `encode71` must give the word back), run the verifier
//!   ([`verify71_hook`]), run the words in `sim::run_gen(Gen::V71, ..)` and
//!   compare the TLB colour with the shape's expectation.
//!
//! A test file includes it with `mod v3d71_common;` and the header's
//! feature set (`--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table`).
//!
//! The colour is read the way Mesa writes RT0 under the default specifier
//! (`vir_emit_tlb_color_write`, nir_to_vir.c:1953-1984): `vfpack(r, g)` then
//! `vfpack(b, a)`, two f16 pairs, the low half first. Any other TLB
//! specifier, a colour write of an undefined lane in a kept pixel, or a
//! count of writes other than two is an error naming it.

#![allow(dead_code)]

use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Sampler, Texture, Varying, LANES};

/// The TLB configuration word of RT0, f16 vec4 (`0xff`, the default
/// specifier): the uniform a TLBU write reads.
pub const TLB_CONFIG: u32 = 0xff;

/// The tolerance of an f16 colour: an absolute 2^-10 plus 2^-10 relative.
fn close(got: f32, want: f32) -> bool {
    (got - want).abs() <= 1.0 / 1024.0 + want.abs() / 1024.0
}

/// One P8 shape: sources, inputs, expectation.
#[derive(Clone, Debug)]
pub struct Shape {
    pub name: &'static str,
    /// GLSL ES 1.00 sources.
    pub vs: &'static str,
    pub fs: &'static str,
    /// The FS uniform values in declaration order (floats; [`Shape::stream`]
    /// is their bits).
    pub uniforms: Vec<f32>,
    /// The FS varyings in input order, one float component each, per lane
    /// (W is 1 in every lane: the varying is the value itself).
    pub varyings: Vec<[f32; LANES]>,
    /// The texture bound as unit 0 (nearest, clamp), if the shape samples.
    pub texture: Option<Texture>,
    /// The program's thread count (`Env::threads`).
    pub threads: u32,
    /// The expected colour of each lane (all lanes are kept).
    pub expect: [[f32; 4]; LANES],
}

impl Shape {
    /// A shape with no sources, inputs or expectation yet: for a hand-built
    /// program.
    pub fn bare(name: &'static str) -> Shape {
        Shape {
            name,
            vs: "",
            fs: "",
            uniforms: Vec::new(),
            varyings: Vec::new(),
            texture: None,
            threads: 1,
            expect: [[0.0; 4]; LANES],
        }
    }

    /// The default uniform stream: the uniform values, then the TLB
    /// configuration. A compiler that orders its stream otherwise (texture
    /// configuration words, constants it pulled out) passes its own to
    /// [`run71_stream`].
    pub fn stream(&self) -> Vec<u32> {
        let mut s: Vec<u32> = self.uniforms.iter().map(|f| f.to_bits()).collect();
        s.push(TLB_CONFIG);
        s
    }
}

/// What a run produced: the colour of each lane, and what the run did.
#[derive(Clone, Debug)]
pub struct Outputs {
    pub colour: [[f32; 4]; LANES],
    /// Lanes whose multisample flags ended at 0.
    pub discarded: [bool; LANES],
    pub instructions: usize,
    pub tmu_lookups: usize,
    pub approximate: bool,
}

/// The verify71 hook. `src/shaderc/online/v3d/verify71.rs` is not on main
/// yet (plan #365 item verify71): when it lands, call its entry point here
/// (it must return Err(reason) for a program Mesa's qpu_validate.c refuses
/// at 7.1). Until then this checks nothing.
pub fn verify71_hook(_words: &[u64]) -> Result<(), String> {
    // HOOK: verify71 is not on main; replace with the verify71 entry point.
    Ok(())
}

/// Round-trip, verify, run and compare, with the shape's default stream.
pub fn run71(words: &[u64], shape: &Shape) -> Result<Outputs, String> {
    run71_stream(words, &shape.stream(), shape)
}

/// [`run71`] with the program's own uniform stream.
pub fn run71_stream(words: &[u64], stream: &[u32], shape: &Shape) -> Result<Outputs, String> {
    let name = shape.name;
    for (pc, &w) in words.iter().enumerate() {
        let i = decode71(w).map_err(|e| format!("{name}: word {pc} ({w:#018x}) does not decode at 7.1: {e:?}"))?;
        let back = encode71(&i).map_err(|e| format!("{name}: word {pc} does not re-encode: {e:?}"))?;
        if back != w {
            return Err(format!("{name}: word {pc}: {w:#018x} re-encodes as {back:#018x}"));
        }
    }
    verify71_hook(words).map_err(|e| format!("{name}: verify71: {e}"))?;
    let out = run_fs(words, stream, shape)?;
    for l in 0..LANES {
        if out.discarded[l] {
            return Err(format!("{name}: lane {l} discarded"));
        }
        for c in 0..4 {
            if !close(out.colour[l][c], shape.expect[l][c]) {
                return Err(format!(
                    "{name}: lane {l} channel {c}: got {}, want {} (lane colour {:?}, want {:?})",
                    out.colour[l][c], shape.expect[l][c], out.colour[l], shape.expect[l]
                ));
            }
        }
    }
    Ok(out)
}

/// Run the words as the shape's fragment shader and read RT0's colour.
fn run_fs(words: &[u64], stream: &[u32], shape: &Shape) -> Result<Outputs, String> {
    let name = shape.name;
    let one = 1.0f32.to_bits();
    let inputs = FsInputs {
        w: [one; LANES],
        w_centroid: [one; LANES],
        z: [0.5f32.to_bits(); LANES],
        x: core::array::from_fn(|l| ((l % 4) as f32 + 0.5).to_bits()),
        y: core::array::from_fn(|l| ((l / 4) as f32 + 0.5).to_bits()),
        back_facing: [false; LANES],
        varyings: shape.varyings.iter().map(|v| Varying { vary: v.map(f32::to_bits), c: [0; LANES] }).collect(),
    };
    let env = Env {
        textures: shape.texture.iter().cloned().collect(),
        samplers: shape.texture.iter().map(Sampler::of).collect(),
        threads: shape.threads,
        ..Env::default()
    };
    let got = sim::run_gen(Gen::V71, words, stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| format!("{name}: sim71: {e}"))?;
    let mut spec = TLB_CONFIG;
    let mut pairs: Vec<[u32; LANES]> = Vec::new();
    for t in &got.tlb {
        if let Some(c) = t.config {
            spec = c & 0xff;
        }
        if spec != 0xff {
            return Err(format!("{name}: TLB specifier {spec:#x} (only RT0 f16 vec4, 0xff, is read)"));
        }
        let undefined = t.value.iter().zip(got.msf.iter()).any(|(v, m)| v.is_none() && *m != 0);
        if undefined {
            return Err(format!("{name}: a colour TLB write of an undefined value in a kept lane"));
        }
        pairs.push(t.value.map(|v| v.unwrap_or(0)));
    }
    if pairs.len() != 2 {
        return Err(format!("{name}: {} colour TLB writes, want 2 (vfpack r,g and b,a)", pairs.len()));
    }
    let h = |w: u32, hi: bool| sim::half_to_f32(if hi { (w >> 16) as u16 } else { w as u16 });
    let colour = core::array::from_fn(|l| {
        [h(pairs[0][l], false), h(pairs[0][l], true), h(pairs[1][l], false), h(pairs[1][l], true)]
    });
    Ok(Outputs {
        colour,
        discarded: got.msf.map(|m| m == 0),
        instructions: got.instructions,
        tmu_lookups: got.tmu_lookups,
        approximate: !got.approximate.is_empty(),
    })
}

// ---------------------------------------------------------------------------
// The four P8 shapes.
// ---------------------------------------------------------------------------

const VS_COLOUR: &str = "attribute vec4 a_pos;\nattribute vec4 a_color;\nvarying vec4 v_color;\nvoid main() {\n    gl_Position = a_pos;\n    v_color = a_color;\n}\n";

/// Values exactly representable as f16, so no rounding enters a comparison.
fn lane_value(l: usize, k: usize) -> f32 {
    ((l * 3 + k * 5) % 16) as f32 / 16.0
}

/// Pass-through: a varying vec4 copied to gl_FragColor.
pub fn passthrough() -> Shape {
    let varyings: Vec<[f32; LANES]> = (0..4).map(|k| core::array::from_fn(|l| lane_value(l, k))).collect();
    Shape {
        name: "passthrough",
        vs: VS_COLOUR,
        fs: "precision mediump float;\nvarying vec4 v_color;\nvoid main() {\n    gl_FragColor = v_color;\n}\n",
        expect: core::array::from_fn(|l| core::array::from_fn(|k| varyings[k][l])),
        varyings,
        ..Shape::bare("passthrough")
    }
}

/// Uniform: gl_FragColor from a vec4 uniform.
pub fn uniform() -> Shape {
    let u = [0.25f32, 0.5, 0.75, 1.0];
    Shape {
        name: "uniform",
        vs: "attribute vec4 a_pos;\nvoid main() {\n    gl_Position = a_pos;\n}\n",
        fs: "precision mediump float;\nuniform vec4 u_color;\nvoid main() {\n    gl_FragColor = u_color;\n}\n",
        uniforms: u.to_vec(),
        expect: [u; LANES],
        ..Shape::bare("uniform")
    }
}

/// The `if` shape's two colours.
pub const IF_THEN: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
pub const IF_ELSE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// If: a branch on a varying against a uniform picks one of two colours.
/// The varying is lane/16 and the uniform 0.5: lanes 0..=8 take the else
/// branch (`v_x > u_t` is false at equality), 9..=15 the then branch.
pub fn if_branch() -> Shape {
    let x: [f32; LANES] = core::array::from_fn(|l| l as f32 / 16.0);
    Shape {
        name: "if",
        vs: "attribute vec4 a_pos;\nattribute float a_x;\nvarying float v_x;\nvoid main() {\n    gl_Position = a_pos;\n    v_x = a_x;\n}\n",
        fs: "precision mediump float;\nuniform float u_t;\nvarying float v_x;\nvoid main() {\n    if (v_x > u_t) {\n        gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);\n    } else {\n        gl_FragColor = vec4(0.0, 0.0, 1.0, 1.0);\n    }\n}\n",
        uniforms: vec![0.5],
        expect: core::array::from_fn(|l| if x[l] > 0.5 { IF_THEN } else { IF_ELSE }),
        varyings: vec![x],
        ..Shape::bare("if")
    }
}

/// The texture shape's 2x2 texels (row 0 first), exact in f16.
pub const TEXELS: [[f32; 4]; 4] =
    [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [0.5, 0.25, 0.75, 0.5]];

/// Texture: texture2D of a varying coordinate, sampler unit 0 (NEAREST,
/// CLAMP_TO_EDGE). Lane l samples the centre of texel (l % 2, (l / 2) % 2);
/// lanes 8..16 move the s coordinate one whole texture away (below 0 or
/// above 1) to read the clamp to the same edge texel.
pub fn texture() -> Shape {
    let col = |l: usize| l % 2;
    let row = |l: usize| (l / 2) % 2;
    let s: [f32; LANES] = core::array::from_fn(|l| {
        let c = (col(l) as f32 + 0.5) / 2.0;
        match (l >= 8, col(l)) {
            (false, _) => c,
            (true, 1) => c + 1.0,
            (true, _) => c - 1.0,
        }
    });
    let t: [f32; LANES] = core::array::from_fn(|l| (row(l) as f32 + 0.5) / 2.0);
    Shape {
        name: "texture",
        vs: "attribute vec4 a_pos;\nattribute vec2 a_uv;\nvarying vec2 v_uv;\nvoid main() {\n    gl_Position = a_pos;\n    v_uv = a_uv;\n}\n",
        fs: "precision mediump float;\nuniform sampler2D u_tex;\nvarying vec2 v_uv;\nvoid main() {\n    gl_FragColor = texture2D(u_tex, v_uv);\n}\n",
        texture: Some(Texture::flat(2, 2, TEXELS.to_vec())),
        expect: core::array::from_fn(|l| TEXELS[row(l) * 2 + col(l)]),
        varyings: vec![s, t],
        ..Shape::bare("texture")
    }
}

/// The four shapes, in P8's order.
pub fn shapes() -> Vec<Shape> {
    vec![passthrough(), uniform(), if_branch(), texture()]
}
