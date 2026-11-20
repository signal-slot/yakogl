// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 GL texture to texture-record composition
//! (src/v3d/texture71.rs), host test against Mesa's 7.1 references.
//!
//! Run: cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_gl_tex
//!
//! For every job of tools/v3d/pi5/gl_tex2d, gl_texfmt and gl_cube (and the
//! draw of gl_mipgen, a 7-level tree), this reads the texture record Mesa's
//! fragment shader samples straight out of the `.clif` dump (the reading
//! rule of tools/v3d/pi5/src/texstate71.py: the fragment shader's uniform
//! stream, the TMU Config Parameter 0/1 pair, the six words at P0's address),
//! decodes it with the 7.1 `Texture Shader State` field list of the vendored
//! v3d_packet.xml, and compares every field with what texture71 composes
//! from the case's GL description (the GL calls of tools/v3d/pi5/README.md,
//! "The GL calls"). The revision comes from the `.stdout` (`renderer V3D
//! 7.1.10.2`), the buffer address from the dump's `@createbuf` names. The
//! words are also compared with the README's record table, and the
//! `v3d_tex71_state` field names with driver/src/v3d/v3d_tex71.h. Nothing
//! expected is a hand-copied word. The last tests check the named refusals.

#[path = "../src/v3d/format71.rs"]
#[allow(dead_code)]
mod format71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;

use std::collections::BTreeMap;

use texture71::*;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// ---- the 7.1 record layout, from v3d_packet.xml --------------------------

/// (name, start, size) of every field of the 7.1 Texture Shader State.
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
    assert!(out.len() > 20, "{} fields", out.len());
    out
}

fn bits_of(words: &[u32; 6]) -> (u128, u64) {
    let lo = words[..4].iter().enumerate().map(|(i, w)| u128::from(*w) << (32 * i)).sum();
    let hi = u64::from(words[4]) | u64::from(words[5]) << 32;
    (lo, hi)
}

fn get(words: &[u32; 6], start: u32, size: u32) -> u64 {
    let (lo, hi) = bits_of(words);
    let mut v = 0u64;
    for b in 0..size {
        let bit = start + b;
        let one = if bit < 128 { (lo >> bit) & 1 } else { u128::from((hi >> (bit - 128)) & 1) };
        v |= (one as u64) << b;
    }
    v
}

fn put(words: &mut [u32; 6], start: u32, size: u32, v: u64) {
    assert!(size == 64 || v < 1 << size, "value {v} in {size} bits");
    for b in 0..size {
        if v >> b & 1 == 1 {
            let bit = start + b;
            words[(bit / 32) as usize] |= 1 << (bit % 32);
        }
    }
}

/// The base pointer is an `address` field over bits 0..31 whose low six
/// bits are the flag fields under it (transfer func, reverse, flips): the
/// address is 64-byte aligned, so they are masked out of it.
fn decode(words: &[u32; 6], fields: &[(String, u32, u32)]) -> BTreeMap<String, u64> {
    fields
        .iter()
        .map(|(n, s, z)| {
            let v = get(words, *s, *z);
            (n.clone(), if n == "Texture base pointer" { v & !63 } else { v })
        })
        .collect()
}

fn encode(values: &BTreeMap<String, u64>, fields: &[(String, u32, u32)]) -> [u32; 6] {
    let mut w = [0u32; 6];
    for (n, s, z) in fields {
        put(&mut w, *s, *z, values[n]);
    }
    w
}

// ---- the dump: texstate71.py's reading rule --------------------------------

struct Dump {
    /// per job: the texture records (six words) its fragment shaders sample
    jobs: Vec<Vec<[u32; 6]>>,
    /// every `resource_0x...` buffer: start address and dumped extent
    resources: Vec<(u32, u32)>,
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

fn parse_dump(path: &str) -> Dump {
    let text = read(path);
    let mut jobs = Vec::new();
    let mut mem: BTreeMap<u32, u32> = BTreeMap::new();
    let mut fs: Vec<u32> = Vec::new();
    let mut unif: Vec<u32> = Vec::new();
    let mut extent: BTreeMap<String, u32> = BTreeMap::new();
    let mut addr: Option<u32> = None;
    let mut cur: Option<(String, u32)> = None;
    let finish = |mem: &BTreeMap<u32, u32>, fs: &[u32], unif: &[u32]| -> Vec<[u32; 6]> {
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
                recs.push(std::array::from_fn(|k| mem[&(ta + 4 * k as u32)]));
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
            // `@format blank N /* [name+0xlo..0xhi] */`: zero words
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
    Dump { jobs, resources }
}

/// (ver, iprev) from the `.stdout`'s `renderer V3D 7.1.10.2`.
fn revision(shape: &str) -> (u32, u32) {
    let out = read(&format!("tools/v3d/pi5/{shape}.stdout"));
    let r = out.lines().find_map(|l| l.strip_prefix("gl_shapes: renderer V3D ")).expect("renderer line");
    let n: Vec<u32> = r.trim().split('.').map(|x| x.parse().unwrap()).collect();
    (n[0] * 10 + n[1], n[2])
}

/// The record words of the README table, per shape ("the same record"
/// repeats the row above).
fn readme_records() -> BTreeMap<String, Vec<[u32; 6]>> {
    let md = read("tools/v3d/pi5/README.md");
    let mut out: BTreeMap<String, Vec<[u32; 6]>> = BTreeMap::new();
    let mut last: Option<[u32; 6]> = None;
    for l in md.lines() {
        let Some(r) = l.strip_prefix("| `gl_") else { continue };
        let shape = format!("gl_{}", &r[..r.find('`').unwrap()]);
        let cols: Vec<&str> = l.split('|').collect();
        let c = cols[2].trim();
        // only the record table: its second column is the six words
        if c != "the same record" && !c.starts_with("`0x") {
            continue;
        }
        let words = if c == "the same record" {
            last.expect("a record above")
        } else {
            let w: Vec<u32> = c.trim_matches('`').split(' ').map(hex).collect();
            w.try_into().expect("six words")
        };
        last = Some(words);
        out.entry(shape).or_default().push(words);
    }
    out
}

// ---- the cases' GL descriptions ---------------------------------------------

const GL_RGBA: u32 = 0x1908;
const GL_RGB: u32 = 0x1907;
const GL_RGBA8: u32 = 0x8058;
const GL_SRGB8_ALPHA8: u32 = 0x8C43;
const GL_RGB565: u32 = 0x8D62;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_UNSIGNED_SHORT_5_6_5: u32 = 0x8363;

fn gl(target: Target, internal_format: u32, ty: u32, size: u32, levels: u32) -> GlTex71 {
    GlTex71 {
        target,
        internal_format,
        ty,
        width: size,
        height: size,
        levels,
        base_level: 0,
        max_level: GL_MAX_LEVEL_DEFAULT,
        swizzle: GL_SWIZZLE_IDENTITY,
        srgb_decode: true,
        bo_address: 0,
        ver: 71,
        iprev: 10,
    }
}

/// The texture each job samples, as README.md "The GL calls" gives it.
fn case_texture(shape: &str, job: usize) -> GlTex71 {
    match (shape, job) {
        ("gl_tex2d", 1 | 2) => gl(Target::Tex2d, GL_RGBA8, GL_UNSIGNED_BYTE, 64, 1),
        ("gl_texfmt", 1) => gl(Target::Tex2d, GL_SRGB8_ALPHA8, GL_UNSIGNED_BYTE, 64, 1),
        ("gl_texfmt", 2) => {
            let _ = GL_RGB; // glTexImage2D's format; the internal format is sized
            gl(Target::Tex2d, GL_RGB565, GL_UNSIGNED_SHORT_5_6_5, 64, 1)
        }
        ("gl_mipgen", 1) => gl(Target::Tex2d, GL_RGBA8, GL_UNSIGNED_BYTE, 64, 7),
        ("gl_cube", 1..=12) => gl(Target::Cube, GL_RGBA, GL_UNSIGNED_BYTE, 4, 1),
        _ => panic!("no texture for {shape} job {job}"),
    }
}

/// Every field of the 7.1 record from the composed values; the ones
/// `v3d_tex71_pack_state` writes itself (the chroma pointers, base >> 6,
/// offsets 1, v3dx_state.c:963-969) and those nothing sets (flips, UIF XOR
/// disable, pad) as v3d_tex71.c writes them.
fn expected_fields(t: &Tex71) -> BTreeMap<String, u64> {
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
    // the C writer's
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

const SHAPES: [(&str, usize); 4] = [("gl_tex2d", 2), ("gl_texfmt", 2), ("gl_cube", 12), ("gl_mipgen", 1)];

#[test]
fn composed_records_match_mesa() {
    let fields = tex_fields();
    let readme = readme_records();
    let mut checked = 0;
    for (shape, njobs) in SHAPES {
        let dump = parse_dump(&format!("tools/v3d/pi5/{shape}.clif"));
        let (ver, iprev) = revision(shape);
        assert_eq!((ver, iprev), (71, 10), "{shape}: the Pi 5's 7.1.10");
        assert_eq!(dump.jobs.len(), njobs, "{shape}: jobs");
        for (j, recs) in dump.jobs.iter().enumerate() {
            let job = j + 1;
            assert_eq!(recs.len(), 1, "{shape} job {job}: one texture record");
            let words = recs[0];
            assert!(readme[shape].contains(&words), "{shape} job {job}: {words:08x?} not in the README table");
            let got = decode(&words, &fields);
            assert_eq!(encode(&got, &fields), words, "{shape} job {job}: the field list covers the record");
            // the buffer the record's base points into
            let base = got["Texture base pointer"] as u32;
            let &(bo, extent) =
                dump.resources.iter().filter(|(a, _)| *a <= base).max_by_key(|(a, _)| *a).expect("a resource");
            let mut desc = case_texture(shape, job);
            desc.bo_address = bo;
            desc.ver = ver;
            desc.iprev = iprev;
            let t = compose(&desc).unwrap_or_else(|e| panic!("{shape} job {job}: {e}"));
            let want = expected_fields(&t);
            assert_eq!(
                want.keys().collect::<Vec<_>>(),
                got.keys().collect::<Vec<_>>(),
                "every field of the 7.1 record is accounted for"
            );
            for (k, v) in &got {
                assert_eq!(want[k], *v, "{shape} job {job}: {k}");
            }
            assert_eq!(encode(&want, &fields), words, "{shape} job {job}: the six words");
            assert!(t.layout.size <= extent, "{shape}: layout {} past the dumped buffer {extent}", t.layout.size);
            assert_eq!(t.base, bo + t.layout.slices[0].offset);
            checked += 1;
        }
    }
    assert_eq!(checked, 17);
}

#[test]
fn layout_and_side_values_of_the_references() {
    // what the record does not show, but the composition decides
    let d = |shape, job| {
        let mut g = case_texture(shape, job);
        g.bo_address = 0x10_0000;
        compose(&g).unwrap()
    };
    let cube = d("gl_cube", 1);
    assert_eq!(cube.layout.slices[0].tiling, Tiling::LinearTile);
    assert_eq!(cube.cube_face_stride, cube.array_stride_64 * 64);
    assert_eq!(cube.layout.size, 6 * cube.cube_face_stride);
    assert_eq!(cube.layout.offset(0, 5), Some(5 * cube.cube_face_stride));
    assert_eq!(cube.layout.offset(0, 6), None);
    let tex = d("gl_tex2d", 1);
    assert_eq!(tex.cube_face_stride, 0);
    assert_eq!(tex.layout.slices[0].tiling, Tiling::UifNoXor);
    assert!(!tex.return_32 && tex.row.pipe == "R8G8B8A8_UNORM");
    let mip = d("gl_mipgen", 1);
    // levels from the smallest up, level 0 last and on 4 KiB
    let s = &mip.layout.slices;
    assert_eq!(s.len(), 7);
    assert!(s.windows(2).all(|w| w[0].offset > w[1].offset));
    assert_eq!(s[0].offset % 4096, 0);
    assert_eq!(s[6].offset, 4096 - s[1..].iter().map(|x| x.size).sum::<u32>() % 4096);
    let tilings: Vec<Tiling> = s.iter().map(|x| x.tiling).collect();
    use Tiling::*;
    assert_eq!(tilings, [UifNoXor, UifNoXor, UbLinear2Column, UbLinear1Column, LinearTile, LinearTile, LinearTile]);
}

#[test]
fn gl_state_reaches_the_record() {
    let mut g = case_texture("gl_texfmt", 2);
    g.bo_address = 0x10_0000;
    // GL swizzle composed with RGB565's XYZ1
    g.swizzle = [GL_BLUE, GL_GREEN, GL_RED, GL_ALPHA];
    assert_eq!(compose(&g).unwrap().swizzle, [4, 3, 2, 1]);
    g.swizzle = [GL_ZERO, GL_ONE, GL_RED, GL_RED];
    assert_eq!(compose(&g).unwrap().swizzle, [0, 1, 2, 2]);
    // SKIP_DECODE views an sRGB format as linear
    let mut s = case_texture("gl_texfmt", 1);
    assert_eq!(compose(&s).unwrap().transfer_func, texstate71::TRANSFER_SRGB);
    s.srgb_decode = false;
    assert_eq!(compose(&s).unwrap().transfer_func, texstate71::TRANSFER_NONE);
    // the view's levels: base and max level against the storage
    let mut m = case_texture("gl_mipgen", 1);
    m.max_level = 0;
    assert_eq!((compose(&m).unwrap().base_level, compose(&m).unwrap().max_level), (0, 0));
    m.base_level = 2;
    m.max_level = 4;
    let t = compose(&m).unwrap();
    assert_eq!((t.base_level, t.max_level), (2, 4));
    // the base stays level 0's (v3d_layer_offset(prsc, 0, 0))
    assert_eq!(t.base, t.layout.slices[0].offset);
    // R/B swap and reverse are 0 at every 7.1 revision the table takes
    for iprev in [5, 10, 15] {
        let t = compose(&GlTex71 { iprev, ..case_texture("gl_texfmt", 2) }).unwrap();
        assert!(!t.r_b_swap && !t.reverse);
    }
    // a 32-bit return format
    let f = compose(&GlTex71 { internal_format: 0x8814, ..case_texture("gl_tex2d", 1) }).unwrap(); // GL_RGBA32F
    assert!(f.return_32 && f.layout.cpp == 16);
}

#[test]
fn every_uncompressed_row_has_a_texel_size() {
    for r in format71::MESA_ROWS.iter() {
        match cpp_of(r.pipe) {
            Ok(c) => assert!([1, 2, 4, 8, 16].contains(&c), "{}", r.pipe),
            Err(Refusal::Compressed { .. }) => {
                assert!(r.pipe.starts_with("ETC2") || r.pipe.starts_with("DXT") || r.pipe.starts_with("ASTC"))
            }
            Err(Refusal::SeparateStencil { .. }) => assert!(r.pipe.contains("S8X24")),
            Err(e) => panic!("{}: {e}", r.pipe),
        }
    }
    for (pipe, cpp) in [
        ("R8_UNORM", 1),
        ("B5G6R5_UNORM", 2),
        ("A4B4G4R4_UNORM", 2),
        ("R8G8B8A8_UNORM", 4),
        ("R10G10B10A2_UNORM", 4),
        ("R11G11B10_FLOAT", 4),
        ("R9G9B9E5_FLOAT", 4),
        ("S8_UINT_Z24_UNORM", 4),
        ("Z16_UNORM", 2),
        ("R16G16B16A16_FLOAT", 8),
        ("R32G32B32A32_UINT", 16),
    ] {
        assert_eq!(cpp_of(pipe), Ok(cpp), "{pipe}");
    }
}

#[test]
fn struct_field_names_match_v3d_tex71_h() {
    let h = read("driver/src/v3d/v3d_tex71.h");
    let s = &h[h.find("struct v3d_tex71_state {").unwrap()..];
    let s = &s[..s.find("};").unwrap()];
    // comments out
    let mut body = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        body.push_str(&rest[..i]);
        rest = &rest[i + rest[i..].find("*/").unwrap() + 2..];
    }
    body.push_str(rest);
    let toks: Vec<&str> = body.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|t| !t.is_empty()).collect();
    let names: Vec<&str> = toks
        .into_iter()
        .filter(|t| !matches!(*t, "struct" | "v3d_tex71_state" | "uint32_t" | "uint8_t" | "bool" | "4"))
        .collect();
    let ours = [
        "base",
        "width",
        "height",
        "depth",
        "base_level",
        "max_level",
        "type",
        "swizzle",
        "transfer_func",
        "r_b_swap",
        "reverse",
        "level_0_is_strictly_uif",
        "level_0_xor_enable",
        "extended",
        "level_0_ub_pad",
        "array_stride_64",
    ];
    assert_eq!(names, ours);
    // the same fields in the Rust struct (`type` is `tex_type`), compile-time
    let mut g = case_texture("gl_tex2d", 1);
    g.bo_address = 0x10_0000;
    let Tex71 {
        base: _,
        width: _,
        height: _,
        depth: _,
        base_level: _,
        max_level: _,
        tex_type: _,
        swizzle: _,
        transfer_func: _,
        r_b_swap: _,
        reverse: _,
        level_0_is_strictly_uif: _,
        level_0_xor_enable: _,
        extended: _,
        level_0_ub_pad: _,
        array_stride_64: _,
        // beside the record
        row: _,
        return_32: _,
        cube_face_stride: _,
        layout: _,
    } = compose(&g).unwrap();
}

#[test]
fn refusals_are_named() {
    let ok = GlTex71 { bo_address: 0x10_0000, ..case_texture("gl_tex2d", 1) };
    assert!(compose(&ok).is_ok());
    let r = |t: GlTex71| compose(&t).unwrap_err();
    use texstate71::Refusal as S;
    let cases = [
        (r(GlTex71 { internal_format: 0x1234, ..ok }), Refusal::Format(format71::Refusal71::UnknownFormat)),
        (r(GlTex71 { internal_format: 0x881B, ..ok }), Refusal::Format(format71::Refusal71::NoThreeChannelRow)),
        (r(GlTex71 { internal_format: 0x9274, ..ok }), Refusal::Compressed { pipe: "ETC2_RGB8" }),
        (r(GlTex71 { internal_format: 0x8CAD, ..ok }), Refusal::SeparateStencil { pipe: "Z32_FLOAT_S8X24_UINT" }),
        (r(GlTex71 { swizzle: [GL_RED, 0x1234, GL_BLUE, GL_ALPHA], ..ok }), Refusal::SwizzleSource { value: 0x1234 }),
        (r(GlTex71 { width: 0, ..ok }), Refusal::ZeroSize),
        (r(GlTex71 { levels: 0, ..ok }), Refusal::ZeroSize),
        (r(GlTex71 { levels: 8, ..ok }), Refusal::LevelsBeyondChain { levels: 8, chain: 7 }),
        (r(GlTex71 { base_level: 1, ..ok }), Refusal::BaseLevelMissing { base_level: 1, levels: 1 }),
        (
            r(GlTex71 { levels: 2, base_level: 1, max_level: 0, ..ok }),
            Refusal::BaseAboveMax { base_level: 1, max_level: 0 },
        ),
        (r(GlTex71 { bo_address: 0xffff_f000, ..ok }), Refusal::AddressOverflow),
        (r(GlTex71 { width: 4097, ..ok }), Refusal::State(S::DimensionTooLarge { dim: 4097 })),
        (r(GlTex71 { ver: 42, ..ok }), Refusal::State(S::NotV3d71 { ver: 42 })),
        (r(GlTex71 { iprev: 2, ..ok }), Refusal::State(S::RevisionBelow715 { iprev: 2 })),
        (r(GlTex71 { target: Target::Cube, height: 32, ..ok }), Refusal::State(S::CubeNotSquare)),
        (r(GlTex71 { bo_address: 0x10_0020, ..ok }), Refusal::State(S::BaseNotAligned { base: 0x10_0020 })),
        (layout(16384, 16, 1, 4, 1).unwrap_err(), Refusal::LayoutSize),
        (layout(16383, 16383, 1, 16, 6).unwrap_err(), Refusal::LayoutTooLarge),
        (cpp_of("FOO").unwrap_err(), Refusal::TexelSize { pipe: "FOO" }),
    ];
    let mut texts = Vec::new();
    for (got, want) in cases {
        assert_eq!(got, want);
        let t = got.to_string();
        assert!(t.starts_with("texture 7.1: ") || t.starts_with("texture state 7.1: "), "{t}");
        texts.push(t);
    }
    texts.sort();
    let n = texts.len();
    texts.dedup();
    // ZeroSize twice; the rest each its own text
    assert_eq!(texts.len(), n - 1);
    // the largest 2D and cube textures compose
    assert!(compose(&GlTex71 { width: 4096, height: 4096, levels: 13, ..ok }).is_ok());
    assert!(compose(&GlTex71 { target: Target::Cube, width: 4096, height: 4096, levels: 13, ..ok }).is_ok());
}
