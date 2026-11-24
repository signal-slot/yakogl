// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 1.00 programs at the GLES2 implementation limits through the 4.2
//! and 7.1 (Raspberry Pi 5) online back ends, pinned (the lim71 study).
//!
//! The sources are generated here, no corpus is read. Each limit shape is
//! built at its limit and one step below it: varyings (all live at once),
//! vertex attributes, vertex and fragment uniform vectors, texture units, and
//! a large uniform array indexed by a variable.
//!
//! For every program 4.2 must compile and link it (`links_at_4_2`). At 7.1
//! (`chain71::compile_program71`) it either compiles, and then every word of
//! its FS, VS and CS decodes with codec71 and the instructions pass verify71,
//! `registers_fit_threads71` and the branch check, or it is in
//! `EXPECTED_REFUSED_71` with the refusal text it gives. The list cannot go
//! stale: the test fails when a listed program compiles, when it refuses with
//! another text, and when an unlisted program is refused. Totals are pinned
//! (`PROGRAMS`, `COMPILED_42`, `COMPILED_71`).
//!
//! tools/check.sh's "every tests/ target" row runs this with its feature set.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_limits -- --nocapture
//! ```
#![cfg(all(feature = "gpu-v3d", feature = "shaderc-online"))]

use core::ffi::c_char;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, same_instr71};
use yakogl::shaderc::online::v3d::sched71_cf::branches_are_structured71;
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// Programs generated.
const PROGRAMS: usize = 19;
/// Programs 4.2 compiles and links (all of them).
const COMPILED_42: usize = 19;
/// Programs 7.1 compiles (the rest are in `EXPECTED_REFUSED_71`).
const COMPILED_71: usize = 17;

/// 7.1-only refusals: (program name, exact refusal text, reason).
///
/// Each program here compiles at 4.2. Every entry is checked in both
/// directions (see the header), so the list cannot go stale.
const EXPECTED_REFUSED_71: &[(&str, &str, &str)] = &[
    // The 7.1 register allocator (alloc71.rs, the `DoesNotFit` return near
    // :326; chain71.rs:230-232 turns it into the link error) cannot fit an FS
    // that keeps this many varyings live at once. 4.2 compiles it.
    ("varyings15", "Alloc(DoesNotFit)", "15 varyings all live in the FS: 7.1 allocator does not fit"),
    // The same refusal one step below the limit: 14 vec4 varyings all live.
    ("varyings14", "Alloc(DoesNotFit)", "14 varyings all live in the FS: 7.1 allocator does not fit"),
];

const VS_PLAIN: &str = "attribute vec4 p; void main() { gl_Position = p; }";
const FS_PLAIN: &str = "precision mediump float; void main() { gl_FragColor = vec4(1.0); }";

struct Prog {
    name: String,
    vs: String,
    fs: String,
}

fn prog(name: String, vs: String, fs: String) -> Prog {
    Prog { name, vs, fs }
}

/// `n` varyings, written by the VS from a uniform and the position and all
/// summed by the FS, so all are live at once.
fn varyings(n: usize) -> Prog {
    let (mut vd, mut vb, mut fd, mut fb) = (String::new(), String::new(), String::new(), String::new());
    for i in 0..n {
        vd.push_str(&format!("varying vec4 v{i};\n"));
        vb.push_str(&format!("v{i} = u * {}.0 + p;\n", i + 1));
        fd.push_str(&format!("varying vec4 v{i};\n"));
        fb.push_str(&format!("+ v{i}"));
    }
    prog(
        format!("varyings{n}"),
        format!("{vd}uniform vec4 u; attribute vec4 p; void main() {{ {vb} gl_Position = p; }}"),
        format!("precision mediump float;\n{fd}void main() {{ gl_FragColor = vec4(0.0) {fb}; }}"),
    )
}

/// `n` vec4 attributes, all summed.
fn attributes(n: usize) -> Prog {
    let (mut d, mut s) = (String::new(), String::new());
    for i in 0..n {
        d.push_str(&format!("attribute vec4 a{i};\n"));
        s.push_str(&format!("+ a{i}"));
    }
    prog(format!("attributes{n}"), format!("{d}void main() {{ gl_Position = vec4(0.0) {s}; }}"), FS_PLAIN.into())
}

/// `n` vec4 uniforms in one stage, each read with a constant index.
fn uniforms(n: usize, vertex: bool) -> Prog {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("+ u[{i}]"));
    }
    if vertex {
        prog(
            format!("vs-uniforms{n}"),
            format!("uniform vec4 u[{n}]; attribute vec4 p; void main() {{ gl_Position = p {s}; }}"),
            FS_PLAIN.into(),
        )
    } else {
        prog(
            format!("fs-uniforms{n}"),
            VS_PLAIN.into(),
            format!("precision mediump float; uniform vec4 u[{n}]; void main() {{ gl_FragColor = vec4(0.0) {s}; }}"),
        )
    }
}

/// `n` samplers, each sampled once, in the fragment (or vertex) shader.
fn textures(n: usize, vertex: bool) -> Prog {
    let (mut d, mut s) = (String::new(), String::new());
    for i in 0..n {
        d.push_str(&format!("uniform sampler2D s{i};\n"));
        if vertex {
            s.push_str(&format!("+ texture2DLod(s{i}, p.xy, 0.0)"));
        } else {
            s.push_str(&format!("+ texture2D(s{i}, v)"));
        }
    }
    if vertex {
        prog(
            format!("vs-textures{n}"),
            format!("{d}attribute vec4 p; void main() {{ gl_Position = vec4(0.0) {s}; }}"),
            FS_PLAIN.into(),
        )
    } else {
        prog(
            format!("fs-textures{n}"),
            "attribute vec4 p; varying vec2 v; void main() { v = p.xy; gl_Position = p; }".into(),
            format!("precision mediump float;\n{d}varying vec2 v; void main() {{ gl_FragColor = vec4(0.0) {s}; }}"),
        )
    }
}

/// A `vec4 u[n]` array indexed by a variable.
fn dyn_array(n: usize, vertex: bool) -> Prog {
    if vertex {
        prog(
            format!("vs-dynarray{n}"),
            format!("uniform vec4 u[{n}]; attribute vec4 p; void main() {{ gl_Position = u[int(p.x)]; }}"),
            FS_PLAIN.into(),
        )
    } else {
        prog(
            format!("fs-dynarray{n}"),
            "attribute vec4 p; varying float i; void main() { i = p.x; gl_Position = p; }".into(),
            format!(
                "precision mediump float; uniform vec4 u[{n}]; varying float i; void main() {{ gl_FragColor = u[int(i)]; }}"
            ),
        )
    }
}

fn programs() -> Vec<Prog> {
    let mut v = Vec::new();
    for n in [8, 14, 15] {
        v.push(varyings(n));
    }
    for n in [15, 16] {
        v.push(attributes(n));
    }
    for n in [127, 254, 255] {
        v.push(uniforms(n, true));
        v.push(uniforms(n, false));
    }
    for n in [7, 8] {
        v.push(textures(n, false));
        v.push(textures(n, true));
    }
    for n in [63, 64] {
        v.push(dyn_array(n, true));
        v.push(dyn_array(n, false));
    }
    v
}

fn links_at_4_2(vsrc: &str, fsrc: &str) -> bool {
    let mut vs = ShcShader::ZERO;
    let mut fs = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let c = yakogl::shaderc::online::SHC_ONLINE.compile.expect("compiles");
    let l = yakogl::shaderc::online::SHC_ONLINE.link.expect("links");
    yakogl::shaderc::online::backend::forget_compiled();
    let (zv, zf) = (format!("{vsrc}\0"), format!("{fsrc}\0"));
    // SAFETY: NUL-terminated sources, real outputs, logs of the size announced.
    unsafe {
        c(SHC_STAGE_VERTEX, zv.as_ptr() as *const c_char, &mut vs, log.as_mut_ptr(), log.len()) == 0
            && c(SHC_STAGE_FRAGMENT, zf.as_ptr() as *const c_char, &mut fs, log.as_mut_ptr(), log.len()) == 0
            && l(&vs, &mut fs, log.as_mut_ptr(), log.len()) == 0
    }
}

fn check_stage(what: &str, c: &Code71<'_>, threads: u32, single_seg: bool) {
    assert_eq!(c.words.len(), c.instrs.len(), "{what}");
    assert!(!c.words.is_empty(), "{what}: no words");
    for (n, (&w, i)) in c.words.iter().zip(c.instrs).enumerate() {
        let back = decode71(w).unwrap_or_else(|e| panic!("{what}: word {n} ({w:#018x}) does not decode: {e:?}"));
        assert!(same_instr71(i, &back), "{what}: word {n} decodes to another instruction");
    }
    assert_eq!(verify71(c.instrs, single_seg), Ok(()), "{what}: verify71");
    assert_eq!(registers_fit_threads71(c.instrs, threads), Ok(()), "{what}: registers at {threads} threads");
    assert_eq!(branches_are_structured71(c.instrs, c.back_edges, c.fwd_branches), Ok(()), "{what}: branches");
}

fn compile71(p: &Prog) -> Result<(), String> {
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, src) in [(Stage::Vertex, &p.vs), (Stage::Fragment, &p.fs)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => m.push(l.module),
                Err(e) => return Err(format!("front end: {}", e.msg)),
            }
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        let r = compile_program71(v, f, Prim::Triangles, TARGET71, a).map_err(|e| format!("{e:?}"))?;
        let n = &p.name;
        check_stage(&format!("{n} fs"), &r.fs.code, r.fs.threads, r.fs.single_seg);
        check_stage(&format!("{n} vs"), &r.vs.code, r.vs.threads, r.vs.single_seg);
        check_stage(&format!("{n} cs"), &r.cs.code, r.cs.threads, r.cs.single_seg);
        Ok(())
    })
    .expect("scratch")
}

#[test]
fn limit_shapes_compile_at_4_2_and_at_7_1_or_are_listed() {
    let progs = programs();
    let (mut c42, mut c71) = (0, 0);
    let mut bad = Vec::new();
    for p in &progs {
        let ok42 = links_at_4_2(&p.vs, &p.fs);
        println!("{}: 4.2 {}", p.name, if ok42 { "ok" } else { "REFUSED" });
        if ok42 {
            c42 += 1;
        } else {
            bad.push(format!("{}: 4.2 refuses", p.name));
        }
        let listed = EXPECTED_REFUSED_71.iter().find(|e| e.0 == p.name);
        let r = compile71(p);
        println!("{}: 7.1 {r:?}", p.name);
        match (r, listed) {
            (Ok(()), None) => c71 += 1,
            (Ok(()), Some(_)) => bad.push(format!("{}: listed in EXPECTED_REFUSED_71 but compiles", p.name)),
            (Err(m), Some(e)) if m == e.1 => {}
            (Err(m), Some(e)) => bad.push(format!("{}: refusal changed: {m:?} (listed {:?})", p.name, e.1)),
            (Err(m), None) => bad.push(format!("{}: 7.1 refuses, unlisted: {m:?}", p.name)),
        }
    }
    println!("programs {} 4.2 {c42} 7.1 {c71}", progs.len());
    for e in EXPECTED_REFUSED_71 {
        assert!(progs.iter().any(|p| p.name == e.0), "{}: listed but not generated", e.0);
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert_eq!(progs.len(), PROGRAMS);
    assert_eq!(c42, COMPILED_42);
    assert_eq!(c71, COMPILED_71);
}
