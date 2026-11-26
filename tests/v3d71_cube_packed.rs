// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The packed cube texture state against Mesa's gl_cube words.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_cube_packed
//! ```
//!
//! tests/v3d71_cube.rs compares fields; this test fills the ffi71 struct for
//! gl_cube (one level of 4x4 RGBA, six faces) with `texfill71::fill`, packs it
//! with the driver's C packer `v3d_tex71_pack_state` (tests/v3d71_texfill_harness.c,
//! reused unchanged) and compares the 24 bytes with the Texture Shader State
//! words of tools/v3d/pi5/gl_cube.clif. A field that differs is pinned in
//! `GAPS` by word, mask and both values. Without a C compiler the test prints
//! a skip line and passes.

#![allow(dead_code)]

#[path = "../src/v3d/ffi71.rs"]
mod ffi71;
#[path = "../src/v3d/format71.rs"]
mod format71;
#[path = "../src/v3d/sampler71.rs"]
mod sampler71;
#[path = "../src/v3d/texfill71.rs"]
mod texfill71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use sampler71::{Filter, Input, MipFilter, Wrap};
use texture71::{GlTex71, Target};

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

/// The texture record of every draw of the reference.
fn texture_records() -> Vec<[u32; 6]> {
    let mut v = Vec::new();
    for job in jobs(&read("tools/v3d/pi5/gl_cube.clif")) {
        for s in &job.streams {
            assert_eq!(s.len(), 2, "the stream is P0, P1");
            let ta = s[0] & !0xf;
            v.push(std::array::from_fn(|k| job.mem[&(ta + 4 * k as u32)]));
        }
    }
    v
}

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

fn sampler() -> Input {
    let mut s = Input::default_2d();
    s.min_filter = Filter::Nearest;
    s.mag_filter = Filter::Nearest;
    s.wrap_s = Wrap::ClampToEdge;
    s.wrap_t = Wrap::ClampToEdge;
    s.mip_filter = MipFilter::None;
    s
}

/// The raw bytes of a `#[repr(C)]` plain struct.
fn raw<T>(v: &T) -> Vec<u8> {
    // SAFETY: a plain repr(C) struct read as bytes; padding is never read by C.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()).to_vec() }
}

fn find_cc() -> Option<String> {
    for c in [std::env::var("CC").unwrap_or_default(), "cc".to_string()] {
        let Some(prog) = c.split_whitespace().next() else { continue };
        if Command::new(prog).arg("--version").output().is_ok() {
            return Some(c);
        }
    }
    None
}

/// The texfill test's harness, unchanged.
const HARNESS_C: &str = include_str!("v3d71_texfill_harness.c");

/// Builds the harness; the path of the executable. The directory is this
/// process's own: tools/check.sh runs this binary in several rows at once, all
/// with the same CARGO_TARGET_TMPDIR, and a fixed path let one row rewrite the
/// executable another row was running ("Text file busy", "Permission denied").
fn build_harness(cc: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("v3d71_cube_packed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("harness");
    let src = dir.join("harness.c");
    std::fs::write(&src, HARNESS_C).unwrap();
    let mesa = format!("{ROOT}/third_party/mesa-26.2.1");
    let mut parts = cc.split_whitespace();
    let out = Command::new(parts.next().unwrap())
        .args(parts)
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-D_GNU_SOURCE", "-DHAVE_ENDIAN_H", "-DV3D_CL_VER=71"])
        .arg("-isystem")
        .arg(format!("{mesa}/src"))
        .arg("-isystem")
        .arg(format!("{mesa}/include"))
        .arg("-isystem")
        .arg(format!("{mesa}/src/broadcom"))
        .arg(format!("-I{ROOT}/driver/src/v3d"))
        .arg(format!("-I{ROOT}/driver/src/v3d/generated"))
        .arg(&src)
        .arg(format!("{ROOT}/driver/src/v3d/v3d_tex71.c"))
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .output()
        .expect("run the C compiler");
    assert!(out.status.success(), "harness build failed:\n{}", String::from_utf8_lossy(&out.stderr));
    exe
}

/// The packed texture record (24 bytes) of the filled structs.
fn c_pack(exe: &PathBuf, f: &texfill71::Filled71) -> Result<[u8; 24], i32> {
    let mut child = Command::new(exe).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut input = raw(&f.state);
    input.extend(raw(&f.sampler));
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().unwrap();
    if !out.status.success() {
        return Err(out.status.code().unwrap_or(-1));
    }
    assert_eq!(out.stdout.len(), 48);
    Ok(out.stdout[..24].try_into().unwrap())
}

/// Fields where the packed bytes differ from the reference: (word, mask,
/// packed value, reference value, why). Empty: the bytes are equal.
const GAPS: &[(usize, u32, u32, u32, &str)] = &[];

#[test]
fn packed_texture_state_bytes_equal_the_reference() {
    let Some(cc) = find_cc() else {
        println!("skip: no C compiler (set CC or install cc): v3d71_cube_packed not run");
        return;
    };
    let d = texture_records();
    assert_eq!(d.len(), 12);
    let reference = d[0];
    assert!(d.iter().all(|x| *x == reference), "one texture record throughout");
    assert_eq!(reference[0] & !63, BO);
    let filled = texfill71::fill(&gl_cube(), &sampler()).expect("fill gl_cube");
    let exe = build_harness(&cc);
    let t = c_pack(&exe, &filled).unwrap_or_else(|c| panic!("the C packer exit {c}"));
    let got: Vec<u32> = (0..6).map(|i| u32::from_le_bytes(t[4 * i..4 * i + 4].try_into().unwrap())).collect();
    for (w, (g, r)) in got.iter().zip(reference.iter()).enumerate() {
        let known: u32 = GAPS.iter().filter(|x| x.0 == w).fold(0, |m, x| m | x.1);
        assert_eq!(g & !known, r & !known, "word {w}: packed {g:#010x}, gl_cube {r:#010x}");
    }
    for &(w, mask, packed, refv, why) in GAPS {
        assert_eq!((got[w] & mask, reference[w] & mask), (packed, refv), "gap word {w} ({why}) moved");
    }
}
