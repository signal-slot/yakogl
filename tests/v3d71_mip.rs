// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/mip71.rs` (`glGenerateMipmap` to the `ffi71` `V3dTfu71Args` of
//! its TFU jobs), host test against Mesa's 7.1 job of
//! tools/v3d/pi5/gl_mipgen (`glTexStorage2D` of a 7-level 64x64 `GL_RGBA8`
//! texture, `glGenerateMipmap`; tools/v3d/pi5/README.md, "The GL calls").
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_mip
//! ```
//!
//! (the features only let the crate build). Mesa's job and dump are
//! embedded with `include_str!`, so the test reads no file at run time but
//! the driver's C sources it compiles:
//!  - the plan's inputs are read back out of Mesa's register words (pure
//!    Rust): the type, size, level count, address, tiling and padded height;
//!  - where a C compiler exists (`$CC` or `cc`), tests/v3d71_mip_harness.c
//!    is built with the driver's v3d_tfu71.c, packs every job each test
//!    plans, and Mesa's job must come out word for word. Without a compiler
//!    that half prints a skip line and passes.
//!
//! Only the buffer address comes from the dump (its `@createbuf` name).

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

use ffi71::{V3dTfu71Args, V3dTfu71Job};
use mip71::{plan, MipPlan71, Refusal};
use texture71::{GlTex71, Target, GL_MAX_LEVEL_DEFAULT, GL_SWIZZLE_IDENTITY};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// Mesa's TFU job of gl_mipgen (`SUBMIT_TFU`, drm_v3d_submit_tfu's words).
const MIPGEN_STDOUT: &str = include_str!("../tools/v3d/pi5/gl_mipgen.stdout");
/// Its dump: the buffers Mesa created (`@createbuf_aligned ... resource_0x...`).
const MIPGEN_CLIF: &str = include_str!("../tools/v3d/pi5/gl_mipgen.clif");

const GL_RGBA8: u32 = 0x8058;
const GL_RGB565: u32 = 0x8D62;
const GL_RGBA32F: u32 = 0x8814;
const GL_DEPTH_COMPONENT16: u32 = 0x81A5;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_UNSIGNED_SHORT: u32 = 0x1403;
const GL_UNSIGNED_SHORT_5_6_5: u32 = 0x8363;
const GL_FLOAT: u32 = 0x1406;

/// The texture buffer of gl_mipgen: `resource_0xf000` of the dump (its
/// level 0 is the TFU's 0x00011000 and the texture record's base).
const MIPGEN_BO: u32 = 0xf000;

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

/// The words of the dump's first `SUBMIT_TFU` line, in v3d_tfu71_job order.
fn mesa_job() -> V3dTfu71Job {
    let line =
        MIPGEN_STDOUT.lines().find(|l| l.contains("SUBMIT_TFU icfg ")).expect("gl_mipgen.stdout has a SUBMIT_TFU line");
    let t: Vec<&str> = line.split_whitespace().collect();
    let word = |name: &str| -> u32 {
        let i = t.iter().position(|w| *w == name).unwrap_or_else(|| panic!("{name} in {line}"));
        hex(t[i + 1])
    };
    let c = t.iter().position(|w| *w == "coef").expect("coef");
    V3dTfu71Job {
        icfg: word("icfg"),
        iia: word("iia"),
        iis: word("iis"),
        ica: word("ica"),
        iua: word("iua"),
        ioa: word("ioa"),
        ios: word("ios"),
        coef: [hex(t[c + 1]), hex(t[c + 2]), hex(t[c + 3]), hex(t[c + 4])],
        ioc: word("ioc"),
    }
}

fn tex(w: u32, h: u32, levels: u32) -> GlTex71 {
    GlTex71 {
        target: Target::Tex2d,
        internal_format: GL_RGBA8,
        ty: GL_UNSIGNED_BYTE,
        width: w,
        height: h,
        levels,
        base_level: 0,
        max_level: GL_MAX_LEVEL_DEFAULT,
        swizzle: GL_SWIZZLE_IDENTITY,
        srgb_decode: true,
        bo_address: MIPGEN_BO,
        ver: 71,
        iprev: 10,
    }
}

/// gl_mipgen's texture: `glTexStorage2D(GL_TEXTURE_2D, 7, GL_RGBA8, 64, 64)`.
fn mipgen() -> GlTex71 {
    tex(64, 64, 7)
}

/// v3d_utile_height: the UIF block is two utiles high.
fn uif_block_rows(cpp: u32) -> u32 {
    2 * match cpp {
        1 => 8,
        2 | 4 => 4,
        _ => 2,
    }
}

// ---- the C packer --------------------------------------------------------------

fn raw<T>(v: &T) -> Vec<u8> {
    // SAFETY: a plain repr(C) struct of u32 fields, read as bytes.
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

/// The harness source, embedded so the test reads nothing from tests/ at run time.
const HARNESS_C: &str = include_str!("v3d71_mip_harness.c");

/// Builds the harness once per test (each test its own directory: the
/// tests of this binary run in parallel); the path of the executable.
fn build_harness(cc: &str, name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d71_mip").join(name);
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

/// Packs every args through `v3d_tfu71_pack`: the job, or None where the
/// packer refused. None for the whole list without a C compiler.
fn c_pack(name: &str, args: &[V3dTfu71Args]) -> Option<Vec<Option<V3dTfu71Job>>> {
    let Some(cc) = find_cc() else {
        println!("skip: no C compiler (set CC or install cc): the C packer half of v3d71_mip::{name} not run");
        return None;
    };
    let exe = build_harness(&cc, name);
    let mut child = Command::new(&exe).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let input: Vec<u8> = args.iter().flat_map(raw).collect();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "harness exit {:?}", out.status.code());
    let rec = 1 + std::mem::size_of::<V3dTfu71Job>();
    assert_eq!(out.stdout.len(), rec * args.len());
    let jobs = out
        .stdout
        .chunks(rec)
        .map(|r| {
            let w: Vec<u32> = r[1..].chunks(4).map(|b| u32::from_ne_bytes(b.try_into().unwrap())).collect();
            (r[0] == 1).then(|| V3dTfu71Job {
                icfg: w[0],
                iia: w[1],
                iis: w[2],
                ica: w[3],
                iua: w[4],
                ioa: w[5],
                ios: w[6],
                coef: [w[7], w[8], w[9], w[10]],
                ioc: w[11],
            })
        })
        .collect();
    println!("packed {} TFU job(s) with {cc}", args.len());
    Some(jobs)
}

/// Every job of the plan packs (the packer agrees the inputs describe the
/// image); the packed jobs.
fn packs(name: &str, p: &MipPlan71) -> Option<Vec<V3dTfu71Job>> {
    let args: Vec<V3dTfu71Args> = p.jobs.iter().map(|j| j.args).collect();
    let packed = c_pack(name, &args)?;
    Some(
        packed
            .into_iter()
            .zip(&args)
            .map(|(j, a)| j.unwrap_or_else(|| panic!("v3d_tfu71_pack refused {a:?}")))
            .collect(),
    )
}

// ---- the tests --------------------------------------------------------------

#[test]
fn mipgen_buffer_is_the_dumps() {
    assert!(
        MIPGEN_CLIF.lines().any(|l| l.ends_with(&format!(" resource_{MIPGEN_BO:#x}"))),
        "gl_mipgen.clif creates no resource_{MIPGEN_BO:#x}"
    );
    // its level 0 is where the texture record (and the TFU) point
    let t = texture71::compose(&mipgen()).unwrap();
    assert_eq!(t.base, mesa_job().iia);
}

/// The plan's inputs, read back out of Mesa's words (v3dx_tfu.c's 7.1 fill,
/// field positions of v3d_tfu.h as ffi71 mirrors them).
#[test]
fn mipgen_inputs_are_mesas() {
    let m = mesa_job();
    let p = plan(&mipgen()).unwrap();
    assert_eq!((p.base_level, p.last_level), (0, 6));
    assert_eq!(p.jobs.len(), 1);
    let (layer, a) = (p.jobs[0].layer, p.jobs[0].args);
    assert_eq!(layer, 0);
    assert_eq!(a.ttype, (m.icfg >> ffi71::V3D_TFU71_ICFG_OTYPE_SHIFT) & 0x7f);
    assert_eq!((a.width, a.height), (m.ios & 0xffff, m.ios >> 16));
    assert_eq!(a.num_mm, (m.ioc >> ffi71::V3D_TFU71_IOC_NUMMM_SHIFT) & 0xf);
    assert_eq!(m.ioc & ffi71::V3D_TFU71_IOC_DIMTW, 1);
    assert_eq!((a.src.addr, a.dst.addr), (m.iia, m.ioa));
    // both sides UIF: the format code is LINEARTILE's plus the distance
    let code = |tiling: u32| ffi71::V3D_TFU71_IOC_FORMAT_LINEARTILE + tiling - ffi71::V3D_TFU71_TILING_LINEARTILE;
    assert_eq!(code(a.src.tiling), m.icfg >> ffi71::V3D_TFU71_ICFG_IFORMAT_SHIFT);
    assert_eq!(code(a.dst.tiling), (m.ioc >> ffi71::V3D_TFU71_IOC_FORMAT_SHIFT) & 0xf);
    assert_eq!(a.dst.tiling, ffi71::V3D_TFU71_TILING_UIF_NO_XOR);
    // padded heights in UIF blocks
    assert_eq!(a.src.stride_or_padded_height / uif_block_rows(a.cpp), m.iis);
    assert_eq!(a.dst.stride_or_padded_height / uif_block_rows(a.cpp), m.ioc >> ffi71::V3D_TFU71_IOC_STRIDE_SHIFT);
    assert_eq!(a.cpp, 4);
}

/// The driver's packer makes Mesa's job, word for word, from the plan.
#[test]
fn mipgen_job_packs_to_mesas_words() {
    let p = plan(&mipgen()).unwrap();
    let Some(jobs) = packs("mipgen", &p) else { return };
    assert_eq!(jobs, [mesa_job()]);
    assert_eq!(jobs[0].ioc, 0x0008_6061);
}

/// GL_TEXTURE_BASE_LEVEL and GL_TEXTURE_MAX_LEVEL: the jobs read the base
/// level and write up to the max level, clamped to the storage.
#[test]
fn base_and_max_level() {
    let mut t = mipgen();
    t.max_level = 3;
    let p = plan(&t).unwrap();
    assert_eq!((p.base_level, p.last_level, p.jobs[0].args.num_mm), (0, 3, 3));

    let mut t = mipgen();
    t.base_level = 2;
    let p = plan(&t).unwrap();
    let a = p.jobs[0].args;
    assert_eq!((p.base_level, p.last_level, a.num_mm), (2, 6, 4));
    assert_eq!((a.width, a.height), (16, 16));
    let l = texture71::layout(64, 64, 7, 4, 1).unwrap();
    assert_eq!(a.src.addr, MIPGEN_BO + l.slices[2].offset);
    assert_eq!(a.dst, a.src);
    // level 2 of a 64x64 RGBA8 tree is UB-linear, 2 columns: no stride field
    assert_eq!(a.dst.tiling, ffi71::V3D_TFU71_TILING_UBLINEAR_2_COLUMN);
    assert_eq!(a.dst.stride_or_padded_height, 0);

    let mut t = tex(256, 128, 9);
    t.base_level = 1;
    t.max_level = 5;
    let p = plan(&t).unwrap();
    let a = p.jobs[0].args;
    assert_eq!((a.width, a.height, a.num_mm), (128, 64, 4));
    assert_eq!(a.dst.tiling, ffi71::V3D_TFU71_TILING_UIF_NO_XOR);
    let l = texture71::layout(256, 128, 9, 4, 1).unwrap();
    assert_eq!(a.dst.stride_or_padded_height, l.slices[1].padded_height);

    let mut all = plan(&mipgen()).unwrap().jobs;
    for (b, m) in [(0, 3), (2, GL_MAX_LEVEL_DEFAULT)] {
        let mut t = mipgen();
        (t.base_level, t.max_level) = (b, m);
        all.extend(plan(&t).unwrap().jobs);
    }
    all.extend(p.jobs);
    let Some(jobs) = packs("base_and_max_level", &MipPlan71 { base_level: 0, last_level: 0, jobs: all.clone() }) else {
        return;
    };
    for (j, m) in jobs.iter().zip(&all) {
        assert_eq!(j.iia, j.ioa);
        assert_eq!((j.ioc >> 4) & 0xf, m.args.num_mm);
        assert_eq!(j.ios, m.args.height << 16 | m.args.width);
    }
}

/// Another type and size: RGB565 (the TFU's type 6), non-square.
#[test]
fn rgb565_non_square() {
    let mut t = tex(64, 16, 7);
    (t.internal_format, t.ty) = (GL_RGB565, GL_UNSIGNED_SHORT_5_6_5);
    let p = plan(&t).unwrap();
    let a = p.jobs[0].args;
    assert_eq!((a.ttype, a.cpp, a.num_mm), (ffi71::V3D_TFU71_TTYPE_RGB565, 2, 6));
    let Some(jobs) = packs("rgb565_non_square", &p) else { return };
    assert_eq!(jobs[0].icfg >> 16 & 0x7f, ffi71::V3D_TFU71_TTYPE_RGB565);
}

/// A cube map: one job per face, each at its face's base level.
#[test]
fn cube_one_job_per_face() {
    let mut t = tex(32, 32, 6);
    t.target = Target::Cube;
    let p = plan(&t).unwrap();
    let l = texture71::layout(32, 32, 6, 4, 6).unwrap();
    assert_eq!(p.jobs.iter().map(|j| j.layer).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5]);
    for j in &p.jobs {
        assert_eq!(j.args.src.addr, MIPGEN_BO + l.offset(0, j.layer).unwrap());
        assert_eq!(j.args.num_mm, 5);
        assert_eq!(j.args.dst, j.args.src);
    }
    let Some(jobs) = packs("cube_one_job_per_face", &p) else { return };
    assert!(jobs.windows(2).all(|w| w[0].ioc == w[1].ioc && w[1].iia - w[0].iia == l.layer_stride));
}

/// Nothing to make: an empty plan, not a refusal.
#[test]
fn nothing_to_generate() {
    let p = plan(&tex(1, 1, 1)).unwrap();
    assert!(p.jobs.is_empty());
    let mut t = mipgen();
    (t.base_level, t.max_level) = (3, 3);
    let p = plan(&t).unwrap();
    assert_eq!((p.base_level, p.last_level), (3, 3));
    assert!(p.jobs.is_empty());
    // the last level of the storage as base
    let mut t = mipgen();
    t.base_level = 6;
    assert!(plan(&t).unwrap().jobs.is_empty());
}

#[test]
fn refusals() {
    // RGBA32F: the TFU copies it but makes no mipmaps of it
    let mut t = mipgen();
    (t.internal_format, t.ty) = (GL_RGBA32F, GL_FLOAT);
    assert_eq!(plan(&t).unwrap_err(), Refusal::Tfu(tfuplan71::Refusal::UnsupportedType));
    // a depth texture: no TFU type
    let mut t = mipgen();
    (t.internal_format, t.ty) = (GL_DEPTH_COMPONENT16, GL_UNSIGNED_SHORT);
    assert_eq!(plan(&t).unwrap_err(), Refusal::Tfu(tfuplan71::Refusal::UnsupportedType));
    // texture71's own: base level above max level, below 7.1.5
    let mut t = mipgen();
    (t.base_level, t.max_level) = (2, 1);
    assert!(matches!(plan(&t).unwrap_err(), Refusal::Texture(texture71::Refusal::BaseAboveMax { .. })));
    let mut t = mipgen();
    t.iprev = 4;
    assert!(matches!(plan(&t).unwrap_err(), Refusal::Texture(texture71::Refusal::State(_))));
    // a buffer address the base level of the last face passes 32 bits at
    let mut t = tex(64, 64, 7);
    t.target = Target::Cube;
    t.bo_address = u32::MAX - 0x1000;
    assert!(matches!(plan(&t).unwrap_err(), Refusal::Texture(texture71::Refusal::AddressOverflow)));
    for r in [
        Refusal::Tfu(tfuplan71::Refusal::UnsupportedType),
        Refusal::AddressOverflow,
        Refusal::Texture(texture71::Refusal::ZeroSize),
    ] {
        assert!(r.to_string().starts_with("mip 7.1: "), "{r}");
    }
}
