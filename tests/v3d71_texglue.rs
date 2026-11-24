// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! texglue71 (cases71, P10): the GL core's description of a sampled texture
//! (`GlesTextureBind`'s values) mapped to the inputs of the merged 7.1
//! modules reproduces Mesa's texture and sampler records of
//! tools/v3d/pi5/gl_tex2d, gl_texfmt and gl_cube (and gl_mipgen's draw), and
//! everything it does not cover is refused by name.
//!
//! * the six texture-state words and the 24 sampler bytes of every job,
//!   read from the dump's TMU Config pair (the reading rule of
//!   tools/v3d/pi5/src/texstate71.py), against `texture71::compose` and
//!   `sampler71::pack` of what `texglue71::glue` gives for the GL calls of
//!   tools/v3d/pi5/README.md "The GL calls", described as the GL core holds
//!   them (storage class, `GlesSwizzle` / `GlesWrap` codes);
//! * the two TMU Config words through `texuni71` with the glue's unit;
//! * the storage classes, the swizzle rules, the sampler rules and the
//!   refusals.
//!
//! The dump helpers (`parse_texdump`, `tex_fields`, `wbits`, `wput`,
//! `expected_tex_fields`, `revision`) are copied from
//! tests/v3d71_compose3.rs (the existing tests are not edited).
//!
//! stage71 (the 7.1 transfer function, R/B swap per revision and the
//! depth/packed storage in format71/texture71) is not on the tip: its open
//! cases are the named gaps in `STAGE71_GAPS`, each pinned by
//! `stage71_open_cases_are_named_gaps` at today's answer.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_texglue
//! ```
#![allow(dead_code)]
#[path = "../src/v3d/format71.rs"]
mod format71;
#[path = "../src/v3d/sampler71.rs"]
mod sampler71;
#[path = "../src/v3d/texglue71.rs"]
mod texglue71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;
#[path = "../src/v3d/texuni71.rs"]
#[allow(dead_code)]
mod texuni71;

use std::collections::BTreeMap;

use sampler71::{Border, Wrap};
use texglue71::*;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// ---- the dump (copied from tests/v3d71_compose3.rs) ---------------------------

struct TexRec {
    p0: u32,
    p1: u32,
    tex: [u32; 6],
    smp: [u32; 6],
}

struct TexDump {
    /// per job: the (P0, P1) pairs the fragment shaders' uniform streams hold
    jobs: Vec<Vec<TexRec>>,
    /// every `resource_0x...` buffer: start address and dumped extent
    resources: Vec<(u32, u32)>,
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

/// texstate71.py's reading rule: the words of each fragment shader's uniform
/// stream, each pair (P0, P1) whose P0 is a 32-byte aligned address and P1 an
/// 8-byte aligned one, both with six dumped words behind them.
fn parse_texdump(file: &str) -> TexDump {
    let text = read(&format!("tools/v3d/pi5/{file}.clif"));
    let mut jobs = Vec::new();
    let mut mem: BTreeMap<u32, u32> = BTreeMap::new();
    let mut fs: Vec<u32> = Vec::new();
    let mut unif: Vec<u32> = Vec::new();
    let mut extent: BTreeMap<String, u32> = BTreeMap::new();
    let mut addr: Option<u32> = None;
    let mut cur: Option<(String, u32)> = None;
    let finish = |mem: &BTreeMap<u32, u32>, fs: &[u32], unif: &[u32]| -> Vec<TexRec> {
        let mut recs = Vec::new();
        for &f in fs {
            let end = unif.iter().copied().filter(|&a| a > f).min();
            let mut stream = Vec::new();
            let mut a = f;
            while let Some(w) = mem.get(&a) {
                if end.is_some_and(|e| a >= e) {
                    break;
                }
                stream.push(*w);
                a += 4;
            }
            for p in stream.windows(2) {
                let (ta, sa) = (p[0] & !0xf, p[1] & !0x7);
                let have = |b: u32| (0..6).all(|k| mem.contains_key(&(b + 4 * k)));
                if ta % 32 != 0 || !have(ta) || !have(sa) {
                    continue;
                }
                recs.push(TexRec {
                    p0: p[0],
                    p1: p[1],
                    tex: std::array::from_fn(|k| mem[&(ta + 4 * k as u32)]),
                    smp: std::array::from_fn(|k| mem[&(sa + 4 * k as u32)]),
                });
            }
        }
        recs
    };
    for raw in text.lines() {
        let Some(line) = raw.strip_prefix("MESA: info: ") else { continue };
        if line == "@wait_render_all_cores" {
            jobs.push(finish(&mem, &fs, &unif));
            mem.clear();
            fs.clear();
            unif.clear();
            continue;
        }
        if let Some(r) = line.strip_prefix("@format binary /* [") {
            let (name, off) = r.split_once('+').unwrap();
            let off = hex(off.split(']').next().unwrap());
            let base = hex(name.rsplit('_').next().unwrap());
            addr = Some(base + off);
            cur = Some((name.to_string(), off));
            continue;
        }
        if let Some(r) = line.strip_prefix("@format blank ") {
            let r = &r[r.find('[').unwrap() + 1..r.find(']').unwrap()];
            let (name, range) = r.split_once('+').unwrap();
            let (lo, hi) = range.split_once("..").unwrap();
            let base = hex(name.rsplit('_').next().unwrap());
            let (lo, hi) = (hex(lo), hex(hi));
            for a in ((base + lo) & !3..=base + hi).step_by(4) {
                mem.entry(a).or_insert(0);
            }
            let e = extent.entry(name.to_string()).or_insert(0);
            *e = (*e).max(hi + 1);
            addr = None;
            cur = None;
            continue;
        }
        if line.starts_with('@') {
            addr = None;
            cur = None;
            continue;
        }
        let t = line.trim();
        if let Some(a) = addr.as_mut() {
            if !t.is_empty() && t.split(' ').all(|w| w.len() == 10 && w.starts_with("0x")) {
                let (name, off) = cur.as_mut().unwrap();
                for w in t.split(' ') {
                    mem.insert(*a, hex(w));
                    *a += 4;
                    *off += 4;
                }
                let e = extent.entry(name.clone()).or_insert(0);
                *e = (*e).max(*off);
                continue;
            }
        }
        for which in ["Fragment", "Vertex", "Coordinate"] {
            let key = format!("/* {which} Shader Uniforms Address: */ [");
            if let Some(i) = t.find(&key) {
                let r = &t[i + key.len()..];
                let v = &r[r.find("/* ").unwrap() + 3..];
                let a = hex(v.split(' ').next().unwrap());
                unif.push(a);
                if which == "Fragment" {
                    fs.push(a);
                }
            }
        }
    }
    let resources = extent
        .iter()
        .filter(|(n, _)| n.starts_with("resource_"))
        .map(|(n, e)| (hex(n.rsplit('_').next().unwrap()), *e))
        .collect();
    TexDump { jobs, resources }
}

/// (name, start, size) of every field of the 7.1 Texture Shader State, from
/// the vendored packet XML.
fn tex_fields() -> Vec<(String, u32, u32)> {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let open = "<struct name=\"Texture Shader State\" min_ver=\"71\">";
    let s = xml.find(open).expect("7.1 Texture Shader State");
    let block = &xml[s..s + xml[s..].find("</struct>").unwrap()];
    let attr = |tag: &str, a: &str| -> String {
        let k = format!(" {a}=\"");
        let j = tag.find(&k).unwrap_or_else(|| panic!("{a} in {tag}")) + k.len();
        tag[j..j + tag[j..].find('"').unwrap()].to_string()
    };
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = block[at..].find("<field ") {
        let t0 = at + i;
        let tag = &block[t0..t0 + block[t0..].find('>').unwrap()];
        out.push((attr(tag, "name"), attr(tag, "start").parse().unwrap(), attr(tag, "size").parse().unwrap()));
        at = t0 + tag.len();
    }
    out
}

fn wbits(words: &[u32; 6], start: u32, size: u32) -> u64 {
    let mut v = 0u64;
    for b in 0..size {
        let bit = start + b;
        v |= u64::from(words[(bit / 32) as usize] >> (bit % 32) & 1) << b;
    }
    v
}

fn wput(words: &mut [u32; 6], start: u32, size: u32, v: u64) {
    assert!(size == 64 || v < 1 << size, "value {v} in {size} bits");
    for b in 0..size {
        if v >> b & 1 == 1 {
            let bit = start + b;
            words[(bit / 32) as usize] |= 1 << (bit % 32);
        }
    }
}

/// Every field of the 7.1 record from the composed values; the C writer's
/// constants (chroma pointers, offsets 1) and the fields nothing sets are as
/// tests/v3d71_gl_tex.rs has them.
fn expected_tex_fields(t: &texture71::Tex71) -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    let mut s = |k: &str, v: u64| {
        m.insert(k.to_string(), v);
    };
    s("Texture base pointer", u64::from(t.base));
    s("Transfer func", u64::from(t.transfer_func));
    s("Reverse", u64::from(t.reverse));
    s("R/B swap", u64::from(t.r_b_swap));
    s("Array Stride (64-byte aligned)", u64::from(t.array_stride_64));
    s("Image Width", u64::from(t.width));
    s("Image Height", u64::from(t.height));
    s("Image Depth", u64::from(t.depth));
    s("Texture type", u64::from(t.tex_type));
    s("Extended", u64::from(t.extended));
    s("Swizzle R", u64::from(t.swizzle[0]));
    s("Swizzle G", u64::from(t.swizzle[1]));
    s("Swizzle B", u64::from(t.swizzle[2]));
    s("Swizzle A", u64::from(t.swizzle[3]));
    s("Max Level", u64::from(t.max_level));
    s("Base Level", u64::from(t.base_level));
    s("Level 0 UB_PAD", u64::from(t.level_0_ub_pad));
    s("Level 0 XOR enable", u64::from(t.level_0_xor_enable));
    s("Level 0 is strictly UIF", u64::from(t.level_0_is_strictly_uif));
    s("texture base pointer Cb", u64::from(t.base >> 6));
    s("texture_base pointer_Cr", u64::from(t.base >> 6));
    s("Chroma offset x", 1);
    s("Chroma offset y", 1);
    s("UIF XOR disable", 0);
    s("Flip texture X Axis", 0);
    s("Flip texture Y Axis", 0);
    s("Pad", 0);
    m
}

fn revision(shape: &str) -> (u32, u32) {
    let out = read(&format!("tools/v3d/pi5/{shape}.stdout"));
    let r = out.lines().find_map(|l| l.strip_prefix("gl_shapes: renderer V3D ")).expect("renderer line");
    let n: Vec<u32> = r.trim().split('.').map(|x| x.parse().unwrap()).collect();
    (n[0] * 10 + n[1], n[2])
}

// ---- the GL calls as the GL core describes them --------------------------------

/// One job of a texture reference: the GL core's bind and sampler for its
/// draw.
struct Case {
    shape: &'static str,
    job: usize,
    tex: GlTexBind71,
    sampler: GlSampler71,
}

fn cases() -> Vec<Case> {
    let n = |shape, job, tex, sampler| Case { shape, job, tex, sampler };
    let nearest = GlSampler71::filters(false, false, 0);
    let linear = GlSampler71::filters(true, true, 0);
    let mut v = vec![];
    // gl_tex2d: glTexImage2D(GL_RGBA8, 64x64), NEAREST then LINEAR, REPEAT
    let rgba = GlTexBind71::tex2d(GLES_TEXFMT_RGBA8, 64, 64, 1, 0);
    v.push(n("gl_tex2d", 1, rgba, nearest));
    v.push(n("gl_tex2d", 2, rgba, linear));
    // gl_texfmt: GL_SRGB8_ALPHA8, then GL_RGB565 (base format RGB: no alpha)
    v.push(n("gl_texfmt", 1, GlTexBind71::tex2d(GLES_TEXFMT_SRGB8_A8, 64, 64, 1, 0), nearest));
    let r565 = GlTexBind71 { no_alpha: true, ..GlTexBind71::tex2d(GLES_TEXFMT_RGB565, 64, 64, 1, 0) };
    v.push(n("gl_texfmt", 2, r565, nearest));
    // gl_mipgen's draw: glTexStorage2D(7, GL_RGBA8, 64x64), LINEAR_MIPMAP_LINEAR / LINEAR
    v.push(n("gl_mipgen", 1, GlTexBind71::tex2d(GLES_TEXFMT_RGBA8, 64, 64, 7, 0), GlSampler71::filters(true, true, 2)));
    // gl_cube: six 4x4 GL_RGBA faces, wrap S and T CLAMP_TO_EDGE, wrap R left
    // at REPEAT (the glue forces it), per face NEAREST then LINEAR
    for i in 0..12 {
        let mut s = if i & 1 == 1 { linear } else { nearest };
        s.wrap_s = GLES_WRAP_CLAMP_TO_EDGE;
        s.wrap_t = GLES_WRAP_CLAMP_TO_EDGE;
        v.push(n("gl_cube", i + 1, GlTexBind71::cube(GLES_TEXFMT_RGBA8, 4, 1, 0), s));
    }
    v
}

const SHAPES: [(&str, usize); 4] = [("gl_tex2d", 2), ("gl_texfmt", 2), ("gl_cube", 12), ("gl_mipgen", 1)];

#[test]
fn the_glued_records_match_mesa() {
    let fields = tex_fields();
    let mut checked = 0;
    for (shape, njobs) in SHAPES {
        let dump = parse_texdump(shape);
        let (ver, iprev) = revision(shape);
        assert_eq!((ver, iprev), (71, 10), "{shape}: the Pi 5's 7.1.10");
        assert_eq!(dump.jobs.len(), njobs, "{shape}: jobs");
        for c in cases().iter().filter(|c| c.shape == shape) {
            let what = format!("{shape} job {}", c.job);
            let recs = &dump.jobs[c.job - 1];
            assert_eq!(recs.len(), 1, "{what}: one texture record");
            let r = &recs[0];
            let got: BTreeMap<String, u64> = fields
                .iter()
                .map(|(n, s, z)| {
                    let v = wbits(&r.tex, *s, *z);
                    (n.clone(), if n == "Texture base pointer" { v & !63 } else { v })
                })
                .collect();
            let base = got["Texture base pointer"] as u32;
            let &(bo, extent) = dump.resources.iter().filter(|(a, _)| *a <= base).max_by_key(|(a, _)| *a).unwrap();
            let mut bind = c.tex;
            bind.bo_address = bo;
            let g = glue(&bind, &c.sampler, ver, iprev).unwrap_or_else(|e| panic!("{what}: {e}"));
            // the glue's composition is texture71's of its input
            let t = texture71::compose(&g.tex).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(t, g.composed, "{what}: composed");
            let want = expected_tex_fields(&t);
            assert_eq!(want.keys().collect::<Vec<_>>(), got.keys().collect::<Vec<_>>(), "{what}: field list");
            for (k, v) in &got {
                assert_eq!(want[k], *v, "{what}: {k}");
            }
            let mut words = [0u32; 6];
            for (n, s, z) in &fields {
                wput(&mut words, *s, *z, want[n]);
            }
            assert_eq!(words, r.tex, "{what}: the six texture words");
            assert!(t.layout.size <= extent, "{what}: layout past the dumped buffer");
            // the sampler's 24 bytes
            let s = sampler71::sampler71(&g.sampler).unwrap_or_else(|e| panic!("{what}: {}", e.message()));
            assert_eq!(s, g.sampler_values, "{what}: sampler values");
            let bytes = sampler71::pack(&s).unwrap_or_else(|| panic!("{what}: sampler pack"));
            let want_bytes: Vec<u8> = r.smp.iter().flat_map(|w| w.to_le_bytes()).collect();
            assert_eq!(&bytes[..], &want_bytes[..], "{what}: the sampler record");
            // TMU Config P0 and P1 through texuni71, the records where Mesa put them
            let unit = g.unit(r.p0 & !0xf, r.p1 & !0x7);
            let (d0, d1) = (g.unit << 24 | (r.p0 & 0xf), g.unit << 24 | (r.p1 & 0x7));
            assert_eq!(texuni71::word(texuni71::TMU_CONFIG_P0, d0, &[unit]), Ok(r.p0), "{what}: P0");
            assert_eq!(texuni71::word(texuni71::TMU_CONFIG_P1, d1, &[unit]), Ok(r.p1), "{what}: P1");
            g.check_return_size(d1).unwrap_or_else(|e| panic!("{what}: {e}"));
            checked += 1;
        }
    }
    assert_eq!(checked, 17);
}

/// The gl_cube detail: GL left wrap R at REPEAT (and S, T could be any
/// mode), Mesa's records have all three CLAMP; the glue forces them.
#[test]
fn a_seamless_cube_samples_with_every_wrap_clamp_to_edge() {
    let t = GlTexBind71::cube(GLES_TEXFMT_RGBA8, 4, 1, 0x10000);
    for w in [GLES_WRAP_REPEAT, GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_CLAMP_TO_EDGE] {
        let s = GlSampler71 { wrap_s: w, wrap_t: w, wrap_r: w, ..GlSampler71::filters(true, true, 0) };
        let g = glue(&t, &s, 71, 10).unwrap();
        assert!(g.sampler.cube);
        assert_eq!([g.sampler.wrap_s, g.sampler.wrap_t, g.sampler.wrap_r], [Wrap::ClampToEdge; 3]);
        assert_eq!(g.sampler_values.wrap_r, sampler71::WRAP_CLAMP);
    }
    // a 2D texture keeps GL's modes
    let s = GlSampler71 {
        wrap_s: GLES_WRAP_MIRRORED_REPEAT,
        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
        ..GlSampler71::filters(false, false, 0)
    };
    let g = glue(&GlTexBind71::tex2d(GLES_TEXFMT_RGBA8, 8, 8, 1, 0x10000), &s, 71, 10).unwrap();
    assert_eq!(
        [g.sampler.wrap_s, g.sampler.wrap_t, g.sampler.wrap_r],
        [Wrap::MirroredRepeat, Wrap::ClampToEdge, Wrap::Repeat]
    );
    assert!(!g.sampler.cube);
}

// ---- the storage classes -----------------------------------------------------------

/// Every `GLES_TEXFMT_*` class: its Mesa row (`format71`), or the named
/// refusal (the compressed ones, texture71's).
#[test]
fn every_storage_class_has_its_mesa_row_or_a_named_refusal() {
    let rows: [(u8, Result<&str, &str>); 40] = [
        (GLES_TEXFMT_RGBA8, Ok("R8G8B8A8_UNORM")),
        (GLES_TEXFMT_ETC2_RGB, Err("ETC2_RGB8")),
        (GLES_TEXFMT_ETC2_RGBA, Err("ETC2_RGBA8")),
        (GLES_TEXFMT_ETC2_RGB_PT, Err("ETC2_RGB8A1")),
        (GLES_TEXFMT_RGB565, Ok("B5G6R5_UNORM")),
        (GLES_TEXFMT_RGBA16F, Ok("R16G16B16A16_FLOAT")),
        (GLES_TEXFMT_R16F, Ok("R16_FLOAT")),
        (GLES_TEXFMT_R32F, Ok("R32_FLOAT")),
        (GLES_TEXFMT_L8, Ok("L8_UNORM")),
        (GLES_TEXFMT_A8, Ok("A8_UNORM")),
        (GLES_TEXFMT_LA8, Ok("L8A8_UNORM")),
        (GLES_TEXFMT_R11G11B10F, Ok("R11G11B10_FLOAT")),
        (GLES_TEXFMT_RGB9E5, Ok("R9G9B9E5_FLOAT")),
        (GLES_TEXFMT_RGB10A2, Ok("R10G10B10A2_UNORM")),
        (GLES_TEXFMT_RGB10A2UI, Ok("R10G10B10A2_UINT")),
        (GLES_TEXFMT_RGBA8_SNORM, Ok("R8G8B8A8_SNORM")),
        (GLES_TEXFMT_RGBA8UI, Ok("R8G8B8A8_UINT")),
        (GLES_TEXFMT_RGBA8I, Ok("R8G8B8A8_SINT")),
        (GLES_TEXFMT_RGBA16UI, Ok("R16G16B16A16_UINT")),
        (GLES_TEXFMT_RGBA16I, Ok("R16G16B16A16_SINT")),
        (GLES_TEXFMT_RGBA32UI, Ok("R32G32B32A32_UINT")),
        (GLES_TEXFMT_RGBA32I, Ok("R32G32B32A32_SINT")),
        (GLES_TEXFMT_RGBA32F, Ok("R32G32B32A32_FLOAT")),
        (GLES_TEXFMT_SRGB8_A8, Ok("R8G8B8A8_SRGB")),
        (GLES_TEXFMT_ETC2_SRGB_RGB, Err("ETC2_SRGB8")),
        (GLES_TEXFMT_ETC2_SRGB_RGBA, Err("ETC2_SRGBA8")),
        (GLES_TEXFMT_R32UI, Ok("R32_UINT")),
        (GLES_TEXFMT_R32I, Ok("R32_SINT")),
        (GLES_TEXFMT_RG32UI, Ok("R32G32_UINT")),
        (GLES_TEXFMT_RG32I, Ok("R32G32_SINT")),
        (GLES_TEXFMT_R8_SNORM, Ok("R8_SNORM")),
        (GLES_TEXFMT_RG8_SNORM, Ok("R8G8_SNORM")),
        (GLES_TEXFMT_R8UI, Ok("R8_UINT")),
        (GLES_TEXFMT_R8I, Ok("R8_SINT")),
        (GLES_TEXFMT_RG8UI, Ok("R8G8_UINT")),
        (GLES_TEXFMT_RG8I, Ok("R8G8_SINT")),
        (GLES_TEXFMT_R16UI, Ok("R16_UINT")),
        (GLES_TEXFMT_R16I, Ok("R16_SINT")),
        (GLES_TEXFMT_RG16UI, Ok("R16G16_UINT")),
        (GLES_TEXFMT_RG16I, Ok("R16G16_SINT")),
    ];
    for (i, (f, want)) in rows.iter().enumerate() {
        assert_eq!(*f as usize, i, "the classes in order");
        let integer = texfmt_is_integer(*f);
        let nearest = GlSampler71::filters(false, false, 0);
        let r = glue(&GlTexBind71::tex2d(*f, 16, 16, 1, 0x10000), &nearest, 71, 10);
        match want {
            Ok(pipe) => {
                let g = r.unwrap_or_else(|e| panic!("class {f}: {e}"));
                assert_eq!(g.composed.row.pipe, *pipe, "class {f}");
                assert_eq!(integer, pipe.contains("_UINT") || pipe.contains("_SINT"), "class {f}: integer");
                // the return size is the row's; a program of the other size is refused
                let p1 = u32::from(g.composed.return_32);
                assert_eq!(g.check_return_size(p1), Ok(()));
                assert!(matches!(g.check_return_size(p1 ^ 1), Err(Refusal::ReturnSize { .. })));
            }
            Err(pipe) => {
                assert_eq!(r.unwrap_err(), Refusal::Texture(texture71::Refusal::Compressed { pipe }), "class {f}");
            }
        }
    }
    assert_eq!(
        glue(&GlTexBind71::tex2d(40, 16, 16, 1, 0), &GlSampler71::filters(false, false, 0), 71, 10).unwrap_err(),
        Refusal::UnknownTexfmt { texfmt: 40 }
    );
}

// ---- the swizzle --------------------------------------------------------------

use format71::{SWIZZLE_ALPHA as VA, SWIZZLE_BLUE as VB, SWIZZLE_GREEN as VG, SWIZZLE_ONE as V1};
use format71::{SWIZZLE_RED as VR, SWIZZLE_ZERO as V0};

fn swz(t: GlTexBind71) -> [u8; 4] {
    glue(&t, &GlSampler71::filters(false, false, 0), 71, 10).unwrap_or_else(|e| panic!("{e}")).composed.swizzle
}

/// The record's swizzle: the class's rule, then GL's, then the row's (4.2's
/// `tmu_format` / `tmu_swizzle`, Mesa's `util_format_compose_swizzles`).
#[test]
fn the_swizzle_composes_the_class_rule_gl_and_the_row() {
    let rgba = GlTexBind71::tex2d(GLES_TEXFMT_RGBA8, 4, 4, 1, 0x10000);
    assert_eq!(swz(rgba), [VR, VG, VB, VA]);
    // no_alpha: GL_RGB8's row, alpha 1
    assert_eq!(swz(GlTexBind71 { no_alpha: true, ..rgba }), [VR, VG, VB, V1]);
    // bgra: the XRGB8888 EGLImage reads B8G8R8X8 (ZYX1)
    let bgra = GlTexBind71 { bgra: true, ..rgba };
    assert_eq!(swz(bgra), [VB, VG, VR, V1]);
    let g = glue(&bgra, &GlSampler71::filters(false, false, 0), 71, 10).unwrap();
    assert_eq!(g.composed.tex_type, format71::bgrx8_egl_image().tex_type);
    // GL's swizzle picks from the class's components
    let s = |sw: [u8; 4], t: GlTexBind71| swz(GlTexBind71 { swizzle: sw, ..t });
    use texglue71::{
        GLES_SWIZZLE_ALPHA as A, GLES_SWIZZLE_BLUE as B, GLES_SWIZZLE_GREEN as G, GLES_SWIZZLE_ONE as ONE,
        GLES_SWIZZLE_RED as R, GLES_SWIZZLE_SAME as SAME, GLES_SWIZZLE_ZERO as ZERO,
    };
    assert_eq!(s([B, G, R, A], rgba), [VB, VG, VR, VA]);
    assert_eq!(s([ZERO, ONE, R, ZERO], rgba), [V0, V1, VR, V0]);
    assert_eq!(s([SAME, ONE, SAME, SAME], rgba), [VR, V1, VB, VA]);
    assert_eq!(s([B, G, R, A], bgra), [VR, VG, VB, V1]);
    assert_eq!(s([A, ZERO, ONE, R], bgra), [V1, V0, V1, VB]);
    // luminance and alpha classes: their rows' swizzles, GL in base terms
    let l8 = GlTexBind71::tex2d(GLES_TEXFMT_L8, 4, 4, 1, 0x10000);
    assert_eq!(swz(l8), [VR, VR, VR, V1]);
    assert_eq!(s([G, SAME, SAME, SAME], l8), [VR, VR, VR, V1]);
    assert_eq!(swz(GlTexBind71::tex2d(GLES_TEXFMT_A8, 4, 4, 1, 0x10000)), [V0, V0, V0, VR]);
    assert_eq!(swz(GlTexBind71::tex2d(GLES_TEXFMT_LA8, 4, 4, 1, 0x10000)), [VR, VR, VR, VG]);
    // the depth class: (d, 0, 0, 1) in ES 3.0, (d, d, d, 1) as ES 2.0 luminance
    let d = GlTexBind71::tex2d(GLES_TEXFMT_R32F, 4, 4, 1, 0x10000);
    assert_eq!(swz(d), [VR, V0, V0, V1]);
    assert_eq!(swz(GlTexBind71 { depth_luminance: true, ..d }), [VR, VR, VR, V1]);
    // a code outside GlesSwizzle
    assert_eq!(gl_swizzle(&GlTexBind71 { swizzle: [7, 0, 0, 0], ..rgba }), Err(Refusal::SwizzleCode { value: 7 }));
}

// ---- the sampler ------------------------------------------------------------------

#[test]
fn the_sampler_rules() {
    let t = GlTexBind71::tex2d(GLES_TEXFMT_RGBA8, 16, 16, 5, 0x10000);
    // filters and mip filter
    for (mag, min, mip) in [(false, false, 0u8), (true, false, 1), (false, true, 2), (true, true, 2)] {
        let g = glue(&t, &GlSampler71::filters(mag, min, mip), 71, 10).unwrap();
        let v = g.sampler_values;
        assert_eq!((v.mag_nearest, v.min_nearest, v.mip_nearest, v.mip_none), (!mag, !min, mip != 2, mip == 0));
    }
    assert_eq!(
        glue(&t, &GlSampler71::filters(false, false, 3), 71, 10).unwrap_err(),
        Refusal::MipFilterCode { value: 3 }
    );
    // the wrap codes, and the two taken ahead of GlesWrap
    for (code, wrap) in [
        (GLES_WRAP_REPEAT, sampler71::WRAP_REPEAT),
        (GLES_WRAP_MIRRORED_REPEAT, sampler71::WRAP_MIRROR),
        (GLES_WRAP_CLAMP_TO_EDGE, sampler71::WRAP_CLAMP),
        (WRAP_CLAMP_TO_BORDER, sampler71::WRAP_BORDER),
        (WRAP_MIRROR_CLAMP_TO_EDGE, sampler71::WRAP_MIRROR_ONCE),
    ] {
        let s = GlSampler71 { wrap_s: code, ..GlSampler71::filters(false, false, 0) };
        assert_eq!(glue(&t, &s, 71, 10).unwrap().sampler_values.wrap_s, wrap);
    }
    let s = GlSampler71 { wrap_t: 5, ..GlSampler71::filters(false, false, 0) };
    assert_eq!(glue(&t, &s, 71, 10).unwrap_err(), Refusal::WrapCode { value: 5 });
    // the compare: the depth class only (ES 3.0.6 3.8.15)
    let depth = GlTexBind71::tex2d(GLES_TEXFMT_R32F, 16, 16, 1, 0x10000);
    for op in 0u8..16 {
        let s = GlSampler71 { compare_op: op, ..GlSampler71::filters(false, false, 0) };
        let g = glue(&depth, &s, 71, 10).unwrap();
        assert!(g.sampler.compare_enabled);
        assert_eq!(g.sampler_values.compare_func, u32::from(op & 7));
        let g = glue(&t, &s, 71, 10).unwrap();
        assert!(!g.sampler.compare_enabled);
        assert_eq!(g.sampler_values.compare_func, 0);
    }
    let off = glue(&depth, &GlSampler71::filters(false, false, 0), 71, 10).unwrap();
    assert!(!off.sampler.compare_enabled);
    let s = GlSampler71 { compare_op: 17, ..GlSampler71::filters(false, false, 0) };
    assert_eq!(glue(&depth, &s, 71, 10).unwrap_err(), Refusal::CompareCode { value: 17 });
    // the border colour: its class must be the format's under CLAMP_TO_BORDER
    let border =
        |b: Border| GlSampler71 { wrap_s: WRAP_CLAMP_TO_BORDER, border: b, ..GlSampler71::filters(false, false, 0) };
    let g = glue(&t, &border(Border::Float([0.25, 0.5, 0.75, 1.0])), 71, 10).unwrap();
    assert_eq!(g.sampler_values.border_color_mode, sampler71::BORDER_FOLLOWS);
    assert_eq!(g.sampler_values.border_color, [0.25f32, 0.5, 0.75, 1.0].map(f32::to_bits));
    assert_eq!(
        glue(&t, &border(Border::Int([1, 2, 3, 4])), 71, 10).unwrap_err(),
        Refusal::BorderClass { integer_format: false }
    );
    let ui = GlTexBind71::tex2d(GLES_TEXFMT_RGBA8UI, 16, 16, 1, 0x10000);
    assert_eq!(
        glue(&ui, &border(Border::Float([0.0; 4])), 71, 10).unwrap_err(),
        Refusal::BorderClass { integer_format: true }
    );
    let g = glue(&ui, &border(Border::Int([1, 2, 3, 4])), 71, 10).unwrap();
    assert_eq!(g.sampler_values.border_color, [1, 2, 3, 4]);
    // without CLAMP_TO_BORDER the border is not read, whatever its class
    let s = GlSampler71 { border: Border::Int([1, 2, 3, 4]), ..GlSampler71::filters(false, false, 0) };
    assert_eq!(glue(&t, &s, 71, 10).unwrap().sampler_values.border_color_mode, sampler71::BORDER_0000);
    // LOD range and bias pass on; sampler71's refusals come back wrapped
    let s = GlSampler71 { min_lod: 1.0, max_lod: 3.0, lod_bias: 0.5, ..GlSampler71::filters(true, true, 2) };
    let v = glue(&t, &s, 71, 10).unwrap().sampler_values;
    assert_eq!((v.min_lod, v.max_lod, v.lod_bias), (1.0, 3.0, 0.5));
    let s = GlSampler71 { lod_bias: 200.0, ..GlSampler71::filters(true, true, 2) };
    assert_eq!(glue(&t, &s, 71, 10).unwrap_err(), Refusal::Sampler(sampler71::Refusal::BiasOutOfRange));
}

// ---- levels and the size words ------------------------------------------------

#[test]
fn levels_and_the_size_words() {
    let mut t = GlTexBind71::tex2d(GLES_TEXFMT_RGBA8, 64, 32, 7, 0x10000);
    t.base_level = 2;
    t.max_level = 4;
    let g = glue(&t, &GlSampler71::filters(true, true, 2), 71, 10).unwrap();
    assert_eq!((g.composed.base_level, g.composed.max_level), (2, 4));
    assert_eq!(g.size, (16, 8, 1, 1));
    let u = g.unit(0x2000, 0x3008);
    assert_eq!(texuni71::word(texuni71::TEXTURE_WIDTH, 0, &[u]), Ok(16));
    assert_eq!(texuni71::word(texuni71::TEXTURE_HEIGHT, 0, &[u]), Ok(8));
    assert_eq!(texuni71::word(texuni71::TEXTURE_DEPTH, 0, &[u]), Ok(1));
    assert_eq!(texuni71::word(texuni71::TEXTURE_ARRAY_SIZE, 0, &[u]), Ok(1));
    // the unit number is the bind's
    let mut t3 = t;
    t3.unit = 3;
    let g = glue(&t3, &GlSampler71::filters(true, true, 2), 71, 10).unwrap();
    assert_eq!(texuni71::word(texuni71::TMU_CONFIG_P0, 3 << 24 | 0xf, &[g.unit(0x2000, 0x3008)]), Ok(0x200f));
    assert_eq!(
        texuni71::word(texuni71::TMU_CONFIG_P0, 0xf, &[g.unit(0x2000, 0x3008)]),
        Err(texuni71::Refusal::NoUnit { unit: 0 })
    );
    // a cube's size words are one face's
    let g = glue(&GlTexBind71::cube(GLES_TEXFMT_RGBA8, 8, 1, 0x10000), &GlSampler71::filters(false, false, 0), 71, 10)
        .unwrap();
    assert_eq!(g.size, (8, 8, 1, 1));
    assert_eq!(g.composed.depth, 6);
}

// ---- the refusals ---------------------------------------------------------------

#[test]
fn what_is_not_covered_is_refused_by_name() {
    let s = GlSampler71::filters(false, false, 0);
    let t = GlTexBind71::tex2d(GLES_TEXFMT_RGBA8, 16, 16, 1, 0x10000);
    let with = |f: &dyn Fn(&mut GlTexBind71)| {
        let mut x = t;
        f(&mut x);
        glue(&x, &s, 71, 10).unwrap_err()
    };
    let cases: Vec<(Refusal, &str)> = vec![
        (with(&|x| x.tex_type = GLES_TEXTYPE_2D_ARRAY), "2D array"),
        (with(&|x| x.tex_type = GLES_TEXTYPE_3D), "3D texture"),
        (with(&|x| x.tex_type = GLES_TEXTYPE_CUBE_TWIDDLED_PROBE), "twiddled probe"),
        (with(&|x| x.tex_type = GLES_TEXTYPE_2D_ARRAY_TWIDDLED), "twiddled probe"),
        (with(&|x| x.tex_type = GLES_TEXTYPE_3D_TWIDDLED), "twiddled probe"),
        (with(&|x| x.tex_type = 9), "unknown texture type"),
        (with(&|x| x.layers = 4), "2D texture binding with 4 layers"),
        (
            with(&|x| {
                x.tex_type = GLES_TEXTYPE_CUBE;
                x.layers = 1
            }),
            "not its six faces",
        ),
        (with(&|x| x.incomplete = true), "incomplete"),
        (with(&|x| x.texfmt = 99), "unknown texel format class"),
        (
            with(&|x| {
                x.texfmt = GLES_TEXFMT_RGB565;
                x.bgra = true
            }),
            "not RGBA8",
        ),
        (with(&|x| x.depth_luminance = true), "not the depth class"),
        (with(&|x| x.swizzle = [0, 0, 0, 8]), "not a GlesSwizzle"),
    ];
    let want = [
        Refusal::Array2d,
        Refusal::Texture3d,
        Refusal::TwiddledProbe { tex_type: 4 },
        Refusal::TwiddledProbe { tex_type: 5 },
        Refusal::TwiddledProbe { tex_type: 6 },
        Refusal::UnknownType { tex_type: 9 },
        Refusal::Layers2d { layers: 4 },
        Refusal::CubeLayers { layers: 1 },
        Refusal::Incomplete,
        Refusal::UnknownTexfmt { texfmt: 99 },
        Refusal::BgraNotRgba8 { texfmt: GLES_TEXFMT_RGB565 },
        Refusal::DepthLuminanceNotDepth { texfmt: GLES_TEXFMT_RGBA8 },
        Refusal::SwizzleCode { value: 8 },
    ];
    assert_eq!(cases.len(), want.len());
    for ((got, text), want) in cases.iter().zip(want) {
        assert_eq!(*got, want);
        let m = got.to_string();
        assert!(m.starts_with("texture glue 7.1: "), "{m}");
        assert!(m.contains(text), "{m} names {text:?}");
    }
    // the modules' own refusals come back wrapped, with their messages
    let r = glue(&GlTexBind71::cube(GLES_TEXFMT_RGBA8, 4, 1, 0x10000), &GlSampler71 { seamless: false, ..s }, 71, 10)
        .unwrap_err();
    assert_eq!(r, Refusal::Sampler(sampler71::Refusal::CubeNonSeamless));
    assert_eq!(r.to_string(), sampler71::Refusal::CubeNonSeamless.message());
    let r = glue(&t, &s, 71, 2).unwrap_err();
    assert_eq!(r, Refusal::Texture(texture71::Refusal::State(texstate71::Refusal::RevisionBelow715 { iprev: 2 })));
    let r = glue(&t, &s, 42, 0).unwrap_err();
    assert!(matches!(r, Refusal::Texture(texture71::Refusal::State(texstate71::Refusal::NotV3d71 { .. }))));
    let mut ns = GlTexBind71::cube(GLES_TEXFMT_RGBA8, 4, 1, 0x10000);
    ns.height = 8;
    assert_eq!(
        glue(&ns, &s, 71, 10).unwrap_err(),
        Refusal::Texture(texture71::Refusal::State(texstate71::Refusal::CubeNotSquare))
    );
    assert_eq!(
        glue(&GlTexBind71::tex2d(GLES_TEXFMT_RGBA8, 16, 16, 9, 0x10000), &s, 71, 10).unwrap_err(),
        Refusal::Texture(texture71::Refusal::LevelsBeyondChain { levels: 9, chain: 5 })
    );
    let r = Refusal::ReturnSize { format_32: true, program_32: false };
    assert_eq!(r.to_string(), "texture glue 7.1: the program reads 16 bits where the format returns 32");
}

// ---- stage71's open cases ---------------------------------------------------------

/// stage71 (format71/texture71: the transfer function, R/B swap per
/// revision, depth and packed storage as Mesa) is not on the tip. Its open
/// cases as this module meets them, each pinned below at today's answer, so
/// that stage71's merge changes them on purpose:
const STAGE71_GAPS: [&str; 4] = [
    // the GL path never sets R/B swap or reverse: the row's swizzle carries
    // a swap (bgra: ZYX1); stage71 decides the bit per revision
    "r_b_swap per revision",
    // the transfer function is sRGB for an sRGB class with decode on, at
    // 7.1.5 and later only (texstate71 refuses older revisions)
    "transfer function per revision",
    // the GL core's depth class is R32F: the R32_FLOAT row (X001), not
    // Mesa's depth rows (Z32_FLOAT / X8Z24_UNORM / Z16_UNORM, XXXX)
    "depth storage as Mesa's depth rows",
    // a D24S8 sampled from its own buffer (`GlesTextureBind::depth_id`)
    // and the packed S8_UINT_Z24_UNORM row: no storage class maps to them
    "packed depth/stencil storage",
];

#[test]
fn stage71_open_cases_are_named_gaps() {
    let s = GlSampler71::filters(false, false, 0);
    // r_b_swap: false for every class that composes, and for bgra
    for f in 0u8..40 {
        for bgra in [false, true] {
            let t = GlTexBind71 { bgra, ..GlTexBind71::tex2d(f, 16, 16, 1, 0x10000) };
            if let Ok(g) = glue(&t, &s, 71, 10) {
                assert!(!g.composed.r_b_swap && !g.composed.reverse, "{}: class {f}", STAGE71_GAPS[0]);
            }
        }
    }
    // transfer function: sRGB with decode, none with SKIP_DECODE
    let srgb = GlTexBind71::tex2d(GLES_TEXFMT_SRGB8_A8, 16, 16, 1, 0x10000);
    assert_eq!(glue(&srgb, &s, 71, 10).unwrap().composed.transfer_func, 1, "{}", STAGE71_GAPS[1]);
    let skip = GlTexBind71 { srgb_decode: false, ..srgb };
    assert_eq!(glue(&skip, &s, 71, 10).unwrap().composed.transfer_func, 0, "{}", STAGE71_GAPS[1]);
    // the depth class reads the R32_FLOAT row
    let d = glue(&GlTexBind71::tex2d(GLES_TEXFMT_R32F, 16, 16, 1, 0x10000), &s, 71, 10).unwrap();
    assert_eq!(d.composed.row.pipe, "R32_FLOAT", "{}", STAGE71_GAPS[2]);
    assert!(d.composed.return_32);
    // no class is a packed depth/stencil row
    for f in 0u8..40 {
        if let Ok(g) = glue(&GlTexBind71::tex2d(f, 16, 16, 1, 0x10000), &s, 71, 10) {
            assert!(!g.composed.row.pipe.contains("Z24"), "{}: class {f}", STAGE71_GAPS[3]);
        }
    }
}
