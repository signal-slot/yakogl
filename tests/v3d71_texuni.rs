// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 texture uniform words (src/v3d/texuni71.rs) against the fragment
//! uniform streams of Mesa's 7.1 references tools/v3d/pi5/gl_tex2d.clif,
//! gl_texfmt.clif and gl_cube.clif (every job: P0, P1, and the size the
//! texture record in the dump holds), and every refusal.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_texuni`

#![allow(dead_code)]

#[path = "../src/v3d/texuni71.rs"]
mod texuni71;

use std::collections::HashMap;
use texuni71::*;

fn read(p: &str) -> String {
    std::fs::read_to_string(format!("{}/{p}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

struct Job {
    mem: HashMap<u32, u32>,
    /// the fragment streams, each up to the next stream address
    streams: Vec<Vec<u32>>,
}

fn hexs(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

/// `name_0xBASE+0xOFF` -> BASE + OFF
fn at(s: &str) -> u32 {
    let (nb, off) = s.split_once('+').unwrap();
    hexs(nb.rsplit_once("_0x").unwrap().1) + hexs(off)
}

/// The jobs of a Mesa clif, as tools/v3d/pi5/src/texstate71.py reads them.
fn jobs(clif: &str) -> Vec<Job> {
    let mut out = Vec::new();
    let mut mem: HashMap<u32, u32> = HashMap::new();
    let mut recs: Vec<u32> = Vec::new();
    let mut unif: Vec<u32> = Vec::new();
    let mut addr: Option<u32> = None;
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
                let lo = at(lo);
                let hi = at(&format!("{}+{hi}", inner.split('+').next().unwrap()));
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

fn finish(mem: HashMap<u32, u32>, recs: Vec<u32>, unif: Vec<u32>) -> Job {
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
}

/// (width, height, depth) of a 7.1 Texture Shader State (Image Width at bit
/// 58, Height 72, Depth 86, 14 bits each).
fn size_of(w: &[u32]) -> (u32, u32, u32) {
    let bits = (w[0] as u128) | (w[1] as u128) << 32 | (w[2] as u128) << 64 | (w[3] as u128) << 96;
    let f = |s: u32| ((bits >> s) & 0x3fff) as u32;
    (f(58), f(72), f(86))
}

/// Every (P0, P1) pair of the three references' fragment streams, with the
/// texture record words at the address.
fn pairs() -> Vec<(&'static str, usize, u32, u32, [u32; 6])> {
    let mut v = Vec::new();
    for shape in ["gl_tex2d", "gl_texfmt", "gl_cube"] {
        for (n, job) in jobs(&read(&format!("tools/v3d/pi5/{shape}.clif"))).iter().enumerate() {
            for s in &job.streams {
                assert_eq!(s.len(), 2, "{shape} job {n}: the stream is P0, P1");
                let (ta, sa) = (s[0] & !0xf, s[1] & !7);
                let tw: [u32; 6] = std::array::from_fn(|k| job.mem[&(ta + 4 * k as u32)]);
                assert!((0..6).all(|k| job.mem.contains_key(&(sa + 4 * k))));
                v.push((shape, n + 1, s[0], s[1], tw));
            }
        }
    }
    v
}

#[test]
fn the_references_hold_the_expected_jobs() {
    let p = pairs();
    let count = |s| p.iter().filter(|x| x.0 == s).count();
    assert_eq!((count("gl_tex2d"), count("gl_texfmt"), count("gl_cube")), (2, 2, 12));
    // P0: return words 3; P1: per-pixel mask only
    for x in &p {
        assert_eq!((x.2 & 0xf, x.3 & 7), (3, 4), "{} job {}", x.0, x.1);
    }
}

/// The compiler's data for the program's sampler 0 (select.rs): P0 unit 0
/// with 3 return words, P1 unit 0 with the per-pixel mask.
const DATA_P0: u32 = 3;
const DATA_P1: u32 = 4;

#[test]
fn p0_p1_and_sizes_match_every_reference_job() {
    for (shape, job, p0, p1, tw) in pairs() {
        let (w, h, d) = size_of(&tw);
        let cube = shape == "gl_cube";
        // the cube record holds the six faces as layers; the size word of a
        // cube is its 2D face size and a depth of 1 (Mesa: not a 3D texture)
        let u = Unit {
            unit: 0,
            tex_state: p0 & !0xf,
            sampler: p1 & !7,
            width: w,
            height: h,
            depth: if cube { 1 } else { d },
            array_size: 1,
        };
        let us = [u];
        assert_eq!(tmu_p0(DATA_P0, &us), Ok(p0), "{shape} {job} P0");
        assert_eq!(tmu_p1(DATA_P1, &us), Ok(p1), "{shape} {job} P1");
        let e = if cube { 4 } else { 64 };
        assert_eq!((w, h), (e, e), "{shape} {job}");
        assert_eq!(size_word(TEXTURE_WIDTH, 0, &us), Ok(e));
        assert_eq!(size_word(TEXTURE_HEIGHT, 0, &us), Ok(e));
        assert_eq!(size_word(TEXTURE_DEPTH, 0, &us), Ok(1));
        assert_eq!(size_word(TEXTURE_ARRAY_SIZE, 0, &us), Ok(1));
        assert_eq!(d, if cube { 6 } else { 1 }, "{shape} {job}");
    }
}

fn unit(unit: u32, tex_state: u32, sampler: u32) -> Unit {
    Unit { unit, tex_state, sampler, width: 8, height: 8, depth: 1, array_size: 1 }
}

#[test]
fn the_chosen_addresses_move_the_words() {
    let us = [unit(0, 0x4_0020, 0x4_1008)];
    assert_eq!(tmu_p0(3, &us), Ok(0x4_0023));
    assert_eq!(tmu_p1(4, &us), Ok(0x4_100c));
    // flags: unnormalized (2) and 32-bit output (1)
    assert_eq!(tmu_p1(4 | 2 | 1, &us), Ok(0x4_100f));
    assert_eq!(word(TMU_CONFIG_P0, 3, &us), Ok(0x4_0023));
    assert_eq!(word(TEXTURE_WIDTH, 0, &us), Ok(8));
}

#[test]
fn the_unit_comes_from_the_top_byte() {
    let mut u2 = unit(2, 0x2000, 0x2100);
    u2.depth = 3;
    u2.array_size = 5;
    let us = [unit(0, 0x1000, 0x1100), u2];
    assert_eq!(tmu_p0(2 << 24 | 3, &us), Ok(0x2003));
    assert_eq!(tmu_p1(2 << 24 | 4, &us), Ok(0x2104));
    assert_eq!(size_word(TEXTURE_DEPTH, 2, &us), Ok(3));
    assert_eq!(size_word(TEXTURE_ARRAY_SIZE, 2, &us), Ok(5));
    assert_eq!(size_word(TEXTURE_WIDTH, 1, &us), Err(Refusal::NoUnit { unit: 1 }));
}

#[test]
fn refusals_are_named() {
    let us = [unit(0, 0x1000, 0x1100)];
    assert_eq!(tmu_p0(1 << 24 | 3, &us), Err(Refusal::NoUnit { unit: 1 }));
    assert_eq!(tmu_p0(0x13, &us), Err(Refusal::DataBits { contents: TMU_CONFIG_P0, data: 0x13 }));
    assert_eq!(tmu_p1(12, &us), Err(Refusal::DataBits { contents: TMU_CONFIG_P1, data: 12 }));
    let bad = [unit(0, 0x1010, 0x1104)];
    assert_eq!(tmu_p0(3, &bad), Err(Refusal::TexStateAlign { addr: 0x1010 }));
    assert_eq!(tmu_p1(4, &bad), Err(Refusal::SamplerAlign { addr: 0x1104 }));
    let mut none = unit(0, 0x1000, 0x1100);
    none.depth = 0;
    assert_eq!(size_word(TEXTURE_DEPTH, 0, &[none]), Err(Refusal::SizeNotGiven { contents: TEXTURE_DEPTH, unit: 0 }));
    assert_eq!(size_word(2, 0, &us), Err(Refusal::NotTexture { contents: 2 }));
    assert!(format!("{}", Refusal::NoUnit { unit: 3 }).contains("unit 3"));
}
