// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/mip71.rs` (glGenerateMipmap to `v3d_tfu71_pack` inputs), host test.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_mip
//! ```
//!
//! The gl_mipgen GL setup (64x64 GL_RGBA8, 7 levels) goes through
//! `mip71::generate`, the arguments through the driver's `v3d_tfu71_pack` (a C
//! harness, tests/v3d71_mip_harness.c, built here with `$CC` or `cc`; without a
//! compiler the C half prints a skip line and passes), and the twelve words are
//! compared with Mesa 7.1's SUBMIT_TFU line in tools/v3d/pi5/gl_mipgen.stdout
//! (IOC 0x00086061). A 256x256 chain is compared with tools/v3d/tfu71_golden.tsv.
//! The Rust half (the argument values) always runs. Refusals are checked by name.

#[path = "../src/v3d/ffi71.rs"]
#[allow(dead_code)]
mod ffi71;
#[path = "../src/v3d/format71.rs"]
#[allow(dead_code)]
mod format71;
#[path = "../src/v3d/mip71.rs"]
mod mip71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;
#[path = "../src/v3d/tfuplan71.rs"]
#[allow(dead_code)]
mod tfuplan71;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use ffi71::V3dTfu71Args;
use mip71::{generate, Refusal, Shape};
use texture71::{GlTex71, Target, GL_MAX_LEVEL_DEFAULT, GL_SWIZZLE_IDENTITY};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const GL_RGBA8: u32 = 0x8058;
const GL_UNSIGNED_BYTE: u32 = 0x1401;

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

fn tex(size: u32, levels: u32, bo: u32) -> GlTex71 {
    GlTex71 {
        target: Target::Tex2d,
        internal_format: GL_RGBA8,
        ty: GL_UNSIGNED_BYTE,
        width: size,
        height: size,
        levels,
        base_level: 0,
        max_level: GL_MAX_LEVEL_DEFAULT,
        swizzle: GL_SWIZZLE_IDENTITY,
        srgb_decode: true,
        bo_address: bo,
        ver: 71,
        iprev: 10,
    }
}

/// Mesa's gl_mipgen job: icfg iia iis ica iua ioa ios coef0..3 ioc.
fn mesa_mipgen() -> [u32; 12] {
    let s = read("tools/v3d/pi5/gl_mipgen.stdout");
    let l = s.lines().find(|l| l.contains("SUBMIT_TFU icfg ")).expect("SUBMIT_TFU line");
    let t: Vec<&str> = l.split_whitespace().collect();
    let at = |k: &str| t.iter().position(|x| *x == k).unwrap_or_else(|| panic!("no {k}"));
    let mut w = [0u32; 12];
    w[0] = hex(t[at("icfg") + 1]);
    w[1] = hex(t[at("iia") + 1]);
    w[2] = hex(t[at("iis") + 1]);
    w[3] = hex(t[at("ica") + 1]);
    w[4] = hex(t[at("iua") + 1]);
    w[5] = hex(t[at("ioa") + 1]);
    w[6] = hex(t[at("ios") + 1]);
    for k in 0..4 {
        w[7 + k] = hex(t[at("coef") + 1 + k]);
    }
    w[11] = hex(t[at("ioc") + 1]);
    w
}

fn raw<T>(v: &T) -> Vec<u8> {
    // SAFETY: a plain repr(C) struct of u32s read as bytes.
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

const HARNESS_C: &str = include_str!("v3d71_mip_harness.c");

fn build_harness(cc: &str) -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| build_harness_once(cc)).clone()
}

fn build_harness_once(cc: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d71_mip");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("harness");
    let src = dir.join("harness.c");
    std::fs::write(&src, HARNESS_C).unwrap();
    let mut parts = cc.split_whitespace();
    let out = Command::new(parts.next().unwrap())
        .args(parts)
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg(format!("-I{ROOT}/driver/src/v3d"))
        .arg(&src)
        .arg(format!("{ROOT}/driver/src/v3d/v3d_tfu71.c"))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run the C compiler");
    assert!(out.status.success(), "harness build failed:\n{}", String::from_utf8_lossy(&out.stderr));
    exe
}

/// Packs each args through the C packer: twelve words per job.
fn c_pack(exe: &PathBuf, args: &[V3dTfu71Args]) -> Result<Vec<[u32; 12]>, i32> {
    let mut child = Command::new(exe).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let input: Vec<u8> = args.iter().flat_map(raw).collect();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().unwrap();
    if !out.status.success() {
        return Err(out.status.code().unwrap_or(-1));
    }
    assert_eq!(out.stdout.len(), 48 * args.len());
    Ok(out
        .stdout
        .chunks(48)
        .map(|c| {
            let mut w = [0u32; 12];
            for (k, x) in c.chunks(4).enumerate() {
                w[k] = u32::from_ne_bytes(x.try_into().unwrap());
            }
            w
        })
        .collect())
}

/// gl_mipgen: glTexStorage2D(GL_TEXTURE_2D, 7, GL_RGBA8, 64, 64); the buffer is
/// drm-shim's resource_0xf000 and level 0 starts 8 KiB in (0x11000).
fn mipgen_jobs() -> Vec<mip71::MipJob> {
    generate(Shape::Tex2d, &tex(64, 7, 0xf000)).expect("gl_mipgen shape")
}

#[test]
fn mipgen_args() {
    let j = mipgen_jobs();
    assert_eq!(j.len(), 1);
    let a = j[0].args;
    assert_eq!((a.ttype, a.cpp, a.width, a.height, a.num_mm), (4, 4, 64, 64, 6));
    assert_eq!((a.src.addr, a.src.tiling, a.src.stride_or_padded_height), (0x11000, 4, 64));
    assert_eq!(a.src, a.dst);
}

#[test]
fn mipgen_words_equal_mesa() {
    let Some(cc) = find_cc() else {
        println!("skip: no C compiler (set CC or install cc): the v3d_tfu71_pack half of v3d71_mip not run");
        return;
    };
    let exe = build_harness(&cc);
    let args: Vec<V3dTfu71Args> = mipgen_jobs().iter().map(|j| j.args).collect();
    let w = c_pack(&exe, &args).expect("v3d_tfu71_pack");
    assert_eq!(w.len(), 1);
    assert_eq!(w[0], mesa_mipgen(), "gl_mipgen TFU words (icfg iia iis ica iua ioa ios coef0..3 ioc)");
    assert_eq!(w[0][11], 0x0008_6061);
}

#[test]
fn rgba8_256_matches_golden() {
    let Some(cc) = find_cc() else {
        println!("skip: no C compiler: golden half not run");
        return;
    };
    let exe = build_harness(&cc);
    // golden: `case 4 4 256 256 8 20000000 5 256 20000000 5 256 1 <12 words>`
    let g = read("tools/v3d/tfu71_golden.tsv");
    let l =
        g.lines().find(|l| l.starts_with("case 4 4 256 256 8 20000000 5 256 20000000 5 256 1 ")).expect("golden case");
    let want: Vec<u32> = l.split_whitespace().skip(13).map(hex).collect();
    assert_eq!(want.len(), 12);
    let off = texture71::layout(256, 256, 9, 4, 1).unwrap().slices[0].offset;
    let j = generate(Shape::Tex2d, &tex(256, 9, 0x2000_0000 - off)).unwrap();
    assert_eq!(j.len(), 1);
    let w = c_pack(&exe, &[j[0].args]).unwrap();
    assert_eq!(w[0].to_vec(), want);
}

#[test]
fn nothing_to_generate_is_no_jobs() {
    assert!(generate(Shape::Tex2d, &tex(64, 1, 0x10000)).unwrap().is_empty());
    let mut t = tex(64, 7, 0x10000);
    t.max_level = 0;
    assert!(generate(Shape::Tex2d, &t).unwrap().is_empty());
}

#[test]
fn max_level_limits_the_chain() {
    let mut t = tex(64, 7, 0x10000);
    t.max_level = 2;
    assert_eq!(generate(Shape::Tex2d, &t).unwrap()[0].args.num_mm, 2);
}

#[test]
fn base_level_is_not_refused() {
    let mut t = tex(64, 7, 0x10000);
    t.base_level = 1;
    let j = generate(Shape::Tex2d, &t).unwrap();
    assert_eq!((j[0].args.width, j[0].args.num_mm), (32, 5));
}

#[test]
fn cube_gives_a_job_per_face() {
    let mut t = tex(64, 7, 0x10000);
    t.target = Target::Cube;
    let j = generate(Shape::Cube, &t).unwrap();
    assert_eq!(j.len(), 6);
    assert_eq!(j[1].args.src.addr - j[0].args.src.addr, texture71::layout(64, 64, 7, 4, 6).unwrap().layer_stride);
    assert!(j.iter().enumerate().all(|(i, x)| x.layer == i as u32));
}

#[test]
fn refusals_by_name() {
    let t = tex(64, 7, 0x10000);
    assert_eq!(generate(Shape::Tex3d, &t).unwrap_err(), Refusal::Texture3d);
    assert_eq!(generate(Shape::Array2d, &t).unwrap_err(), Refusal::TextureArray);
    assert_eq!(generate(Shape::Cube, &t).unwrap_err(), Refusal::ShapeMismatch);
    let mut c = t;
    c.internal_format = 0x9278; // GL_COMPRESSED_RGBA8_ETC2_EAC
    let e = generate(Shape::Tex2d, &c).unwrap_err();
    assert!(matches!(e, Refusal::Compressed | Refusal::Texture(_)), "{e:?}");
    println!("compressed: {}", e.name());
    // more levels than the chain has
    let e = generate(Shape::Tex2d, &tex(64, 8, 0x10000)).unwrap_err();
    assert!(matches!(e, Refusal::Texture(texture71::Refusal::LevelsBeyondChain { .. })));
    assert_eq!(e.name(), "mip71: texture refused by texture71");
    assert_eq!(Refusal::Texture3d.name(), "mip71: 3d texture");
    assert_eq!(Refusal::TextureArray.name(), "mip71: array texture");
}
