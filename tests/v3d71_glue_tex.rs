// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/glue71.rs` on a textured draw: the draw of Mesa's 7.1
//! reference tools/v3d/pi5/gl_tex2d.clif (two jobs: a 64x64 `GL_RGBA8`
//! texture sampled over the 64x64 window, NEAREST, then LINEAR; the GL calls
//! are tools/v3d/pi5/src/gl_shapes.py `tex2d`, as tools/v3d/pi5/README.md
//! "The GL calls" lists them).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_glue_tex
//! ```
//!
//! The program is ours: gl_test's ES 1.00 UV pair
//! (tools/shaders/gl_test_es100/uv.vert and texture.frag, the twins of
//! `gl_test_uv_vs_src` and `gl_test_texture_fs_src`) linked through
//! `link_sources71`, put where Mesa put its code and uniform streams (the
//! table's placement, read from the dump). The GlesDraw is the one the GL
//! core resolves for `glClear(GL_COLOR_BUFFER_BIT)` and
//! `glDrawArrays(GL_TRIANGLES, 0, 6)` with the 24-byte `puv` vertex (pos4 at
//! location 0, uv2 at location 1) and the texture on unit 0.
//!
//! What `glue71::ops` and `fill71::fill` do (pinned): `ops` refuses any draw
//! that samples a texture (`REF_TEXTURES`, `GLUE71_REFUSALS[4]`) before
//! mapping it; with the texture taken out of the GlesDraw, `fill` refuses
//! our program's TMU configuration words by uniform71's name; and the
//! `v3d_job71_draw` `fill` hands over has no TMU table (`tmu` null,
//! `num_tmu` 0). Through that path, what is compared with the clif is:
//!
//! * the shader record of the same draw with our program's texture words
//!   left out of its fragment stream (every field the clif prints, by name;
//!   the three fields that differ from Mesa's are the triangle program's
//!   known three, pinned in [`RECORD_DIFFS`]);
//! * the attribute records glue71's arrays give (attr71's plan; every field);
//! * the call (`VERTEX_ARRAY_PRIMS`) and the fused clear's colour;
//! * the texture uniforms: our fragment stream's TMU Config Parameter 0 and
//!   1, resolved by texuni71 with the record addresses Mesa used, are Mesa's
//!   words.
//!
//! The textured draw goes through `glue71::ops_textured` (the TMU table the
//! table placed, `glue71::TexInputs`) and `fill71::fill_textured`
//! ([`the_textured_draw_fills_through_glue71`]): it fills, the fragment
//! stream v3d_job71 writes from the program's list and the table is Mesa's
//! (P0 and P1), the shader record is the untextured draw's, and
//! `v3d_job71_draw` gets one TMU entry for unit 0. The tables are checked by
//! name ([`the_textured_draws_tables_are_checked_by_name`]); a draw without
//! textures fills through that path as through `fill`.
//!
//! The program is linked by the online compiler's 7.1 back end, so the file
//! builds only with `shaderc-online` and `gpu-v3d` (the gate's other feature
//! sets compile it to nothing).
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]
#![allow(dead_code)]

/// The GL core's back-end types, as every test reaches them: glue71 reads
/// `crate::backend`.
mod backend {
    pub use yakogl::backend::*;
}

#[path = "../src/v3d/admit71.rs"]
mod admit71;
#[path = "../src/v3d/attr71.rs"]
#[allow(clippy::if_same_then_else)]
mod attr71;
#[path = "../src/v3d/clearplan71.rs"]
mod clearplan71;
#[path = "../src/v3d/draw71.rs"]
mod draw71;
#[path = "../src/v3d/ffi71.rs"]
mod ffi71;
#[path = "../src/v3d/fill71.rs"]
mod fill71;
#[path = "../src/v3d/format71.rs"]
mod format71;
#[path = "../src/v3d/glue71.rs"]
mod glue71;
#[path = "../src/v3d/ident71.rs"]
mod ident71;
#[path = "../src/v3d/limits71.rs"]
mod limits71;
#[path = "../src/v3d/prim71.rs"]
mod prim71;
#[path = "../src/v3d/record71.rs"]
mod record71;
#[path = "../src/v3d/rt71.rs"]
#[allow(clippy::eq_op)]
mod rt71;
#[path = "../src/v3d/texuni71.rs"]
mod texuni71;
#[path = "../src/v3d/uniform71.rs"]
mod uniform71;
#[path = "../src/v3d/zskeep71.rs"]
mod zskeep71;

use backend::*;
use fill71::{ClearQuad, FilledJob, History, Op, Program};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use yakogl::shaderc::online::store;
use yakogl::shaderc::online::v3d::link71::link_sources71;
use yakogl::shaderc::{LodKey, ShcV3dUniform};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const CLIF: &str = "tools/v3d/pi5/gl_tex2d.clif";

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn num(s: &str) -> u32 {
    let s = s.trim();
    match s.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16).unwrap_or_else(|e| panic!("{s}: {e}")),
        None => s.parse().unwrap_or_else(|e| panic!("{s}: {e}")),
    }
}

/* ---- the clif, as text ------------------------------------------------------- */

/// One job of the dump: what the draw's records say.
#[derive(Debug, Default)]
struct Job {
    /// `shadrec_gl_main`, field name to value (addresses: the address).
    record: BTreeMap<String, u32>,
    /// `shadrec_gl_attr` 0, 1, .. by field name.
    attrs: Vec<BTreeMap<String, u32>>,
    /// Every `@format binary` word by its GPU address.
    mem: BTreeMap<u32, u32>,
    /// `VERTEX_ARRAY_PRIMS`: mode, length, first vertex.
    prims: Vec<(u32, u32, u32)>,
    /// `Clear Color low bits` of render target 0.
    clear_low: Option<u32>,
}

impl Job {
    fn f(&self, name: &str) -> u32 {
        *self.record.get(name).unwrap_or_else(|| panic!("record field {name}"))
    }
    /// The fragment stream: from its address to the vertex stream's.
    fn fs_stream(&self) -> Vec<u32> {
        let (a, b) = (self.f("Fragment Shader Uniforms Address"), self.f("Vertex Shader Uniforms Address"));
        (a..b).step_by(4).map(|x| self.mem[&x]).collect()
    }
}

/// `/* name: */ value ...` into (name, value); an address `[buf+off] /* 0x.. */`
/// is the address in its comment.
fn field(line: &str) -> Option<(String, u32)> {
    let t = line.trim().strip_prefix("/*")?;
    let (name, rest) = t.split_once(": */")?;
    let rest = rest.trim();
    let v = if rest.starts_with('[') {
        let c = rest.split("/*").nth(1)?;
        num(c.trim().trim_end_matches("*/"))
    } else {
        let tok = rest.split_whitespace().next()?;
        if tok.contains('.') {
            return None; /* floats: not read here */
        }
        tok.parse::<u64>().ok()? as u32
    };
    Some((name.trim().to_string(), v))
}

fn jobs() -> &'static Vec<Job> {
    static J: OnceLock<Vec<Job>> = OnceLock::new();
    J.get_or_init(|| {
        let text = read(CLIF);
        let mut out: Vec<Job> = Vec::new();
        let mut sect = String::new();
        let mut base = 0u32;
        let mut in_prims = false;
        let mut prim = Vec::new();
        for raw in text.lines() {
            let l = raw.strip_prefix("MESA: info: ").unwrap_or(raw);
            if l.starts_with("@createbuf_aligned") && !sect.starts_with("@createbuf") {
                out.push(Job::default());
            }
            let job = out.last_mut().expect("a job");
            if let Some(f) = l.strip_prefix("@format ") {
                sect = l.to_string();
                if f.starts_with("binary") {
                    /* `[NAME_0x<base>+0x<off>]` */
                    let at = f.split('[').nth(1).unwrap();
                    let (buf, off) = at.split_once('+').unwrap();
                    let off = off.split(']').next().unwrap();
                    base = num(buf.rsplit('_').next().unwrap()) + num(off);
                } else if f.starts_with("shadrec_gl_attr") {
                    job.attrs.push(BTreeMap::new());
                }
                in_prims = false;
                continue;
            }
            if l.starts_with('@') {
                sect = l.to_string();
                continue;
            }
            if sect.starts_with("@format binary") {
                for w in l.split_whitespace() {
                    job.mem.insert(base, num(w));
                    base += 4;
                }
            } else if sect.starts_with("@format shadrec_gl_main") {
                if let Some((n, v)) = field(l) {
                    job.record.insert(n, v);
                }
            } else if sect.starts_with("@format shadrec_gl_attr") {
                if let Some((n, v)) = field(l) {
                    job.attrs.last_mut().unwrap().insert(n, v);
                }
            } else if sect.starts_with("@format ctrllist") {
                if !l.starts_with(' ') && !l.is_empty() {
                    in_prims = l == "VERTEX_ARRAY_PRIMS";
                    continue;
                }
                if let Some((n, v)) = field(l) {
                    if in_prims {
                        prim.push(v);
                        if prim.len() == 3 {
                            job.prims.push((prim[0], prim[1], prim[2]));
                            prim.clear();
                        }
                    }
                    if n == "Clear Color low bits" {
                        job.clear_low = Some(v);
                    }
                }
            }
        }
        out
    })
}

/* ---- our textured program ---------------------------------------------------- */

const UV_VS: &str = include_str!("../tools/shaders/gl_test_es100/uv.vert");
const TEXTURE_FS: &str = include_str!("../tools/shaders/gl_test_es100/texture.frag");

/// What the link gives, copied out of the keep.
#[derive(Clone, Debug)]
struct Linked {
    fs: Vec<(u32, u32)>,
    vs: Vec<(u32, u32)>,
    cs: Vec<(u32, u32)>,
    record_of_location: [u8; 16],
    desc: record71::DrawDesc,
    cs_reads: [u8; 16],
    vs_reads: [u8; 16],
    cs_vcm: u32,
    vs_vcm: u32,
    separate: bool,
    varyings: fill71::VaryingFlags,
}

fn stream(u: *const ShcV3dUniform, n: u32) -> Vec<(u32, u32)> {
    if n == 0 {
        return Vec::new();
    }
    // SAFETY: the linked record's stream of `n` entries, in the open keep.
    unsafe { core::slice::from_raw_parts(u, n as usize) }.iter().map(|u| (u.contents, u.data)).collect()
}

/// gl_test's UV pair linked at 7.1, its code and streams where Mesa's are
/// (`code` and `uniforms` of each stage from the first job's record).
fn linked() -> &'static Linked {
    static L: OnceLock<Linked> = OnceLock::new();
    L.get_or_init(|| {
        let j = &jobs()[0];
        let _keep = store::begin_keep().expect("keep");
        store::with_scratch(|a| {
            let (sh, _) =
                link_sources71(UV_VS.as_bytes(), None, LodKey::NONE, TEXTURE_FS.as_bytes(), &[], a, store::keep())
                    .unwrap_or_else(|e| panic!("the UV pair links at 7.1: {e:?}"));
            let p = sh.v3d;
            assert_eq!(p.linked, 1);
            let b = |v: u8| v != 0;
            let prog = |code: &str, unif: &str, threads: u8, single: u8| record71::ProgramDesc {
                code: j.f(code),
                uniforms: j.f(unif),
                threads: u32::from(threads),
                single_seg: b(single),
                propagate_nans: false,
            };
            let vdesc = |v: &yakogl::shaderc::ShcV3dVs, code: &str, unif: &str| record71::VertexDesc {
                prog: prog(code, unif, v.threads, v.single_seg),
                vpm_input_size: v.vpm_input_size,
                vpm_output_size: v.vpm_output_size,
                min_input_segments: 1,
                min_output_segments: 0,
                uses_vid: b(v.uses_vid),
                uses_iid: b(v.uses_iid),
                uses_base_instance: b(v.uses_biid),
                uses_base_vertex: false,
                uses_draw_index: false,
            };
            let mut desc = record71::DrawDesc::default();
            desc.fs.prog =
                prog("Fragment Shader Code Address", "Fragment Shader Uniforms Address", p.fs.threads, p.fs.single_seg);
            desc.fs.writes_z = b(p.fs.writes_z);
            desc.fs.disable_ez = b(p.fs.disable_ez);
            desc.fs.num_varyings = p.fs.num_inputs;
            desc.fs.uses_center_w = b(p.fs.uses_center_w);
            desc.fs.uses_pid = b(p.fs.uses_pid);
            desc.fs.lock_scoreboard_on_first_thrsw = b(p.fs.lock_scoreboard_on_first_thrsw);
            desc.fs.uses_implicit_point_line_varyings = b(p.fs.uses_implicit_point_line_varyings);
            desc.vs = vdesc(&p.vs, "Vertex Shader Code Address", "Vertex Shader Uniforms Address");
            desc.cs = vdesc(&p.cs, "Coordinate Shader Code Address", "Coordinate Shader Uniforms Address");
            desc.point_size = b(p.vs.writes_psiz);
            Linked {
                fs: stream(p.fs.uniforms, p.fs.num_uniforms),
                vs: stream(p.vs.uniforms, p.vs.num_uniforms),
                cs: stream(p.cs.uniforms, p.cs.num_uniforms),
                record_of_location: p.record_of_location,
                desc,
                cs_reads: p.cs.vattr_sizes,
                vs_reads: p.vs.vattr_sizes,
                cs_vcm: p.cs.vcm_cache_size,
                vs_vcm: p.vs.vcm_cache_size,
                separate: b(p.vs.separate_segments) || b(p.cs.separate_segments),
                varyings: fill71::VaryingFlags {
                    flat: [p.fs.flat_shade_flags, p.fs.flat_shade_flags_hi],
                    noperspective: [p.fs.noperspective_flags, p.fs.noperspective_flags_hi],
                    centroid: [p.fs.centroid_flags, p.fs.centroid_flags_hi],
                },
            }
        })
        .expect("scratch")
    })
}

fn is_texture(contents: u32) -> bool {
    (texuni71::TMU_CONFIG_P0..=texuni71::TEXTURE_ARRAY_SIZE).contains(&contents)
}

fn words(v: &[(u32, u32)], keep_textures: bool) -> &'static [uniform71::Word] {
    let w: Vec<uniform71::Word> = v
        .iter()
        .filter(|(c, _)| keep_textures || !is_texture(*c))
        .map(|&(contents, data)| uniform71::Word { contents, data })
        .collect();
    Box::leak(w.into_boxed_slice())
}

/// Our program as fill71 takes it; `textures` false leaves the fragment
/// stream's texture words out.
fn program(textures: bool) -> &'static Program<'static> {
    static WITH: OnceLock<Program<'static>> = OnceLock::new();
    static WITHOUT: OnceLock<Program<'static>> = OnceLock::new();
    let make = || {
        let l = linked();
        Program {
            desc: l.desc,
            cs_reads: l.cs_reads,
            vs_reads: l.vs_reads,
            cs_vcm_cache_size: l.cs_vcm,
            vs_vcm_cache_size: l.vs_vcm,
            separate_segments: l.separate,
            varyings: l.varyings,
            discards: l.desc.fs.disable_ez,
            fs_uniforms: words(&l.fs, textures),
            vs_uniforms: words(&l.vs, true),
            cs_uniforms: words(&l.cs, true),
        }
    };
    if textures {
        WITH.get_or_init(make)
    } else {
        WITHOUT.get_or_init(make)
    }
}

/* ---- the GL core's GlesDraw of gl_tex2d's draw --------------------------------- */

const G_RT: u32 = 0x3000; /* Mesa's colour target, resource_0x3000 */
const G_VTX: u32 = 0xe000; /* Mesa's vertex buffer, resource_0xe000 */
const G_TEX: u32 = 0xf000; /* Mesa's texture, resource_0xf000 */
const G_ZS: u32 = 0x2000000;
const G_TILE: u32 = 0x15000;
const SIZE: u32 = 64;

/// The texture bound to unit 0: 64x64 RGBA8, level 0, NEAREST (`linear`
/// false) or LINEAR, REPEAT.
fn texture(linear: bool) -> GlesTextureBind {
    let mut t = GlesTextureBind::ZERO;
    t.unit = 0;
    t.format = GLES_TEXFMT_RGBA8;
    t.type_ = GLES_TEXTYPE_2D;
    t.layers = 1;
    t.base_gpu = u64::from(G_TEX);
    (t.width, t.height, t.stride_texels, t.bytes) = (SIZE, SIZE, SIZE, SIZE * SIZE * 4);
    (t.linear_filter, t.min_linear) = (u8::from(linear), u8::from(linear));
    t.mip_levels = 1;
    t.content_gen = 1;
    t
}

/// `glClear(GL_COLOR_BUFFER_BIT)` fused into `glDrawArrays(GL_TRIANGLES, 0,
/// 6)` on the window (a 64x64 RGBA target and its D24S8), the texture on
/// unit 0 (`linear`: job 2's filter).
fn tex2d_draw(linear: bool) -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    d.target_gpu = u64::from(G_RT);
    (d.target_w, d.target_h) = (SIZE, SIZE);
    d.target_format = GLES_SURF_RGBA8888;
    d.target_alpha_bits = 8;
    d.targets[0] = GlesDrawTarget { gpu: u64::from(G_RT), w: SIZE, h: SIZE, format: 0, out_shift: 0 };
    d.num_targets = 1;
    d.samples = 1;
    d.viewport = [0, 0, SIZE as i32, SIZE as i32];
    d.depth_range = [0.0, 1.0];
    (d.ds_persist, d.ds_id, d.depth_id, d.stencil_id) = (true, 1, 1, 1);
    (d.ds_w, d.ds_h) = (SIZE, SIZE);
    (d.color_write, d.color_mask) = (true, 0xf);
    (d.depth_func, d.depth_write) = (GLES_CMP_LESS, true);
    (d.stencil_func, d.stencil_back_func) = (GLES_CMP_ALWAYS, GLES_CMP_ALWAYS);
    (d.stencil_cmp_mask, d.stencil_back_cmp_mask) = (0xff, 0xff);
    (d.stencil_write_mask, d.stencil_back_write_mask) = (0xff, 0xff);
    d.line_width = 1.0;
    (d.do_clear, d.clear_rgba) = (true, [0.0, 0.0, 0.0, 1.0]);
    d.prim = GLES_PRIM_TRIANGLES;
    (d.index_count, d.instance_count) = (6, 1);
    /* the 24-byte puv vertex: pos4 +0 at location 0, uv2 +16 at location 1 */
    for (k, (comps, off)) in [(4u8, 0u32), (2, 16)].into_iter().enumerate() {
        let mut a = GlesAttrib::ZERO;
        (a.vi_reg, a.components, a.format) = (k as u8, comps, GLES_ATTRIBF_FLOAT32);
        (a.stride, a.offset, a.buffer_gpu) = (24, off, u64::from(G_VTX));
        a.buffer_bytes = 6 * 24u64;
        d.attribs[k] = a;
    }
    d.num_attribs = 2;
    d.textures[0] = texture(linear);
    d.num_textures = 1;
    d
}

fn frame(d: &GlesDraw) -> glue71::Frame71 {
    glue71::frame(d, glue71::DsPlace { gl_format: None, address: G_ZS, memory_format: 4, height_in_ub: 8 }).unwrap()
}

fn clear_quad() -> ClearQuad<'static> {
    let attr = glue71::attrs(&tex2d_draw(false), &[]).unwrap()[0];
    ClearQuad { program: program(false), attr, dummy_addr: G_VTX }
}

/// The draw without its texture, through glue71 and fill71 with `prog`.
fn fill_untextured(linear: bool, prog: &'static Program<'static>) -> Result<Vec<FilledJob>, fill71::Refusal> {
    let mut d = tex2d_draw(linear);
    d.num_textures = 0;
    let f = frame(&d);
    let attrs = glue71::attrs(&d, &[])?;
    let input = glue71::DrawInputs {
        program: prog,
        attrs: &attrs,
        vs_words: &[],
        fs_words: &[],
        dummy_addr: G_VTX,
        oq_addr: 0,
        indices: None,
    };
    let ops = glue71::ops(&d, &f, Some(&input))?;
    fill71::fill(&f.framebuffer(G_TILE), record71::BCM2712_D0, &History::NEW, &clear_quad(), &ops)
}

/// The record v3d_job71 packs for a filled draw (its programs as
/// record71's description; the uniform stream addresses are the table's,
/// here those of Mesa's `job`).
fn record_of(fd: &fill71::FilledDraw, job: &Job) -> record71::Record {
    let mut desc = record71::DrawDesc::default();
    desc.fs.prog = record71::ProgramDesc {
        code: fd.fs.code,
        uniforms: job.f("Fragment Shader Uniforms Address"),
        threads: fd.fs.threads,
        single_seg: fd.fs.single_seg,
        propagate_nans: false,
    };
    desc.fs.writes_z = fd.fs.writes_z;
    desc.fs.disable_ez = fd.fs.disable_ez;
    desc.fs.num_varyings = fd.fs.num_inputs;
    desc.fs.uses_center_w = fd.fs.uses_center_w;
    desc.fs.uses_pid = fd.fs.uses_pid;
    desc.fs.lock_scoreboard_on_first_thrsw = fd.fs.lock_scoreboard_on_first_thrsw;
    desc.fs.uses_implicit_point_line_varyings = fd.fs.uses_implicit_point_line_varyings;
    let v = |p: &ffi71::V3dVsProg, unif: u32| record71::VertexDesc {
        prog: record71::ProgramDesc {
            code: p.code,
            uniforms: unif,
            threads: p.threads,
            single_seg: p.single_seg,
            propagate_nans: false,
        },
        vpm_input_size: p.vpm_input_size,
        vpm_output_size: p.vpm_output_size,
        min_input_segments: 1,
        min_output_segments: 0,
        uses_vid: p.uses_vid,
        uses_iid: p.uses_iid,
        uses_base_instance: p.uses_biid,
        uses_base_vertex: false,
        uses_draw_index: false,
    };
    desc.vs = v(&fd.vs, job.f("Vertex Shader Uniforms Address"));
    desc.cs = v(&fd.cs, job.f("Coordinate Shader Uniforms Address"));
    desc.point_size = fd.vs_writes_psiz;
    record71::describe(record71::BCM2712_D0, &desc).unwrap()
}

/// The record fields where our UV pair's record is not Mesa's:
/// (field, ours, Mesa's), FIELDS order. They are the three that
/// docs/notes/v3d71-tri-ours-record.md (its table and the list at its end)
/// records for our triangle program, and they have the same causes here:
/// our FS sets `uses_center_w` (perspective-correct interpolation reads W;
/// Mesa's FS does not set it); Mesa's "insert primitive ID" is 1 for a
/// program that reads no primitive ID, and record71 takes `uses_pid` (the
/// 4.2 frame code's mapping); "disable implicit point/line varyings" is
/// `!uses_implicit_point_line_varyings` (1) where Mesa writes 0. None comes
/// from the texture.
const RECORD_DIFFS: &[(&str, u32, u32)] = &[
    ("Fragment shader uses real pixel centre W in addition to centroid W2", 1, 0),
    ("Insert Primitive ID as first varying to fragment shader", 0, 1),
    ("Disable implicit point/line varyings", 1, 0),
];

/* ---- the tests ------------------------------------------------------------------- */

#[test]
fn the_reference_holds_two_textured_jobs() {
    let j = jobs();
    assert_eq!(j.len(), 2, "NEAREST, then LINEAR");
    for (n, job) in j.iter().enumerate() {
        assert_eq!(job.prims, vec![(4, 6, 0)], "job {n}: one glDrawArrays(GL_TRIANGLES, 0, 6)");
        assert_eq!(job.attrs.len(), 2, "job {n}: pos4 and uv2");
        assert!(job.record.len() > 40, "job {n}: {} record fields", job.record.len());
        assert_eq!(job.fs_stream().len(), 2, "job {n}: the fragment stream is P0 and P1");
        assert_eq!(job.clear_low, Some(0xff00_0000), "job {n}: the clear to (0,0,0,1)");
    }
}

#[test]
fn our_uv_pair_links_with_one_sampler_on_unit_0() {
    let l = linked();
    let tex: Vec<(u32, u32)> = l.fs.iter().copied().filter(|(c, _)| is_texture(*c)).collect();
    assert_eq!(
        tex.iter().map(|t| t.0).collect::<Vec<_>>(),
        [texuni71::TMU_CONFIG_P0, texuni71::TMU_CONFIG_P1],
        "the fragment stream's texture words: {:?}",
        l.fs
    );
    assert!(tex.iter().all(|t| t.1 >> 24 == 0), "unit 0: {tex:?}");
    assert!(l.vs.iter().chain(&l.cs).all(|(c, _)| !is_texture(*c)), "no vertex texture");
    assert_eq!(&l.record_of_location[..2], &[0, 1], "a_pos is record 0, a_uv record 1");
}

#[test]
fn glue71_refuses_the_textured_draw_by_name() {
    for linear in [false, true] {
        let d = tex2d_draw(linear);
        let named = |e: fill71::Refusal| {
            assert_eq!((e.module, e.reason.as_str()), ("glue71", glue71::GLUE71_REFUSALS[4]));
            assert!(e.reason.contains("sampling textures"), "{}", e.reason);
        };
        named(glue71::refusal(&d).expect("a textured draw is refused"));
        let f = frame(&d);
        let attrs = glue71::attrs(&d, &[]).unwrap();
        let input = glue71::DrawInputs {
            program: program(true),
            attrs: &attrs,
            vs_words: &[],
            fs_words: &[],
            dummy_addr: G_VTX,
            oq_addr: 0,
            indices: None,
        };
        named(glue71::ops(&d, &f, Some(&input)).unwrap_err());
        /* a vertex-stage texture is the same refusal */
        let mut v = d;
        (v.num_textures, v.num_vs_textures) = (0, 1);
        named(glue71::refusal(&v).unwrap());
        /* what does not read the texture state maps as for any draw */
        assert_eq!((f.width, f.height, f.n), (SIZE, SIZE, 1));
        let c = glue71::clear(&d).unwrap();
        assert_eq!((c.color, c.draw_buffers, c.color_mask), (Some([0.0, 0.0, 0.0, 1.0]), 1, 0xf));
        let p = glue71::prim(&d, None).unwrap();
        assert_eq!((p.mode, p.first, p.count, p.instances, p.indices), (prim71::GL_TRIANGLES, 0, 6, 1, None));
        /* a clear alone on the textured state is not refused */
        let mut c = d;
        c.index_count = 0;
        assert!(glue71::refusal(&c).is_none());
    }
}

#[test]
fn fill71_refuses_our_tmu_config_words_by_uniform71s_name() {
    let e = fill_untextured(false, program(true)).unwrap_err();
    let want = uniform71::Refusal::TmuConfig { contents: texuni71::TMU_CONFIG_P0 }.to_string();
    assert_eq!((e.module, e.reason.as_str()), ("uniform71", want.as_str()));
}

#[test]
fn the_shader_record_is_mesas_but_the_named_differences() {
    for (n, job) in jobs().iter().enumerate() {
        let jobs = fill_untextured(n == 1, program(false)).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].draws.len(), 1, "the clear is fused: the GL draw alone, as Mesa's job");
        let r = record_of(&jobs[0].draws[0], job);
        let mut diffs = Vec::new();
        for (i, fd) in record71::FIELDS.iter().enumerate() {
            let ours = r.value(i);
            match job.record.get(fd.name) {
                Some(&mesa) if mesa != ours => diffs.push((fd.name, ours, mesa)),
                Some(_) => {}
                /* the draw-index record's four bits: the clif prints the
                 * plain record */
                None => assert_eq!(ours, 0, "job {n}: {} is not in the clif", fd.name),
            }
        }
        for name in job.record.keys() {
            assert!(record71::FIELDS.iter().any(|f| f.name == name), "job {n}: the clif's {name} is no field");
        }
        assert_eq!(diffs, RECORD_DIFFS, "job {n}: (field, ours, Mesa's)");
        assert_eq!(job.f("Number of varyings in Fragment Shader"), 2, "v_uv");
    }
}

#[test]
fn the_attribute_records_are_mesas() {
    let d = tex2d_draw(false);
    let attrs = glue71::attrs(&d, &[]).unwrap();
    let l = linked();
    let plan = attr71::plan(&attrs, &l.cs_reads, &l.vs_reads, false, G_VTX).unwrap();
    for (n, job) in jobs().iter().enumerate() {
        assert_eq!(plan.count, job.attrs.len(), "job {n}");
        for (k, mesa) in job.attrs.iter().enumerate() {
            let b = attr71::pack(&plan.records[k]);
            let bits = u128::from_le_bytes(b);
            for (name, start, size) in attr71::FIELDS {
                let mut ours = ((bits >> start) & ((1u128 << size) - 1)) as u32;
                if name == "Vec size" && ours == 0 {
                    ours = 4; /* the 2-bit field's 0 is 4, as the clif prints it */
                }
                assert_eq!(Some(&ours), mesa.get(name), "job {n} attribute {k}: {name}");
            }
        }
        let filled = fill_untextured(n == 1, program(false)).unwrap();
        let fa = &filled[0].draws[0].attrs;
        let got: Vec<(u32, u32, u32, u32)> =
            fa.iter().map(|a| (a.address, a.stride, a.components, a.divisor)).collect();
        let want: Vec<(u32, u32, u32, u32)> = job
            .attrs
            .iter()
            .map(|a| {
                (a["Address"], a["Stride"], if a["Vec size"] == 0 { 4 } else { a["Vec size"] }, a["Instance Divisor"])
            })
            .collect();
        assert_eq!(got, want, "job {n}: v3d_job71's vertex arrays");
    }
}

#[test]
fn the_call_and_the_clear_are_mesas() {
    for (n, job) in jobs().iter().enumerate() {
        let filled = fill_untextured(n == 1, program(false)).unwrap();
        let fd = &filled[0].draws[0];
        assert_eq!(vec![(fd.prim, fd.count, fd.first)], job.prims, "job {n}");
        assert_eq!(fd.prim, ffi71::V3D_PRIM_TRIANGLES);
        assert!(!fd.indexed && fd.instances == 1 && !fd.is_clear, "job {n}");
        let c = glue71::clear(&tex2d_draw(n == 1)).unwrap();
        let packed = c.color.unwrap().map(|x| (x * 255.0).round() as u32);
        let low = packed[0] | packed[1] << 8 | packed[2] << 16 | packed[3] << 24;
        assert_eq!(Some(low), job.clear_low, "job {n}: RGBA8 (0,0,0,1)");
        /* no TMU table goes to v3d_job71_draw */
        let ffi = fd.ffi();
        assert!(ffi.tmu.is_null() && ffi.num_tmu == 0 && ffi.vs_tmu.is_null() && ffi.num_vs_tmu == 0);
    }
}

#[test]
fn our_texture_uniforms_resolve_to_mesas_words() {
    let l = linked();
    let tex: Vec<(u32, u32)> = l.fs.iter().copied().filter(|(c, _)| is_texture(*c)).collect();
    for (n, job) in jobs().iter().enumerate() {
        let mesa = job.fs_stream();
        /* the record addresses Mesa used (the table's placement): P0's
         * 32-byte aligned texture state, P1's 8-byte aligned sampler */
        let unit = texuni71::Unit {
            unit: 0,
            tex_state: mesa[0] & !(texuni71::TEX_STATE_ALIGN - 1),
            sampler: mesa[1] & !(texuni71::SAMPLER_STATE_ALIGN - 1),
            width: SIZE,
            height: SIZE,
            depth: 1,
            array_size: 1,
        };
        let ours: Vec<u32> = tex.iter().map(|&(c, d)| texuni71::word(c, d, &[unit]).unwrap()).collect();
        assert_eq!(ours, mesa, "job {n}: P0 and P1 ({unit:x?})");
        /* the texture record at P0 is the README's (tests/v3d71_gl_tex.rs
         * composes it) */
        let rec: Vec<u32> = (0..6).map(|i| job.mem[&(unit.tex_state + 4 * i)]).collect();
        assert_eq!(rec, [0x0000_f000, 0x0000_0200, 0x0040_4001, 0x00b1_a840, 0x000f_0340, 0x0000_3c00], "job {n}");
    }
}

/// Job `n`'s TMU table entry for unit 0: the records Mesa placed (the
/// table's placement, read from the dump) and the base level's size.
fn tmu_of_job(n: usize) -> ffi71::V3dJob71Tmu {
    let mesa = jobs()[n].fs_stream();
    ffi71::V3dJob71Tmu {
        unit: 0,
        tex_state: mesa[0] & !(texuni71::TEX_STATE_ALIGN - 1),
        sampler: mesa[1] & !(texuni71::SAMPLER_STATE_ALIGN - 1),
        width: SIZE,
        height: SIZE,
        depth: 1,
        array_size: 1,
    }
}

/// gl_tex2d's draw `linear` through `glue71::ops_textured` and
/// `fill71::fill_textured` with the TMU table `fs`.
fn fill_textured<'t>(
    linear: bool,
    fs: &'t [ffi71::V3dJob71Tmu],
) -> Result<Vec<fill71::TexFilledJob<'t>>, fill71::Refusal> {
    let d = tex2d_draw(linear);
    let f = frame(&d);
    let attrs = glue71::attrs(&d, &[])?;
    let input = glue71::DrawInputs {
        program: program(true),
        attrs: &attrs,
        vs_words: &[],
        fs_words: &[],
        dummy_addr: G_VTX,
        oq_addr: 0,
        indices: None,
    };
    let t = glue71::ops_textured(&d, &f, Some(&input), &glue71::TexInputs { fs, vs: &[] })?;
    assert!(matches!(t.ops.as_slice(), [Op::Clear(_), Op::Draw(_)]));
    assert_eq!(t.tmu.len(), 1, "one TMU table pair for the one draw");
    fill71::fill_textured(&f.framebuffer(G_TILE), record71::BCM2712_D0, &History::NEW, &clear_quad(), &t.ops, &t.tmu)
}

/// P10: the textured draw fills through glue71, its fragment stream holds
/// Mesa's P0/P1 words, and `v3d_job71_draw` gets one TMU entry for unit 0.
#[test]
fn the_textured_draw_fills_through_glue71() {
    for (n, job) in jobs().iter().enumerate() {
        let table = [tmu_of_job(n)];
        let jobs = fill_textured(n == 1, &table).expect("glue71 and fill71 fill the textured draw");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job.draws.len(), 1, "the clear is fused: the GL draw alone, as Mesa's job");
        let fd = &jobs[0].job.draws[0];
        /* the stream v3d_job71 writes from the program's own list (the TMU
         * words with their contents) and the table is Mesa's */
        let vp = uniform71::Viewport { scale: fd.state.vp.scale, translate: fd.state.vp.translate };
        let stream: Vec<u32> = fd
            .fs_uniforms
            .iter()
            .map(|u| match fill71::tmu_word(u.contents, u.data, &table) {
                Some(w) => w.unwrap(),
                None => uniform71::word(uniform71::Word { contents: u.contents, data: u.data }, &vp, &[]).unwrap(),
            })
            .collect();
        assert!(fd.fs_uniforms.iter().any(|u| u.contents == texuni71::TMU_CONFIG_P0));
        assert!(fd.fs_uniforms.iter().any(|u| u.contents == texuni71::TMU_CONFIG_P1));
        assert_eq!(stream, job.fs_stream(), "job {n}: the fragment stream, P0 and P1");
        /* the record is the untextured draw's: the texture words change no
         * field of it */
        let r = record_of(fd, job);
        let plain = fill_untextured(n == 1, program(false)).unwrap();
        let p = record_of(&plain[0].draws[0], job);
        for i in 0..record71::FIELDS.len() {
            assert_eq!(r.value(i), p.value(i), "job {n}: {}", record71::FIELDS[i].name);
        }
        /* one TMU entry for v3d_job71_draw, none for the vertex stages */
        let ffi = jobs[0].ffi(0);
        assert_eq!(ffi.num_tmu, 1, "one sampler binding for v3d_job71_draw");
        assert!(ffi.vs_tmu.is_null() && ffi.num_vs_tmu == 0);
        // SAFETY: `num_tmu` entries of `table`, which outlives `ffi`.
        let t = unsafe { *ffi.tmu };
        let mesa = job.fs_stream();
        assert_eq!((t.unit, t.tex_state, t.sampler), (0, mesa[0] & !31, mesa[1] & !7));
        assert_eq!((t.width, t.height, t.depth, t.array_size), (SIZE, SIZE, 1, 1));
    }
}

#[test]
fn the_textured_draws_tables_are_checked_by_name() {
    let table = [tmu_of_job(0)];
    let own = |e: fill71::Refusal, module: &str, reasons: &[&str]| {
        assert_eq!(e.module, module, "{}", e.reason);
        assert!(reasons.contains(&e.reason.as_str()), "{}", e.reason);
    };
    /* no table, or one for another unit: glue71 names the bindings */
    own(fill_textured(false, &[]).unwrap_err(), "glue71", &glue71::GLUE71_TEX_REFUSALS);
    let mut other = table;
    other[0].unit = 1;
    own(fill_textured(false, &other).unwrap_err(), "glue71", &glue71::GLUE71_TEX_REFUSALS);
    /* a record address with a field's bits set, a size of 0: v3d_job71's
     * rules, named by fill71 */
    let mut bad = table;
    bad[0].sampler |= 4;
    own(fill_textured(false, &bad).unwrap_err(), "fill71", &fill71::FILL71_TEX_REFUSALS);
    let mut bad = table;
    bad[0].tex_state = 0;
    own(fill_textured(false, &bad).unwrap_err(), "fill71", &fill71::FILL71_TEX_REFUSALS);
    assert!(fill71::tmu_word(texuni71::TEXTURE_WIDTH, 0, &[ffi71::V3dJob71Tmu { width: 0, ..table[0] }])
        .unwrap()
        .is_err());
    /* fill71 wants one table pair per draw */
    let d = tex2d_draw(false);
    let f = frame(&d);
    let attrs = glue71::attrs(&d, &[]).unwrap();
    let input = glue71::DrawInputs {
        program: program(true),
        attrs: &attrs,
        vs_words: &[],
        fs_words: &[],
        dummy_addr: G_VTX,
        oq_addr: 0,
        indices: None,
    };
    let t = glue71::ops_textured(&d, &f, Some(&input), &glue71::TexInputs { fs: &table, vs: &[] }).unwrap();
    let fb = f.framebuffer(G_TILE);
    let e = fill71::fill_textured(&fb, record71::BCM2712_D0, &History::NEW, &clear_quad(), &t.ops, &[]).unwrap_err();
    own(e, "fill71", &fill71::FILL71_TEX_REFUSALS);
    let two = [t.tmu[0], t.tmu[0]];
    let e = fill71::fill_textured(&fb, record71::BCM2712_D0, &History::NEW, &clear_quad(), &t.ops, &two).unwrap_err();
    own(e, "fill71", &fill71::FILL71_TEX_REFUSALS);
}

#[test]
fn an_untextured_draw_fills_through_the_textured_path_as_through_fill() {
    let mut d = tex2d_draw(false);
    d.num_textures = 0;
    let f = frame(&d);
    let attrs = glue71::attrs(&d, &[]).unwrap();
    let input = glue71::DrawInputs {
        program: program(false),
        attrs: &attrs,
        vs_words: &[],
        fs_words: &[],
        dummy_addr: G_VTX,
        oq_addr: 0,
        indices: None,
    };
    let t = glue71::ops_textured(&d, &f, Some(&input), &glue71::TexInputs::default()).unwrap();
    let fb = f.framebuffer(G_TILE);
    let tex = fill71::fill_textured(&fb, record71::BCM2712_D0, &History::NEW, &clear_quad(), &t.ops, &t.tmu).unwrap();
    let plain = fill_untextured(false, program(false)).unwrap();
    assert_eq!(tex.len(), plain.len());
    for (a, b) in tex.iter().zip(&plain) {
        assert_eq!((a.job.target, a.job.clear, a.job.after), (b.target, b.clear, b.after));
        assert_eq!(a.job.draws.len(), b.draws.len());
        for (k, (x, y)) in a.job.draws.iter().zip(&b.draws).enumerate() {
            assert_eq!(
                (x.state, x.attrs.clone(), x.fs_uniforms.clone()),
                (y.state, y.attrs.clone(), y.fs_uniforms.clone())
            );
            assert_eq!((x.prim, x.count, x.first, x.instances), (y.prim, y.count, y.first, y.instances));
            let ffi = a.ffi(k);
            assert!(ffi.tmu.is_null() && ffi.num_tmu == 0 && ffi.vs_tmu.is_null() && ffi.num_vs_tmu == 0);
        }
    }
}
