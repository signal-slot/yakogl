// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/texfill71.rs` (GL texture and sampler state to the `ffi71`
//! `V3dTex71State` and `V3dTex71Sampler`), host test against Mesa's 7.1
//! references.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_texfill
//! ```
//!
//! For every job of tools/v3d/pi5/gl_tex2d, gl_texfmt and gl_cube this reads
//! the texture record and the sampler record Mesa's fragment shader uses out of
//! the `.clif` dump (the reading rule of tools/v3d/pi5/src/texstate71.py),
//! fills the structs with `texfill71::fill` from the case's GL calls
//! (tools/v3d/pi5/README.md, "The GL calls"), and
//!  - compares the sampler through `sampler71::pack` (pure Rust);
//!  - where a C compiler exists (`$CC` or `cc`), builds
//!    tests/v3d71_texfill_harness.c with the driver's v3d_tex71.c, sends it the
//!    raw bytes of both structs, and compares the 24-byte texture and sampler
//!    records it packs with the dump's words. Without a compiler that half
//!    prints a skip line and passes.
//!
//! Only the buffer address comes from the dump (its `@createbuf` names).

#[path = "../src/v3d/ffi71.rs"]
#[allow(dead_code)]
mod ffi71;
#[path = "../src/v3d/format71.rs"]
#[allow(dead_code)]
mod format71;
#[path = "../src/v3d/sampler71.rs"]
#[allow(dead_code)]
mod sampler71;
#[path = "../src/v3d/texfill71.rs"]
mod texfill71;
#[path = "../src/v3d/texstate71.rs"]
#[allow(dead_code)]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
#[allow(dead_code)]
mod texture71;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use sampler71::{Filter, Input, MipFilter, Wrap};
use texture71::{GlTex71, Target, GL_MAX_LEVEL_DEFAULT, GL_SWIZZLE_IDENTITY};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

// ---- the dump: texstate71.py's reading rule ----------------------------------

type Rec = ([u32; 6], [u32; 6]);

struct Dump {
    /// per job: (texture words, sampler words) its fragment shaders use
    jobs: Vec<Vec<Rec>>,
    /// every `resource_0x...` buffer: start address and dumped extent
    resources: Vec<(u32, u32)>,
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
    let finish = |mem: &BTreeMap<u32, u32>, fs: &[u32], unif: &[u32]| -> Vec<Rec> {
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
                let words = |b: u32| -> [u32; 6] { std::array::from_fn(|k| mem[&(b + 4 * k as u32)]) };
                recs.push((words(ta), words(sa)));
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
    Dump { jobs, resources }
}

fn revision(shape: &str) -> (u32, u32) {
    let out = read(&format!("tools/v3d/pi5/{shape}.stdout"));
    let r = out.lines().find_map(|l| l.strip_prefix("gl_shapes: renderer V3D ")).expect("renderer line");
    let n: Vec<u32> = r.trim().split('.').map(|x| x.parse().unwrap()).collect();
    (n[0] * 10 + n[1], n[2])
}

// ---- the cases' GL calls (tools/v3d/pi5/README.md) -----------------------------

const GL_RGBA: u32 = 0x1908;
const GL_RGBA8: u32 = 0x8058;
const GL_SRGB8_ALPHA8: u32 = 0x8C43;
const GL_RGB565: u32 = 0x8D62;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_UNSIGNED_SHORT_5_6_5: u32 = 0x8363;

fn gl(target: Target, internal_format: u32, ty: u32, size: u32) -> GlTex71 {
    GlTex71 {
        target,
        internal_format,
        ty,
        width: size,
        height: size,
        levels: 1,
        base_level: 0,
        max_level: GL_MAX_LEVEL_DEFAULT,
        swizzle: GL_SWIZZLE_IDENTITY,
        srgb_decode: true,
        bo_address: 0,
        ver: 71,
        iprev: 10,
    }
}

/// The GL texture and sampler state of one job, as gl_shapes.py sets them.
/// A cube leaves wrap R at GL's REPEAT: the fill forces CLAMP as Mesa does.
fn case(shape: &str, job: usize) -> (GlTex71, Input) {
    let mut smp = Input::default_2d();
    let filter = |f: Filter, s: &mut Input| {
        s.min_filter = f;
        s.mag_filter = f;
    };
    let tex = match shape {
        "gl_tex2d" => {
            filter(if job == 1 { Filter::Nearest } else { Filter::Linear }, &mut smp);
            gl(Target::Tex2d, GL_RGBA8, GL_UNSIGNED_BYTE, 64)
        }
        "gl_texfmt" => {
            filter(Filter::Nearest, &mut smp);
            if job == 1 {
                gl(Target::Tex2d, GL_SRGB8_ALPHA8, GL_UNSIGNED_BYTE, 64)
            } else {
                gl(Target::Tex2d, GL_RGB565, GL_UNSIGNED_SHORT_5_6_5, 64)
            }
        }
        "gl_cube" => {
            // odd jobs NEAREST, even ones LINEAR; s and t CLAMP_TO_EDGE
            filter(if job % 2 == 1 { Filter::Nearest } else { Filter::Linear }, &mut smp);
            smp.wrap_s = Wrap::ClampToEdge;
            smp.wrap_t = Wrap::ClampToEdge;
            smp.mip_filter = MipFilter::None;
            gl(Target::Cube, GL_RGBA, GL_UNSIGNED_BYTE, 4)
        }
        _ => panic!("no case for {shape}"),
    };
    (tex, smp)
}

const SHAPES: [(&str, usize); 3] = [("gl_tex2d", 2), ("gl_texfmt", 2), ("gl_cube", 12)];

fn bytes(w: &[u32; 6]) -> [u8; 24] {
    let mut b = [0u8; 24];
    for (i, w) in w.iter().enumerate() {
        b[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    b
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

/// Builds the harness; the path of the executable.
fn build_harness(cc: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d71_texfill");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("harness");
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
        .arg(format!("{ROOT}/tests/v3d71_texfill_harness.c"))
        .arg(format!("{ROOT}/driver/src/v3d/v3d_tex71.c"))
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .output()
        .expect("run the C compiler");
    assert!(out.status.success(), "harness build failed:\n{}", String::from_utf8_lossy(&out.stderr));
    exe
}

/// Packs both structs through the C packer: (texture record, sampler record).
fn c_pack(exe: &PathBuf, f: &texfill71::Filled71) -> Result<([u8; 24], [u8; 24]), i32> {
    let mut child = Command::new(exe).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut input = raw(&f.state);
    input.extend(raw(&f.sampler));
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().unwrap();
    if !out.status.success() {
        return Err(out.status.code().unwrap_or(-1));
    }
    assert_eq!(out.stdout.len(), 48);
    Ok((out.stdout[..24].try_into().unwrap(), out.stdout[24..].try_into().unwrap()))
}

struct Job {
    name: String,
    filled: texfill71::Filled71,
    tex_words: [u32; 6],
    smp_words: [u32; 6],
}

/// Every job of the three shapes, filled (the base address from the dump).
fn filled_jobs() -> Vec<Job> {
    let mut v = Vec::new();
    for (shape, njobs) in SHAPES {
        let dump = parse_dump(&format!("tools/v3d/pi5/{shape}.clif"));
        let (ver, iprev) = revision(shape);
        assert_eq!((ver, iprev), (71, 10));
        assert_eq!(dump.jobs.len(), njobs, "{shape}: jobs");
        for (j, recs) in dump.jobs.iter().enumerate() {
            let job = j + 1;
            assert_eq!(recs.len(), 1, "{shape} job {job}: one texture and sampler");
            let (tw, sw) = recs[0];
            let base = tw[0] & !63;
            let &(bo, _) =
                dump.resources.iter().filter(|(a, _)| *a <= base).max_by_key(|(a, _)| *a).expect("a resource");
            let (mut tex, smp) = case(shape, job);
            tex.bo_address = bo;
            let filled = texfill71::fill(&tex, &smp).unwrap_or_else(|e| panic!("{shape} job {job}: {e}"));
            v.push(Job { name: format!("{shape} job {job}"), filled, tex_words: tw, smp_words: sw });
        }
    }
    v
}

#[test]
fn struct_fields_follow_the_composition() {
    for j in filled_jobs() {
        let (name, f) = (&j.name, &j.filled);
        let t = &f.tex;
        assert_eq!(f.state.type_, t.tex_type, "{name}");
        assert_eq!(f.state.swizzle, t.swizzle, "{name}");
        assert_eq!(f.state.base, t.base, "{name}");
        assert_eq!(f.state.depth, if name.starts_with("gl_cube") { 6 } else { 1 }, "{name}");
        assert_eq!(f.state.level_0_is_strictly_uif, f.state.extended, "{name}");
        if name.starts_with("gl_cube") {
            assert_eq!(f.state.array_stride_64 * 64, t.cube_face_stride, "{name}");
            assert_eq!((f.sampler.wrap_s, f.sampler.wrap_t, f.sampler.wrap_r), (1, 1, 1), "{name}: cube wraps");
        }
    }
}

/// The sampler71 value behind the ffi struct, field for field.
fn sampler71_of(f: &texfill71::Filled71) -> sampler71::Sampler71 {
    let s = &f.sampler;
    sampler71::Sampler71 {
        wrap_s: s.wrap_s,
        wrap_t: s.wrap_t,
        wrap_r: s.wrap_r,
        min_nearest: s.min_nearest,
        mag_nearest: s.mag_nearest,
        mip_nearest: s.mip_nearest,
        mip_none: s.mip_none,
        min_lod: s.min_lod,
        max_lod: s.max_lod,
        lod_bias: s.lod_bias,
        compare_func: s.compare_func,
        max_anisotropy: s.max_anisotropy,
        border_color_mode: s.border_color_mode,
        border_color: s.border_color,
    }
}

#[test]
fn samplers_equal_mesa_through_the_rust_packer() {
    let jobs = filled_jobs();
    for j in &jobs {
        let s = sampler71::pack(&sampler71_of(&j.filled)).unwrap_or_else(|| panic!("{}: refused", j.name));
        assert_eq!(s, bytes(&j.smp_words), "{}: sampler record", j.name);
    }
    assert_eq!(jobs.len(), 16);
}

#[test]
fn records_packed_by_the_c_packer_equal_mesa() {
    let Some(cc) = find_cc() else {
        println!("skip: no C compiler (set CC or install cc): the C packer half of v3d71_texfill not run");
        return;
    };
    let exe = build_harness(&cc);
    let jobs = filled_jobs();
    for j in &jobs {
        let (t, s) = c_pack(&exe, &j.filled).unwrap_or_else(|c| panic!("{}: the C packer exit {c}", j.name));
        assert_eq!(t, bytes(&j.tex_words), "{}: texture record", j.name);
        assert_eq!(s, bytes(&j.smp_words), "{}: sampler record", j.name);
    }
    assert_eq!(jobs.len(), 16);
    println!("packed and compared {} texture and sampler records with {cc}", jobs.len());
}

#[test]
fn refusals_are_named_and_passed_through() {
    let (mut tex, mut smp) = case("gl_cube", 1);
    smp.seamless = false;
    assert_eq!(
        texfill71::fill(&tex, &smp).unwrap_err(),
        texfill71::Refusal::Sampler(sampler71::Refusal::CubeNonSeamless)
    );
    smp.seamless = true;
    // a 2D texture needs no cube rule: its own wraps stay, and the target
    // decides `cube`, not the caller
    let (t2, mut s2) = case("gl_tex2d", 1);
    s2.wrap_s = Wrap::MirroredRepeat;
    s2.cube = true;
    assert_eq!(texfill71::fill(&t2, &s2).unwrap().sampler.wrap_s, sampler71::WRAP_MIRROR);
    tex.width = 0;
    assert!(matches!(
        texfill71::fill(&tex, &smp).unwrap_err(),
        texfill71::Refusal::Texture(texture71::Refusal::ZeroSize)
    ));
}
