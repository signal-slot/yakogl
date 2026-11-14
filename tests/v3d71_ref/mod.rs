// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 reference side of plan #365 (P8, the 7.1 online compiler):
//! the four P8 shapes as inline GLSL ES 1.00 pairs, a runner for Mesa's
//! `v3d_shc --device 71`, and each shape's inputs and expected outputs in
//! `sim::run_gen(Gen::V71, ..)`'s form.
//!
//! A test file uses it with `mod v3d71_ref;` (a directory module: cargo does
//! not build `tests/v3d71_ref/` as a test of its own). The test then needs
//! `#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]`, and
//! Mesa's side needs the environment named in [`shc_path`] and [`gap`].
//!
//! The shapes are inline on purpose: `tools/shaders/` is the 4.2 corpus
//! tests/v3d_online_corpus.rs pins by count, and a file added there moves it.
//!
//! What is here, in the order of a test's use:
//!
//! * [`SHAPES`]: pass-through, uniform, `if` and texture, each a vertex and
//!   a fragment shader of GLSL ES 1.00.
//! * [`run_shc`]: translate the pair to GL SPIR-V the way
//!   `tools/v3d/glsl2v3d.py` does (`tools/vivante/glsl2vivante.py`'s ES 1.00
//!   translation, then `glslangValidator -G`), run v3d_shc with `--device 71`
//!   and read back the fs/vs/cs words, their uniform streams, the attribute
//!   records, the fragment input slots and the uniform names ([`Mesa71`]).
//!   The Python in between is a short script run with `python3 -I`; the
//!   JSON stays on its side (the crate has no JSON reader).
//! * [`model`]: what a shape computes, per lane, from its inputs ([`Model`]):
//!   the attribute values, the uniform values, the texture, the clip
//!   position, the varyings and the fragment colour. It does not look at any
//!   compiler's output, so the same expectation judges Mesa's code and,
//!   later, ours.
//! * [`vs_rows`], [`fs_inputs`], [`stream`], [`env`]: the sim71 inputs for a
//!   program, from the program's own attribute/varying/uniform layout (Mesa's
//!   from [`Mesa71`]; ours supply theirs).
//! * [`check_vs`], [`check_fs`]: the outcome against the model.
//!
//! The constants and the rules below are those of tests/v3d_sim_mesa.rs,
//! which run Mesa's fixture programs on the same interpreter; where one is
//! copied, it says so.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use yakogl::shaderc::online::v3d::sim::{self, FsInputs, Inputs, Outcome, Varying, VsInputs, LANES};

// ---------------------------------------------------------------------------
// The shapes.
// ---------------------------------------------------------------------------

/// One P8 shape: a GLSL ES 1.00 program, two sources.
pub struct Shape {
    pub name: &'static str,
    pub vs: &'static str,
    pub fs: &'static str,
}

pub const PASSTHROUGH: Shape = Shape {
    name: "passthrough",
    vs: "attribute vec4 a_pos;\n\
         attribute vec4 a_color;\n\
         varying vec4 v_color;\n\
         void main() {\n\
             gl_Position = a_pos;\n\
             v_color = a_color;\n\
         }\n",
    fs: "precision mediump float;\n\
         varying vec4 v_color;\n\
         void main() {\n\
             gl_FragColor = v_color;\n\
         }\n",
};

/// A matrix in the vertex shader and a colour in the fragment shader.
pub const UNIFORM: Shape = Shape {
    name: "uniform",
    vs: "attribute vec4 a_pos;\n\
         uniform mat4 u_mvp;\n\
         void main() {\n\
             gl_Position = u_mvp * a_pos;\n\
         }\n",
    fs: "precision mediump float;\n\
         uniform vec4 u_color;\n\
         void main() {\n\
             gl_FragColor = u_color;\n\
         }\n",
};

/// A fragment `if` on a varying, both sides computing something. The `then`
/// side squares the varying (a product of two varyings): linear arithmetic
/// on a varying (`v * 0.5 + 0.25`) Mesa's `nir_opt_varyings` moves into the
/// vertex shader, so the fragment shader's input is no longer the GLSL
/// varying and the model could not drive it without Mesa's VS (measured:
/// the first version of this shape, 2026-11-14; tools/v3d/README.md says the
/// two stages are optimised together).
pub const IF: Shape = Shape {
    name: "if",
    vs: "attribute vec4 a_pos;\n\
         attribute vec4 a_color;\n\
         varying vec4 v_color;\n\
         void main() {\n\
             gl_Position = a_pos;\n\
             v_color = a_color;\n\
         }\n",
    fs: "precision mediump float;\n\
         varying vec4 v_color;\n\
         void main() {\n\
             if (v_color.x > 0.5) {\n\
                 gl_FragColor = v_color * v_color;\n\
             } else {\n\
                 gl_FragColor = vec4(v_color.y, v_color.z, v_color.x, 1.0);\n\
             }\n\
         }\n",
};

pub const TEXTURE: Shape = Shape {
    name: "texture",
    vs: "attribute vec4 a_pos;\n\
         attribute vec2 a_uv;\n\
         varying vec2 v_uv;\n\
         void main() {\n\
             gl_Position = a_pos;\n\
             v_uv = a_uv;\n\
         }\n",
    fs: "precision mediump float;\n\
         uniform sampler2D u_tex;\n\
         varying vec2 v_uv;\n\
         void main() {\n\
             gl_FragColor = texture2D(u_tex, v_uv);\n\
         }\n",
};

pub const SHAPES: [&Shape; 4] = [&PASSTHROUGH, &UNIFORM, &IF, &TEXTURE];

// ---------------------------------------------------------------------------
// The environment, and the named gap.
// ---------------------------------------------------------------------------

/// The `v3d_shc` binary: `V3D_SHC`, else `MESA_V3D_TREE`'s
/// `build-v3d/src/broadcom/v3d_shc` (tools/v3d/README.md: the Mesa 26.2.1
/// tree with the build-v3d it describes). `Err` names what is missing.
pub fn shc_path() -> Result<PathBuf, String> {
    let (var, path) = if let Some(p) = std::env::var_os("V3D_SHC").filter(|p| !p.is_empty()) {
        ("V3D_SHC", PathBuf::from(p))
    } else if let Some(t) = std::env::var_os("MESA_V3D_TREE").filter(|p| !p.is_empty()) {
        ("MESA_V3D_TREE", PathBuf::from(t).join("build-v3d/src/broadcom/v3d_shc"))
    } else {
        return Err("neither V3D_SHC nor MESA_V3D_TREE is set (tools/v3d/README.md: build-v3d, v3d_shc)".into());
    };
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{var} names {}, which is not a file (ninja -C build-v3d src/broadcom/v3d_shc)", path.display()))
    }
}

/// Print the named gap for `what` and say so loudly. The repository's
/// env-gated tests skip themselves and tools/check.sh names the gap in its
/// own row; this test prints the same words, so a log with no Mesa side in
/// it says it: the output is NOT a comparison with Mesa.
pub fn gap(what: &str, why: &str) {
    println!("GAP {what}: {why}: Mesa's V3D 7.1 code was NOT run and NOT compared (a gap is not a pass)");
}

// ---------------------------------------------------------------------------
// v3d_shc --device 71.
// ---------------------------------------------------------------------------

/// One stage of Mesa's output.
#[derive(Debug, Default, Clone)]
pub struct StageOut {
    pub code: Vec<u64>,
    /// `(contents, data)`: Mesa's `enum quniform_contents` and its datum.
    pub uniforms: Vec<(u32, u32)>,
    pub meta: BTreeMap<String, String>,
    /// The fragment shader's input slots, `(slot, component)`, in LDVARY
    /// order (`slot - 32` is the GLSL location, VARYING_SLOT_VAR0 = 32).
    pub input_slots: Vec<(u32, u32)>,
    /// The vertex and coordinate shaders' VPM input rows per attribute
    /// record (`vattr_sizes`).
    pub vattr: Vec<u32>,
}

/// An `attributes` entry: the name, its location, and per slot the record
/// that fetches it (`None`: the shader does not read the slot).
#[derive(Debug, Clone)]
pub struct AttrOut {
    pub name: String,
    pub location: u32,
    pub records: Vec<Option<u32>>,
}

/// A `uniform_names` entry; `slot` or `binding` (a sampler) is set.
#[derive(Debug, Clone)]
pub struct UniformOut {
    pub stage: String,
    pub name: String,
    pub slot: Option<u32>,
    pub binding: Option<u32>,
    pub vec4s: u32,
}

/// What v3d_shc answered for one pair.
#[derive(Debug, Clone)]
pub struct Mesa71 {
    pub fs: StageOut,
    pub vs: StageOut,
    pub cs: StageOut,
    pub attrs: Vec<AttrOut>,
    pub uniform_names: Vec<UniformOut>,
    /// v3d_shc's listing of the three programs (its stderr), for a failure
    /// message.
    pub disasm: String,
}

impl Mesa71 {
    /// Instruction counts, `[fs, vs, cs]`.
    pub fn counts(&self) -> [usize; 3] {
        [self.fs.code.len(), self.vs.code.len(), self.cs.code.len()]
    }
}

/// Written to a scratch directory and run with `python3 -I -c`: the ES 1.00
/// translation and the glslang step of tools/v3d/glsl2v3d.py, one pair, then
/// v3d_shc, the answer in the line format of tools/v3d/sim_fixture.py
/// (`pair_lines`, minus the spirv `io` lines).
const GLUE: &str = r#"
import json, os, subprocess, sys
repo, shc, vs_src, fs_src, work = sys.argv[1:6]
sys.path.insert(0, os.path.join(repo, "tools", "vivante"))
import glsl2vivante as gv

def spv(src, stage, ext):
    text = gv.translate_es100_gl(open(src).read(), stage)
    gl = os.path.join(work, stage + ext)
    with open(gl, "w") as f:
        f.write(text)
    out = os.path.join(work, stage + ".spv")
    r = subprocess.run([gv.GLSLANG, "-G", gl, "-o", out], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("glslang (%s): %s" % (stage, (r.stdout + r.stderr)[-2000:]))
    return out

vs = spv(vs_src, "vs", ".vert")
fs = spv(fs_src, "fs", ".frag")
r = subprocess.run([shc, vs, fs, "--device", "71", "--vpm-size", "65536"], capture_output=True, text=True)
if r.returncode != 0:
    sys.exit("v3d_shc exit %d: %s" % (r.returncode, r.stderr[-2000:]))
js = json.loads(r.stdout)
for ln in r.stderr.splitlines():
    print("disasm " + ln)
for a in js["attributes"]:
    print("attr %s %d %s" % (a["name"], a["location"], ",".join("-" if x is None else str(x) for x in a["records"])))
for u in js["uniform_names"]:
    where = ("slot=%d" % u["slot"]) if "slot" in u else ("binding=%d" % u["binding"])
    print("uniform %s %s %s %d" % (u["stage"], u["name"], where, u["vec4s"]))
for st in ("fs", "vs", "cs"):
    s = js[st]
    meta = " ".join("%s=%s" % (k, s[k]) for k in sorted(s)
                    if k not in ("code", "uniforms", "input_slots", "vattr_sizes"))
    print("%s meta %s" % (st, meta))
    for sl in s.get("input_slots", []):
        print("%s input %d %d" % (st, sl["slot"], sl["component"]))
    if "vattr_sizes" in s:
        print("%s vattr %s" % (st, ",".join(str(x) for x in s["vattr_sizes"])))
    print("%s code %s" % (st, " ".join(s["code"])))
    print("%s uniforms %s" % (st, " ".join("%d:%s" % (u["contents"], u["data"]) for u in s["uniforms"])))
print("end")
"#;

fn parse_num(s: &str) -> Result<u64, String> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16),
        None => s.parse(),
    }
    .map_err(|e| format!("{s:?}: {e}"))
}

/// Compile one GLSL ES 1.00 pair with v3d_shc at 7.1. `Err` carries the
/// reason (a missing tool, glslang's or v3d_shc's message); a caller that
/// treats a missing V3D_SHC as a gap checks [`shc_path`] first.
pub fn run_shc(tag: &str, vs: &str, fs: &str) -> Result<Mesa71, String> {
    let shc = shc_path()?;
    let work = std::env::temp_dir().join(format!("yakogl-v3d71-ref-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let result = (|| {
        let (vs_path, fs_path, glue) = (work.join("in.vert"), work.join("in.frag"), work.join("glue.py"));
        std::fs::write(&vs_path, vs).map_err(|e| e.to_string())?;
        std::fs::write(&fs_path, fs).map_err(|e| e.to_string())?;
        std::fs::write(&glue, GLUE).map_err(|e| e.to_string())?;
        // -I: no script directory, no user site, no PYTHON* variables on the
        // path of an interpreter that reads files out of a scratch dir.
        let out = Command::new("python3")
            .arg("-I")
            .arg(&glue)
            .arg(env!("CARGO_MANIFEST_DIR"))
            .arg(&shc)
            .arg(&vs_path)
            .arg(&fs_path)
            .arg(&work)
            .output()
            .map_err(|e| format!("python3: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        parse(&String::from_utf8_lossy(&out.stdout))
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn parse(text: &str) -> Result<Mesa71, String> {
    let mut m = Mesa71 {
        fs: StageOut::default(),
        vs: StageOut::default(),
        cs: StageOut::default(),
        attrs: Vec::new(),
        uniform_names: Vec::new(),
        disasm: String::new(),
    };
    let mut ended = false;
    for line in text.lines() {
        if let Some(d) = line.strip_prefix("disasm ") {
            m.disasm.push_str(d);
            m.disasm.push('\n');
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["end"] => ended = true,
            ["attr", name, loc, recs] => m.attrs.push(AttrOut {
                name: name.to_string(),
                location: loc.parse().map_err(|e| format!("{line}: {e}"))?,
                records: recs
                    .split(',')
                    .map(|r| if r == "-" { Ok(None) } else { r.parse().map(Some) })
                    .collect::<Result<_, _>>()
                    .map_err(|e| format!("{line}: {e}"))?,
            }),
            ["uniform", stage, name, where_, vec4s] => {
                let (k, v) = where_.split_once('=').ok_or_else(|| format!("{line}: no slot or binding"))?;
                let v: u32 = v.parse().map_err(|e| format!("{line}: {e}"))?;
                m.uniform_names.push(UniformOut {
                    stage: stage.to_string(),
                    name: name.to_string(),
                    slot: (k == "slot").then_some(v),
                    binding: (k == "binding").then_some(v),
                    vec4s: vec4s.parse().map_err(|e| format!("{line}: {e}"))?,
                });
            }
            [st @ ("fs" | "vs" | "cs"), what, rest @ ..] => {
                let s = match *st {
                    "fs" => &mut m.fs,
                    "vs" => &mut m.vs,
                    _ => &mut m.cs,
                };
                match *what {
                    "meta" => {
                        for kv in rest {
                            if let Some((k, v)) = kv.split_once('=') {
                                s.meta.insert(k.to_string(), v.to_string());
                            }
                        }
                    }
                    "input" => s.input_slots.push((parse_num(rest[0])? as u32, parse_num(rest[1])? as u32)),
                    "vattr" => {
                        s.vattr = rest[0]
                            .split(',')
                            .map(|x| x.parse().map_err(|e| format!("{line}: {e}")))
                            .collect::<Result<_, _>>()?
                    }
                    "code" => s.code = rest.iter().map(|w| parse_num(w)).collect::<Result<_, _>>()?,
                    "uniforms" => {
                        s.uniforms = rest
                            .iter()
                            .map(|cd| {
                                let (c, d) = cd.split_once(':').ok_or_else(|| format!("{cd}: no ':'"))?;
                                Ok((parse_num(c)? as u32, parse_num(d)? as u32))
                            })
                            .collect::<Result<_, String>>()?
                    }
                    other => return Err(format!("unknown line {other:?}: {line}")),
                }
            }
            [] => {}
            _ => return Err(format!("unknown line: {line}")),
        }
    }
    if !ended || m.fs.code.is_empty() || m.vs.code.is_empty() || m.cs.code.is_empty() {
        return Err("v3d_shc's answer is incomplete (no end line, or a stage without code)".into());
    }
    Ok(m)
}

// ---------------------------------------------------------------------------
// What each shape computes.
// ---------------------------------------------------------------------------

/// The viewport and depth transform v3d_shc's QUNIFORM_VIEWPORT_* contents
/// (2..5) stand for, a window of 1024 x 768: copied from
/// tests/v3d_sim_mesa.rs (X_SCALE .. Z_OFFSET).
pub const X_SCALE: f32 = 8192.0;
pub const Y_SCALE: f32 = -6144.0;
pub const Z_SCALE: f32 = 0.5;
pub const Z_OFFSET: f32 = 0.5;
/// Where the harness puts the uniform buffer a QUNIFORM_UBO_ADDR names.
pub const UNIFORM_BASE: u32 = 0x1000_0000;
/// Texture units v3d_sim_mesa.rs binds; a sampler binding is a unit.
pub const TEXTURE_UNITS: u32 = 16;
pub const TEXTURE_SIZE: u32 = 5;

/// The per-lane model of one shape. Lanes are independent vertices (VS, CS)
/// or pixels (FS): lane `l` of the FS reads the varyings the VS writes in
/// lane `l`.
pub struct Model {
    /// Attribute name and its value per lane (a vec2 uses the first two).
    pub attrs: Vec<(&'static str, [[f32; 4]; LANES])>,
    /// Uniform name and its components, in the order the GLSL type lays them
    /// out in vec4 slots (a `mat4` is its 16 floats, column-major).
    pub uniforms: Vec<(&'static str, Vec<f32>)>,
    /// The texture bound to the sampler of this binding (and every other
    /// unit: [`env`]), if the shape samples.
    pub texture: Option<(u32, sim::Texture)>,
    /// gl_Position per lane.
    pub position: [[f32; 4]; LANES],
    /// The varyings by GLSL location (`slot - 32`), per lane. A `vec2`
    /// uses the first two components.
    pub varyings: Vec<(u32, [[f32; 4]; LANES])>,
    /// gl_FragColor per lane.
    pub colour: [[f32; 4]; LANES],
    /// The names the shape's varyings go by, location order, for messages.
    pub varying_names: Vec<&'static str>,
}

/// Texel (i, j) of the shape's 5 x 5 texture: every channel exact in f16.
fn texel(i: u32, j: u32) -> [f32; 4] {
    let n = (i + 5 * j) as f32;
    [(i + 1) as f32 / 8.0, (j + 1) as f32 / 8.0, n / 32.0, 1.0 - n / 64.0]
}

/// A lane's colour: multiples of 1/16 in [0, 1], distinct per lane, `.x`
/// crossing 0.5 in the middle of the 16 lanes.
fn lane_colour(l: usize) -> [f32; 4] {
    let l = l as f32;
    [l / 16.0, (15.0 - l) / 16.0, ((l * 3.0) % 16.0) / 16.0, 1.0 - l / 32.0]
}

/// A lane's position: x and y on a 1/4 grid, z in (0, 1), w = 1, so the
/// scaled coordinates are exact whatever the compiler's order of operations.
fn lane_position(l: usize) -> [f32; 4] {
    [((l % 4) as f32 - 1.5) / 4.0, ((l / 4) as f32 - 1.5) / 4.0, (l as f32 + 1.0) / 32.0, 1.0]
}

pub fn model(shape: &Shape) -> Model {
    let pos: [[f32; 4]; LANES] = core::array::from_fn(lane_position);
    let col: [[f32; 4]; LANES] = core::array::from_fn(lane_colour);
    match shape.name {
        "passthrough" => Model {
            attrs: vec![("a_pos", pos), ("a_color", col)],
            uniforms: vec![],
            texture: None,
            position: pos,
            varyings: vec![(0, col)],
            colour: col,
            varying_names: vec!["v_color"],
        },
        "if" => {
            let colour = core::array::from_fn(|l| {
                let v = col[l];
                if v[0] > 0.5 {
                    [v[0] * v[0], v[1] * v[1], v[2] * v[2], v[3] * v[3]]
                } else {
                    [v[1], v[2], v[0], 1.0]
                }
            });
            Model {
                attrs: vec![("a_pos", pos), ("a_color", col)],
                uniforms: vec![],
                texture: None,
                position: pos,
                varyings: vec![(0, col)],
                colour,
                varying_names: vec!["v_color"],
            }
        }
        "uniform" => {
            // x' = 2x + 1/4, y' = y/2 - 1/4, z, w unchanged; column-major.
            #[rustfmt::skip]
            let mvp = vec![
                2.0, 0.0, 0.0, 0.0,
                0.0, 0.5, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                0.25, -0.25, 0.0, 1.0,
            ];
            let u_color = vec![0.25, 0.5, 0.75, 1.0];
            let position = core::array::from_fn(|l| {
                let p = pos[l];
                [2.0 * p[0] + 0.25, 0.5 * p[1] - 0.25, p[2], p[3]]
            });
            Model {
                attrs: vec![("a_pos", pos)],
                uniforms: vec![("u_mvp", mvp), ("u_color", u_color.clone())],
                texture: None,
                position,
                varyings: vec![],
                colour: [[u_color[0], u_color[1], u_color[2], u_color[3]]; LANES],
                varying_names: vec![],
            }
        }
        "texture" => {
            let texels: Vec<[f32; 4]> = (0..TEXTURE_SIZE * TEXTURE_SIZE).map(|n| texel(n % 5, n / 5)).collect();
            // The centre of texel (i, j), i and j distinct per lane; NEAREST
            // reads texel (i, j) a long way from every edge.
            let cell = |l: usize| ((l % 5) as u32, ((l / 5) % 5) as u32);
            let uv: [[f32; 4]; LANES] = core::array::from_fn(|l| {
                let (i, j) = cell(l);
                [(i as f32 + 0.5) / 5.0, (j as f32 + 0.5) / 5.0, 0.0, 0.0]
            });
            let colour = core::array::from_fn(|l| {
                let (i, j) = cell(l);
                texel(i, j)
            });
            Model {
                attrs: vec![("a_pos", pos), ("a_uv", uv)],
                uniforms: vec![],
                texture: Some((0, sim::Texture::flat(TEXTURE_SIZE, TEXTURE_SIZE, texels))),
                position: pos,
                varyings: vec![(0, uv)],
                colour,
                varying_names: vec!["v_uv"],
            }
        }
        other => panic!("v3d71_ref: no model for shape {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Inputs in sim71's form.
// ---------------------------------------------------------------------------

fn bits(x: f32) -> u32 {
    x.to_bits()
}

/// The VPM input rows of a vertex or coordinate shader: for each attribute
/// record in record order, `(attribute name, rows it takes)`. Mesa's list
/// comes from [`Mesa71`] ([`mesa_attr_order`]); a program of ours gives its
/// own. The shapes use no vertex or instance id (the rows for them, which
/// come first, are not made here).
pub fn vs_rows(m: &Model, order: &[(&str, usize)]) -> Result<Inputs, String> {
    let mut rows = Vec::new();
    for (name, n) in order {
        let (_, vals) =
            m.attrs.iter().find(|(a, _)| a == name).ok_or_else(|| format!("attribute {name} is not in the model"))?;
        for c in 0..*n {
            rows.push(core::array::from_fn(|l| bits(vals[l][c])));
        }
    }
    Ok(Inputs::Vertex(VsInputs { vpm: rows }))
}

/// `(name, rows)` per Mesa attribute record of `stage`, in record order.
pub fn mesa_attr_order<'a>(mesa: &'a Mesa71, stage: &StageOut) -> Result<Vec<(&'a str, usize)>, String> {
    for key in ["uses_iid", "uses_vid"] {
        if stage.meta.get(key).map(String::as_str) == Some("True") {
            return Err(format!("the program reads {key}: the shapes do not"));
        }
    }
    let mut out = Vec::new();
    for (rec, &size) in stage.vattr.iter().enumerate() {
        if size == 0 {
            continue;
        }
        let a = mesa
            .attrs
            .iter()
            .find(|a| a.records.contains(&Some(rec as u32)))
            .ok_or_else(|| format!("VPM record {rec} belongs to no attribute"))?;
        out.push((a.name.as_str(), size as usize));
    }
    Ok(out)
}

/// Fragment inputs: W a power of two that differs per lane (so the varying
/// construction below is exact and a W mix-up shows), Z, the pixel
/// coordinates, front-facing, and for each fragment input `i` of `slots`
/// (`(location, component)`, LDVARY order) a smooth varying: `vary = (v/2)/W`,
/// `C = v/2`, which a program rebuilds as `vary * W + C = v` (the
/// construction of tests/v3d_sim_mesa.rs, for 7.1's rf3 payload W
/// included: [`FsInputs`] carries it).
pub fn fs_inputs(m: &Model, slots: &[(u32, u32)]) -> Result<Inputs, String> {
    let pow2 = [0.5f32, 1.0, 2.0, 4.0];
    let w: [u32; LANES] = core::array::from_fn(|l| bits(pow2[l % 4]));
    let wc: [u32; LANES] = core::array::from_fn(|l| bits(pow2[(l + 1) % 4]));
    let z: [u32; LANES] = core::array::from_fn(|l| bits((l as f32 + 1.0) / 64.0));
    let x: [u32; LANES] = core::array::from_fn(|l| bits((l % 4) as f32 + 0.5));
    let y: [u32; LANES] = core::array::from_fn(|l| bits((l / 4) as f32 + 0.5));
    let mut varyings = Vec::new();
    for &(loc, comp) in slots {
        let (_, vals) = m
            .varyings
            .iter()
            .find(|(k, _)| *k == loc)
            .ok_or_else(|| format!("the fragment shader reads varying location {loc}, which the model has not"))?;
        let mut v = Varying { vary: [0; LANES], c: [0; LANES] };
        for l in 0..LANES {
            let value = vals[l][comp as usize];
            let half = value * 0.5;
            let wl = f32::from_bits(w[l]);
            let (pv, cv) = (half / wl, half);
            if pv * wl + cv != value {
                return Err(format!("the varying construction is not exact for {value}"));
            }
            v.vary[l] = bits(pv);
            v.c[l] = bits(cv);
        }
        varyings.push(v);
    }
    Ok(Inputs::Fragment(Box::new(FsInputs { w, w_centroid: wc, z, x, y, back_facing: [false; LANES], varyings })))
}

/// Resolve Mesa's uniform stream of `stage` (`"vert"` or `"frag"`) to words:
/// the contents tests/v3d_sim_mesa.rs's `stream` resolves for a Mesa program,
/// with the uniform values of the model. Contents it does not know are an
/// `Err` naming them, never a guessed word.
pub fn stream(m: &Model, mesa: &Mesa71, st: &StageOut, stage: &str) -> Result<Vec<u32>, String> {
    let mut out = Vec::new();
    for &(contents, data) in &st.uniforms {
        out.push(match contents {
            0 => data,
            1 => {
                let (slot, name) = mesa
                    .uniform_names
                    .iter()
                    .filter(|u| u.stage == stage)
                    .find_map(|u| {
                        let s = u.slot?;
                        (data >= s * 4 && data < (s + u.vec4s) * 4).then_some((s, u.name.as_str()))
                    })
                    .ok_or_else(|| format!("{stage}: QUNIFORM_UNIFORM {data} is in no uniform's slots"))?;
                let (_, vals) = m
                    .uniforms
                    .iter()
                    .find(|(n, _)| *n == name)
                    .ok_or_else(|| format!("{stage}: uniform {name} is not in the model"))?;
                let v = vals
                    .get((data - slot * 4) as usize)
                    .ok_or_else(|| format!("{stage}: uniform {name}[{}] is past its value", data - slot * 4))?;
                bits(*v)
            }
            2 => bits(X_SCALE),
            3 => bits(Y_SCALE),
            4 => bits(Z_OFFSET),
            5 => bits(Z_SCALE),
            // QUNIFORM_TMU_CONFIG_P0/P1: the texture or sampler unit in
            // bits 24..31, passed unrelocated, as the interpreter takes it.
            40 | 41 if data >> 24 < TEXTURE_UNITS => data,
            // QUNIFORM_TEXTURE_WIDTH/HEIGHT of a unit.
            44 => TEXTURE_SIZE,
            45 => TEXTURE_SIZE,
            c => return Err(format!("{stage}: uniform contents {c} (data {data:#x}) is not modelled by this harness")),
        });
    }
    Ok(out)
}

/// The interpreter's environment: the model's texture on every unit (the
/// program names one by its P0 unit, which is the sampler's binding), the
/// program's thread count (`threads` in v3d_shc's prog_data).
pub fn env(m: &Model, st: &StageOut) -> sim::Env {
    let threads: u32 = st.meta.get("threads").and_then(|t| t.parse().ok()).unwrap_or(0);
    let textures = match &m.texture {
        Some((_, t)) => (0..TEXTURE_UNITS).map(|_| t.clone()).collect(),
        None => Vec::new(),
    };
    sim::Env {
        memory: vec![sim::Region { base: UNIFORM_BASE, words: Vec::new() }],
        textures,
        samplers: Vec::new(),
        thread: threads.max(1) - 1,
        threads,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Expected outputs.
// ---------------------------------------------------------------------------

fn close_f32(want: f32, got: f32, approx: bool) -> bool {
    want.to_bits() == got.to_bits() || (approx && (want - got).abs() <= want.abs().max(got.abs()) * 1e-5)
}

/// The fixed-function rows of a VS (scaled x, scaled y, Zs, 1/Wc) or a CS
/// (clip x y z w, scaled x y), then the varyings of `slots` in the FS's
/// input order, against the model. Returns how many values were compared;
/// every row must be written. The scaled coordinates round to nearest even
/// on 7.1 (v3d_nir_lower_io.c:529-543, as tests/v3d_sim_mesa.rs has it).
/// An SFU operation in the run (the 1/W is one) loosens the float
/// comparisons to 1e-5 and the scaled coordinates to one unit.
pub fn check_vs(m: &Model, o: &Outcome, is_cs: bool, slots: &[(u32, u32)]) -> Result<usize, String> {
    let approx = !o.approximate.is_empty();
    let row = |r: usize| -> Result<[u32; LANES], String> {
        o.vpm_out.get(r).copied().flatten().ok_or_else(|| format!("VPM row {r} was never written"))
    };
    let mut n = 0;
    for l in 0..LANES {
        let p = m.position[l];
        let rcp = 1.0 / p[3];
        let sx = ((p[0] * X_SCALE) * rcp).round_ties_even();
        let sy = ((p[1] * Y_SCALE) * rcp).round_ties_even();
        let mut want: Vec<(usize, f32, bool, &str)> = Vec::new();
        let base = if is_cs {
            for (c, v) in p.iter().enumerate() {
                want.push((c, *v, false, "clip position"));
            }
            4
        } else {
            0
        };
        want.push((base, sx, true, "scaled x"));
        want.push((base + 1, sy, true, "scaled y"));
        if !is_cs {
            want.push((2, (p[2] * Z_SCALE) * rcp + Z_OFFSET, false, "Zs"));
            want.push((3, rcp, false, "1/Wc"));
        }
        for (r, w, int, what) in want {
            let g = row(r)?[l];
            n += 1;
            let ok = if int {
                let g = g as i32;
                (g as f32 == w) || (approx && (g as f32 - w).abs() <= 1.0)
            } else {
                close_f32(w, f32::from_bits(g), approx)
            };
            if !ok {
                let shown = if int { format!("{}", g as i32) } else { format!("{}", f32::from_bits(g)) };
                return Err(format!("lane {l} row {r} ({what}): want {w}, got {shown} ({g:#010x})"));
            }
        }
        if !is_cs {
            for (i, &(loc, comp)) in slots.iter().enumerate() {
                let (_, vals) = m
                    .varyings
                    .iter()
                    .find(|(k, _)| *k == loc)
                    .ok_or_else(|| format!("FS input {i}: no varying at location {loc} in the model"))?;
                let w = vals[l][comp as usize];
                let g = f32::from_bits(row(4 + i)?[l]);
                n += 1;
                if !close_f32(w, g, false) {
                    return Err(format!(
                        "lane {l} row {} (FS input {i}, location {loc}.{comp}): want {w}, got {g}",
                        4 + i
                    ));
                }
            }
        }
    }
    Ok(n)
}

/// The fragment colour against the model: Mesa writes RT 0 as two f16 pairs
/// (`vfpack(r, g)`, `vfpack(b, a)`), two TLB writes under the default
/// specifier; a depth write (specifier type 2) is skipped, any other
/// specifier is an `Err`. No lane may be discarded. Returns the number of
/// values compared; f16 words are compared exactly unless the run executed
/// an SFU operation (then within one ulp).
pub fn check_fs(m: &Model, o: &Outcome) -> Result<usize, String> {
    let approx = !o.approximate.is_empty();
    let mut spec = 0xffu32;
    let mut words: Vec<[u32; LANES]> = Vec::new();
    for t in &o.tlb {
        if let Some(c) = t.config {
            spec = c & 0xff;
        }
        if spec >> 6 == 2 {
            spec = 0xff;
            continue;
        }
        if spec >> 6 != 3 || spec & 0b111 != 0b111 || 7 - ((spec >> 3) & 7) != 0 {
            return Err(format!("TLB specifier {spec:#x}: not an RT 0 f16 colour write"));
        }
        if t.value.iter().zip(o.msf.iter()).any(|(v, k)| v.is_none() && *k != 0) {
            return Err("a colour TLB write of an undefined value".into());
        }
        words.push(t.value.map(|v| v.unwrap_or(0)));
    }
    if words.len() != 2 {
        return Err(format!("{} colour TLB writes, want 2", words.len()));
    }
    let mut n = 0;
    for l in 0..LANES {
        if o.msf[l] == 0 {
            return Err(format!("lane {l} was discarded"));
        }
        for k in 0..4 {
            let word = words[k / 2][l];
            let g = if k % 2 == 0 { word as u16 } else { (word >> 16) as u16 };
            let w = sim::f32_to_half(m.colour[l][k]);
            n += 1;
            let ok = w == g || (approx && (i32::from(w) - i32::from(g)).abs() <= 1);
            if !ok {
                return Err(format!(
                    "lane {l} colour[{k}]: want {} (f16 {w:#06x}), got {} ({g:#06x})",
                    m.colour[l][k],
                    sim::half_to_f32(g)
                ));
            }
        }
    }
    Ok(n)
}
