// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 cube-map texture state (src/v3d/texture71.rs, texstate71.rs,
//! sampler71.rs, tfuplan71.rs) against Mesa's 7.1 reference
//! tools/v3d/pi5/gl_cube.clif (P10 of docs/notes/pre-study-rpi5.md).
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_cube`
//!
//! The reference is tools/v3d/pi5/src/gl_shapes.py `cube`: six 4x4 GL_RGBA
//! faces (glTexImage2D per face, level 0 only, blue 16 + 40 * face), wrap S/T
//! CLAMP_TO_EDGE (R left at GL's REPEAT), and 12 draws that alternate NEAREST
//! and LINEAR filters. The test reads, for every job, the Texture Shader State
//! and Sampler State records the fragment shader's TMU parameters point at and
//! the face texels of the buffer, and compares them field by field (bit
//! positions from the vendored v3d_packet.xml) with what `texture71::compose`
//! and `sampler71::sampler71` produce for the same GL calls.

#![allow(dead_code)]

#[path = "../src/v3d/format71.rs"]
mod format71;
#[path = "../src/v3d/sampler71.rs"]
mod sampler71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;
#[path = "../src/v3d/tfuplan71.rs"]
mod tfuplan71;

use std::collections::HashMap;

use sampler71::{Filter, Input, MipFilter, Wrap};
use texture71::{GlTex71, Target, Tex71};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
/// The BO address of the cube's buffer in the reference (`resource_0xd000`).
const BO: u32 = 0xd000;

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn hexs(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

/// `name_0xBASE+0xOFF` -> BASE + OFF
fn at(s: &str) -> u32 {
    let (nb, off) = s.split_once('+').unwrap();
    hexs(nb.rsplit_once("_0x").unwrap().1) + hexs(off)
}

struct Job {
    mem: HashMap<u32, u32>,
    /// the fragment uniform streams (P0, P1 of sampler 0)
    streams: Vec<Vec<u32>>,
}

/// The jobs of a Mesa clif: the binary words of each job's buffers and its
/// fragment uniform streams (as tools/v3d/pi5/src/texstate71.py reads them).
fn jobs(clif: &str) -> Vec<Job> {
    let mut out = Vec::new();
    let mut mem: HashMap<u32, u32> = HashMap::new();
    let mut recs: Vec<u32> = Vec::new();
    let mut unif: Vec<u32> = Vec::new();
    let mut addr: Option<u32> = None;
    let finish = |mem: HashMap<u32, u32>, recs: Vec<u32>, unif: Vec<u32>| {
        let mut streams = Vec::new();
        for fs in recs {
            let end = unif.iter().copied().filter(|&a| a > fs).min();
            let mut s = Vec::new();
            let mut a = fs;
            while let Some(&w) = mem.get(&a) {
                if end.is_some_and(|e| a >= e) {
                    break;
                }
                s.push(w);
                a += 4;
            }
            streams.push(s);
        }
        Job { mem, streams }
    };
    for raw in clif.lines() {
        let Some(l) = raw.strip_prefix("MESA: info: ") else { continue };
        if l == "@wait_render_all_cores" {
            out.push(finish(std::mem::take(&mut mem), std::mem::take(&mut recs), std::mem::take(&mut unif)));
            continue;
        }
        if let Some(r) = l.strip_prefix("@format binary /* [") {
            addr = Some(at(r.split(']').next().unwrap()));
            continue;
        }
        if let Some(r) = l.strip_prefix("@format blank ") {
            if let Some(inner) = r.split('[').nth(1) {
                let inner = inner.split(']').next().unwrap();
                let (lo, hi) = inner.split_once("..").unwrap();
                let (lo, hi) = (at(lo), at(&format!("{}+{hi}", inner.split('+').next().unwrap())));
                let mut a = lo & !3;
                while a <= hi {
                    mem.entry(a).or_insert(0);
                    a += 4;
                }
            }
            addr = None;
            continue;
        }
        if l.starts_with('@') {
            addr = None;
            continue;
        }
        let t = l.trim();
        if let Some(a) = addr.as_mut() {
            if t.starts_with("0x") && t.split_whitespace().all(|w| w.len() == 10 && w.starts_with("0x")) {
                for w in t.split_whitespace() {
                    mem.insert(*a, hexs(w));
                    *a += 4;
                }
                continue;
            }
        }
        if let Some(p) = t.find("Shader Uniforms Address: */ [") {
            let kind = t[..p].trim_start_matches("/*").trim();
            let a = hexs(t.rsplit("/* 0x").next().unwrap().split(' ').next().unwrap());
            unif.push(a);
            if kind.ends_with("Fragment") {
                recs.push(a);
            }
        }
    }
    out
}

/// One draw of the reference: its texture and sampler records and its memory.
struct Draw {
    tex: [u32; 6],
    smp: [u32; 6],
    mem: HashMap<u32, u32>,
}

fn draws() -> Vec<Draw> {
    let mut v = Vec::new();
    for job in jobs(&read("tools/v3d/pi5/gl_cube.clif")) {
        for s in &job.streams {
            assert_eq!(s.len(), 2, "the stream is P0, P1");
            let (ta, sa) = (s[0] & !0xf, s[1] & !7);
            v.push(Draw {
                tex: std::array::from_fn(|k| job.mem[&(ta + 4 * k as u32)]),
                smp: std::array::from_fn(|k| job.mem[&(sa + 4 * k as u32)]),
                mem: job.mem.clone(),
            });
        }
    }
    v
}

/// Bits `start..start + size` (size <= 32) of a record's words.
fn bits(w: &[u32], start: u32, size: u32) -> u32 {
    (0..size).fold(0, |v, k| {
        let b = start + k;
        v | ((w[(b / 32) as usize] >> (b % 32)) & 1) << k
    })
}

/// (start, size) of a field of the 7.1 "Texture Shader State" struct.
fn tex_field(xml: &str, name: &str) -> (u32, u32) {
    let open = "<struct name=\"Texture Shader State\"";
    let mut from = 0;
    while let Some(i) = xml[from..].find(open) {
        let s = from + i;
        let e = s + xml[s..].find("</struct>").unwrap();
        let block = &xml[s..e];
        if block[..block.find('>').unwrap()].contains("min_ver=\"71\"") {
            let key = format!("<field name=\"{name}\" ");
            let i = block.find(&key).unwrap_or_else(|| panic!("no field {name}"));
            let line = &block[i..i + block[i..].find('>').unwrap()];
            let num = |attr: &str| -> u32 {
                let k = format!("{attr}=\"");
                let j = line.find(&k).unwrap() + k.len();
                line[j..j + line[j..].find('"').unwrap()].parse().unwrap()
            };
            return (num("start"), num("size"));
        }
        from = e;
    }
    panic!("no 7.1 Texture Shader State");
}

/// The GL calls of `cube`: six 4x4 RGBA/UNSIGNED_BYTE faces at level 0.
fn gl_cube() -> GlTex71 {
    GlTex71 {
        target: Target::Cube,
        internal_format: GL_RGBA,
        ty: GL_UNSIGNED_BYTE,
        width: 4,
        height: 4,
        levels: 1,
        base_level: 0,
        max_level: texture71::GL_MAX_LEVEL_DEFAULT,
        swizzle: texture71::GL_SWIZZLE_IDENTITY,
        srgb_decode: true,
        bo_address: BO,
        ver: 71,
        iprev: 10,
    }
}

fn composed() -> Tex71 {
    texture71::compose(&gl_cube()).unwrap()
}

#[test]
fn the_reference_holds_twelve_draws_of_one_cube() {
    let d = draws();
    assert_eq!(d.len(), 12);
    // one texture record throughout, two sampler records (NEAREST, LINEAR)
    for x in &d {
        assert_eq!(x.tex, d[0].tex);
    }
    for (i, x) in d.iter().enumerate() {
        assert_eq!(x.smp, d[i & 1].smp, "draw {i}");
    }
    assert_ne!(d[0].smp, d[1].smp);
}

#[test]
fn texture_record_matches_every_field_of_the_reference() {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let t = composed();
    let rec = draws()[0].tex;
    let f = |n: &str| {
        let (s, w) = tex_field(&xml, n);
        bits(&rec, s, w)
    };
    // the cube's six faces are the record's depth, its face size the width
    // and height; Mesa does not mark it as a 3D texture
    assert_eq!((f("Image Width"), t.width), (4, 4));
    assert_eq!((f("Image Height"), t.height), (4, 4));
    assert_eq!((f("Image Depth"), t.depth), (6, 6));
    assert_eq!(f("Base Level"), t.base_level);
    assert_eq!(f("Max Level"), t.max_level);
    assert_eq!((t.base_level, t.max_level), (0, 0));
    assert_eq!(f("Texture base pointer") & !63, t.base);
    assert_eq!(t.base, BO);
    assert_eq!(f("Texture type"), t.tex_type);
    assert_eq!(f("Swizzle R"), u32::from(t.swizzle[0]));
    assert_eq!(f("Swizzle G"), u32::from(t.swizzle[1]));
    assert_eq!(f("Swizzle B"), u32::from(t.swizzle[2]));
    assert_eq!(f("Swizzle A"), u32::from(t.swizzle[3]));
    assert_eq!(t.swizzle, [2, 3, 4, 5]);
    assert_eq!(f("Transfer func"), t.transfer_func);
    assert_eq!(f("R/B swap"), u32::from(t.r_b_swap));
    assert_eq!(f("Reverse"), u32::from(t.reverse));
    assert_eq!(f("Extended"), u32::from(t.extended));
    assert_eq!(f("Level 0 is strictly UIF"), u32::from(t.level_0_is_strictly_uif));
    assert_eq!(f("Level 0 XOR enable"), u32::from(t.level_0_xor_enable));
    assert_eq!(f("Level 0 UB_PAD"), t.level_0_ub_pad);
    // the cube face stride: one 64-byte utile per 4x4 RGBA face
    assert_eq!(f("Array Stride (64-byte aligned)"), t.array_stride_64);
    assert_eq!(t.array_stride_64, 1);
    assert_eq!(t.cube_face_stride, 64);
    assert!(!t.return_32);
}

#[test]
fn texstate71_words_equal_the_reference_where_it_owns_the_bits() {
    let t = composed();
    let st = texstate71::texture_state(&texstate71::TexDesc {
        width: 4,
        height: 4,
        depth: 6,
        levels: 1,
        base_level: 0,
        cube: true,
        srgb: false,
        transfer_func: None,
        bytes_per_texel: 4,
        layer_stride: 64,
        base: BO,
        ver: 71,
        iprev: 10,
    })
    .unwrap();
    assert_eq!((st.width, st.height, st.depth, st.array_stride_64), (t.width, t.height, t.depth, t.array_stride_64));
    let w = texstate71::pack_words(&st);
    let r = draws()[0].tex;
    assert_eq!(w[0], r[0], "base address word");
    assert_eq!(w[1], r[1], "array stride and width");
    assert_eq!(w[2], r[2], "height and depth");
    // the reference's word 3 also holds the swizzle and type: only the
    // base/max level nibbles (bits 24..31) are texstate71's
    assert_eq!(w[3] >> 24, r[3] >> 24);
}

fn sampler_for(filter: Filter) -> Input {
    Input {
        min_filter: filter,
        mag_filter: filter,
        mip_filter: MipFilter::None,
        // glTexParameteri CLAMP_TO_EDGE on S and T; R stays GL's REPEAT
        wrap_s: Wrap::ClampToEdge,
        wrap_t: Wrap::ClampToEdge,
        // a cube sampler: seamless, and texfill71 sets all three wraps CLAMP
        wrap_r: Wrap::ClampToEdge,
        cube: true,
        ..Input::default_2d()
    }
}

fn words(b: &[u8; sampler71::SAMPLER_BYTES]) -> [u32; 6] {
    std::array::from_fn(|k| u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap()))
}

#[test]
fn sampler_record_matches_the_reference_for_nearest_and_linear() {
    let d = draws();
    for (i, x) in d.iter().enumerate() {
        let filter = if i & 1 == 1 { Filter::Linear } else { Filter::Nearest };
        let smp = sampler71::sampler71(&sampler_for(filter)).unwrap();
        assert_eq!((smp.wrap_s, smp.wrap_t, smp.wrap_r), (1, 1, 1));
        let packed = sampler71::pack(&smp).unwrap();
        assert_eq!(words(&packed), x.smp, "draw {i}");
    }
    // by name: mag/min NEAREST in bits 0-1, mip NEAREST bit 2, max LOD 1/256
    // (no mipmaps) at bit 20, wrap S/T/R CLAMP at bits 48/51/54
    assert_eq!(d[0].smp[0], 0x0010_0007);
    assert_eq!(d[1].smp[0], 0x0010_0004);
    assert_eq!(d[0].smp[1], 0x0049_0000);
}

#[test]
fn a_cube_sampler_is_seamless_only() {
    let i = Input { seamless: false, ..sampler_for(Filter::Linear) };
    assert_eq!(sampler71::sampler71(&i), Err(sampler71::Refusal::CubeNonSeamless));
}

#[test]
fn face_layout_matches_the_upload_in_the_reference() {
    let t = composed();
    let l = &t.layout;
    assert_eq!((l.layers, l.layer_stride, l.cpp), (6, 64, 4));
    assert_eq!(l.slices.len(), 1);
    assert_eq!(l.slices[0].offset, 0);
    assert_eq!(l.slices[0].tiling, texture71::Tiling::LinearTile);
    // the dump holds 0x180 bytes: six faces of 64 bytes
    assert_eq!(l.size, 6 * 64);
    let ramp = [0u32, 0x55, 0xaa, 0xff];
    for x in draws() {
        for face in 0..6u32 {
            let off = l.offset(0, face).unwrap();
            assert_eq!(off, 64 * face);
            let blue = 16 + 40 * face;
            // one 4x4 utile, row-major texels; word = A B G R
            for j in 0..4usize {
                for i in 0..4usize {
                    let w = 0xff00_0000 | blue << 16 | ramp[j] << 8 | ramp[i];
                    let a = BO + off + 4 * (j * 4 + i) as u32;
                    assert_eq!(x.mem[&a], w, "face {face} texel ({i},{j})");
                }
            }
        }
        assert_eq!(l.offset(0, 6), None);
    }
}

#[test]
fn tfu_plan_makes_one_job_per_cube_face() {
    // glGenerateMipmap on this cube would be one TFU job per face (the
    // reference generates no mipmaps; gl_mipgen.clif is the 2D case)
    let a = tfuplan71::Args {
        width: 4,
        height: 4,
        base_level: 0,
        last_level: 2,
        src_level: 0,
        ttype: composed().tex_type,
        cpp: 4,
        src_tiling: tfuplan71::Tiling::UifNoXor,
        dst_tiling: tfuplan71::Tiling::UifNoXor,
        layer: 0,
        layer_count: 6,
    };
    let jobs = tfuplan71::plan(&a).unwrap();
    assert_eq!(jobs.len(), 6);
    for (f, j) in jobs.iter().enumerate() {
        assert_eq!((j.layer, j.in_level, j.out_level, j.nummm), (f as u32, 0, 1, 2));
    }
}
