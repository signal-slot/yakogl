// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 texture format answers (src/v3d/format71.rs, with the
//! transfer-function and swap bits of src/v3d/texstate71.rs) against Mesa's
//! 7.1 reference tools/v3d/pi5/gl_texfmt.clif/.stdout, host test.
//!
//! Run: cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_texfmt
//!
//! gl_texfmt draws twice (tools/v3d/pi5/README.md, "The GL calls"): job 1
//! samples a `GL_SRGB8_ALPHA8` texture (`GL_RGBA`, `GL_UNSIGNED_BYTE`), job 2
//! a `GL_RGB565` one (`GL_RGB`, `GL_UNSIGNED_SHORT_5_6_5`). For each job this
//! reads the texture record the fragment shader samples straight out of the
//! dump (the reading rule of tools/v3d/pi5/src/texstate71.py: the fragment
//! shader's uniform stream, the TMU Config Parameter 0/1 pair, the six words
//! at P0's address), decodes texture type, swizzle, transfer function, R/B
//! swap and reverse with the 7.1 `Texture Shader State` field list of the
//! vendored v3d_packet.xml, takes the return size from P1's `Output Type
//! 32-bit` and the return words from P0, and compares each with what
//! `tmu_format71` answers for the job's GL internal format and type. Every
//! form of the two formats an ES 2.0 context can name is covered: the
//! unsized `GL_RGB`/`GL_UNSIGNED_SHORT_5_6_5` upload is RGB565's; sRGB has no
//! ES 2.0 form here (GL_EXT_sRGB is not claimed, src/gl/state.rs), and the
//! last test pins that `GL_SRGB_ALPHA_EXT` stays a named refusal.

#[path = "../src/v3d/format71.rs"]
#[allow(dead_code)]
mod format71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;

use format71::*;
use std::collections::BTreeMap;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn xml() -> String {
    read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml")
}

/// The value of attribute `a` in an xml tag.
fn attr(tag: &str, a: &str) -> String {
    let k = format!(" {a}=\"");
    let j = tag.find(&k).unwrap_or_else(|| panic!("{a} in {tag}")) + k.len();
    tag[j..j + tag[j..].find('"').unwrap()].to_string()
}

/// name -> (start, size) of every field of the 7.1 Texture Shader State.
fn tex_fields() -> BTreeMap<String, (u32, u32)> {
    let x = xml();
    let open = "<struct name=\"Texture Shader State\" min_ver=\"71\">";
    let s = x.find(open).expect("7.1 Texture Shader State");
    let block = &x[s..s + x[s..].find("</struct>").unwrap()];
    let mut out = BTreeMap::new();
    let mut at = 0;
    while let Some(i) = block[at..].find("<field ") {
        let t0 = at + i;
        let tag = &block[t0..t0 + block[t0..].find('>').unwrap()];
        out.insert(attr(tag, "name"), (attr(tag, "start").parse().unwrap(), attr(tag, "size").parse().unwrap()));
        at = t0 + tag.len();
    }
    out
}

/// TEXTURE_DATA_FORMAT value -> its xml name ("RGBA8", "RGB565", ...).
fn tex_type_names() -> BTreeMap<u32, String> {
    let x = xml();
    let s = x.find("<enum name=\"Texture Data Formats\">").expect("enum");
    let block = &x[s..s + x[s..].find("</enum>").unwrap()];
    let mut out = BTreeMap::new();
    let mut at = 0;
    while let Some(i) = block[at..].find("<value ") {
        let t0 = at + i;
        let tag = &block[t0..t0 + block[t0..].find('>').unwrap()];
        let name = attr(tag, "name");
        let name = name.strip_prefix("Texture Data Format ").unwrap_or(&name).to_string();
        out.insert(attr(tag, "value").parse().unwrap(), name);
        at = t0 + tag.len();
    }
    out
}

fn get(words: &[u32; 6], start: u32, size: u32) -> u32 {
    let bits: u128 = words[..4].iter().enumerate().map(|(i, w)| u128::from(*w) << (32 * i)).sum();
    assert!(start + size <= 128);
    ((bits >> start) & ((1u128 << size) - 1)) as u32
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

/// One sampled texture of a job: the record and the P0/P1 flag bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sampled {
    record: [u32; 6],
    p0_return_words: u32,
    p1_flags: u32,
}

/// Per job (split at `@wait_render_all_cores`), the textures its fragment
/// shader samples.
fn parse_dump(path: &str) -> Vec<Vec<Sampled>> {
    let text = read(path);
    let mut jobs = Vec::new();
    let mut mem: BTreeMap<u32, u32> = BTreeMap::new();
    let mut fs: Vec<u32> = Vec::new();
    let mut unif: Vec<u32> = Vec::new();
    let mut addr: Option<u32> = None;
    let finish = |mem: &BTreeMap<u32, u32>, fs: &[u32], unif: &[u32]| -> Vec<Sampled> {
        let mut out = Vec::new();
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
                out.push(Sampled {
                    record: std::array::from_fn(|k| mem[&(ta + 4 * k as u32)]),
                    p0_return_words: p[0] & 0xf,
                    p1_flags: p[1] & 0x7,
                });
            }
        }
        out
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
            addr = Some(hex(name.rsplit('_').next().unwrap()) + hex(off.split(']').next().unwrap()));
            continue;
        }
        if let Some(r) = line.strip_prefix("@format blank ") {
            let r = &r[r.find('[').unwrap() + 1..r.find(']').unwrap()];
            let (name, range) = r.split_once('+').unwrap();
            let (lo, hi) = range.split_once("..").unwrap();
            let base = hex(name.rsplit('_').next().unwrap());
            for a in ((base + hex(lo)) & !3..=base + hex(hi)).step_by(4) {
                mem.entry(a).or_insert(0);
            }
            addr = None;
            continue;
        }
        if line.starts_with('@') {
            addr = None;
            continue;
        }
        let t = line.trim();
        if let Some(a) = addr.as_mut() {
            if !t.is_empty() && t.split(' ').all(|w| w.len() == 10 && w.starts_with("0x")) {
                for w in t.split(' ') {
                    mem.insert(*a, hex(w));
                    *a += 4;
                }
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
    jobs
}

/// The one texture each gl_texfmt job samples.
fn sampled() -> Vec<Sampled> {
    let jobs = parse_dump("tools/v3d/pi5/gl_texfmt.clif");
    assert_eq!(jobs.len(), 2, "gl_texfmt has two draws");
    jobs.into_iter()
        .enumerate()
        .map(|(i, j)| {
            assert_eq!(j.len(), 1, "job {} samples one texture: {j:x?}", i + 1);
            j[0]
        })
        .collect()
}

// GL internal formats not in format71's enum list.
const GL_SRGB8_ALPHA8: u32 = 0x8C43;
const GL_RGB565: u32 = 0x8D62;
const GL_SRGB_ALPHA_EXT: u32 = 0x8C42;

/// One gl_texfmt job: its README name, the GL (internal format, type) pairs
/// that name the same texture (the call's sized one first; then an ES 2.0
/// unsized one, if there is one), and the expected xml type name and Mesa
/// pipe format.
struct Case {
    what: &'static str,
    gl: &'static [(u32, u32)],
    type_name: &'static str,
    pipe: &'static str,
}

const CASES: [Case; 2] = [
    Case {
        what: "job 1 GL_SRGB8_ALPHA8",
        gl: &[(GL_SRGB8_ALPHA8, GL_UNSIGNED_BYTE)],
        type_name: "RGBA8",
        pipe: "R8G8B8A8_SRGB",
    },
    Case {
        what: "job 2 GL_RGB565",
        gl: &[(GL_RGB565, GL_UNSIGNED_SHORT_5_6_5), (GL_RGB, GL_UNSIGNED_SHORT_5_6_5)],
        type_name: "RGB565",
        pipe: "B5G6R5_UNORM",
    },
];

#[test]
fn the_reference_is_a_7_1_run_without_errors() {
    let out = read("tools/v3d/pi5/gl_texfmt.stdout");
    assert!(out.contains("gl_shapes: renderer V3D 7.1."), "{out}");
    assert!(out.contains("glGetError 0x0"), "{out}");
}

#[test]
fn mesa_records_decode_to_the_pinned_values() {
    let f = tex_fields();
    let names = tex_type_names();
    let s = sampled();
    let g = |r: &[u32; 6], n: &str| {
        let (st, sz) = f[n];
        get(r, st, sz)
    };
    // (type name, swizzle R G B A, transfer func, R/B swap, reverse,
    //  P0 return words, P1 output 32-bit)
    type Pinned = (&'static str, [u32; 4], u32, u32, u32, u32, u32);
    let pinned: [Pinned; 2] = [("RGBA8", [2, 3, 4, 5], 1, 0, 0, 3, 0), ("RGB565", [2, 3, 4, 1], 0, 0, 0, 3, 0)];
    for (i, (s, p)) in s.iter().zip(pinned).enumerate() {
        let r = &s.record;
        let ty = g(r, "Texture type");
        assert_eq!(names[&ty], p.0, "job {}", i + 1);
        let swz = [g(r, "Swizzle R"), g(r, "Swizzle G"), g(r, "Swizzle B"), g(r, "Swizzle A")];
        assert_eq!(swz, p.1, "job {}", i + 1);
        assert_eq!(g(r, "Transfer func"), p.2, "job {}", i + 1);
        assert_eq!(g(r, "R/B swap"), p.3, "job {}", i + 1);
        assert_eq!(g(r, "Reverse"), p.4, "job {}", i + 1);
        assert_eq!(s.p0_return_words, p.5, "job {}", i + 1);
        assert_eq!(s.p1_flags & 1, p.6, "job {}", i + 1);
    }
}

#[test]
fn format71_answers_match_mesa_records() {
    let f = tex_fields();
    let names = tex_type_names();
    let g = |r: &[u32; 6], n: &str| {
        let (st, sz) = f[n];
        get(r, st, sz)
    };
    for (s, c) in sampled().iter().zip(CASES.iter()) {
        let r = &s.record;
        let mesa_type = g(r, "Texture type");
        assert_eq!(names[&mesa_type], c.type_name, "{}", c.what);
        let mesa_swz = [g(r, "Swizzle R"), g(r, "Swizzle G"), g(r, "Swizzle B"), g(r, "Swizzle A")];
        let mesa_srgb = g(r, "Transfer func") == texstate71::TRANSFER_SRGB;
        let mesa_32 = s.p1_flags & 1 == 1;
        for &(internal, ty) in c.gl {
            let what = format!("{} as ({internal:#06x}, {ty:#06x})", c.what);
            let a = tmu_format71(internal, ty).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(a.pipe, c.pipe, "{what}");
            assert_eq!(a.tex_type, mesa_type, "{what}: texture type");
            assert_eq!(a.swizzle.map(u32::from), mesa_swz, "{what}: swizzle");
            assert_eq!(a.transfer == Transfer::Srgb, mesa_srgb, "{what}: sRGB");
            assert_eq!(a.r_b_swap, g(r, "R/B swap") == 1, "{what}: R/B swap");
            assert_eq!(a.reverse, g(r, "Reverse") == 1, "{what}: reverse");
            assert_eq!(a.return_32, mesa_32, "{what}: return size");
            // Mesa's row: return size 16 or 32, return channels 0 (the
            // shader's vec4 read decides the channels, P0's word mask).
            let m = mesa_row(a.pipe).unwrap();
            assert_eq!(m.return_size, if mesa_32 { 32 } else { 16 }, "{what}");
            assert_eq!(m.return_channels, 0, "{what}");
            // a vec4 read: 4 channels of 16 bits are 2 words, of 32 bits 4
            let words = if m.return_size == 32 { 4 } else { 2 };
            assert_eq!(s.p0_return_words, (1 << words) - 1, "{what}: P0 return words");
        }
    }
}

#[test]
fn texstate71_writes_the_mesa_transfer_and_swap_bits() {
    let f = tex_fields();
    for (s, c) in sampled().iter().zip(CASES.iter()) {
        let a = tmu_format71(c.gl[0].0, c.gl[0].1).unwrap();
        let d = texstate71::TexDesc {
            width: 64,
            height: 64,
            depth: 1,
            levels: 1,
            base_level: 0,
            cube: false,
            srgb: a.transfer == Transfer::Srgb,
            transfer_func: None,
            bytes_per_texel: 4,
            layer_stride: 0,
            base: 0,
            ver: 71,
            iprev: 10,
        };
        let st = texstate71::texture_state(&d).unwrap_or_else(|e| panic!("{}: {e:?}", c.what));
        let w = texstate71::pack_words(&st);
        for n in ["Transfer func", "R/B swap", "Reverse"] {
            let (start, size) = f[n];
            assert_eq!(get(&w, start, size), get(&s.record, start, size), "{}: {n}", c.what);
        }
    }
}

#[test]
fn srgb_has_no_es2_form() {
    // GL_EXT_sRGB is not claimed (src/gl/state.rs), so its unsized
    // GL_SRGB_ALPHA_EXT is no format the 7.1 table answers.
    assert_eq!(tmu_format71(GL_SRGB_ALPHA_EXT, GL_UNSIGNED_BYTE), Err(Refusal71::UnknownFormat));
}
