// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2-es2 on the Mali compiler: the first refusal of every glmark2
//! shader, per scene, pinned in `tests/fixtures/mali_glmark2_verdicts.txt`
//! so that the per-scene verdicts only move forward.
//!
//! The corpus is the one `tests/glmark2_corpus.rs` reads (GPL-3.0 text, never
//! committed): `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC"` generates it
//! into `build/glmark2-host/shaders/`, or `GLMARK2_CORPUS` names another
//! directory holding `index.txt`. With no corpus at the default place the
//! test prints a SKIP line and returns; a directory `GLMARK2_CORPUS` names
//! must hold the corpus, or the test fails.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_corpus -- --nocapture
//! MALI_GLMARK2_WRITE=1 cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_corpus -- --nocapture
//! ```
//!
//! Each shader is compiled as glCompileShader compiles it; each program an
//! index entry links ([`PROGRAMS`]) whose two stages compile is built as
//! glLinkProgram builds it on the Mali (`src/gl/program.rs`): the vertex
//! shader again for the fragment shader (`compile_vs_linked` with no map),
//! then the link. Per scene, stage and shader the fixture holds `ok`, the
//! compile refusal (`MaliError::message`'s text, or the whole log when the
//! refusal is not the code generator's), or `link <vs>+<fs>: <log>`, the
//! first refusal of a program the shader is in. A shader whose partner is
//! refused is `ok` itself. A scene is compile-clean when every one of its
//! shaders is `ok`: all of its programs compile and link.
//!
//! The test fails only on a regression: a shader the fixture holds `ok` that
//! is refused now, a refusal whose message changed, or a corpus that does not
//! match the fixture's lines. A refusal that became `ok` passes and is
//! printed; the second command above rewrites the fixture.
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
use core::ffi::c_char;
use std::collections::BTreeMap;
use std::path::Path;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

const FIXTURE: &str = "tests/fixtures/mali_glmark2_verdicts.txt";

/// glmark2 22c527cb's default benchmark list (`src/default-benchmarks.h`),
/// in order: the scene numbers of the harness (`FINDINGS-mali-allscenes.md`).
const ENTRIES: [&str; 33] = [
    "build:use-vbo=false",
    "build:use-vbo=true",
    "texture:texture-filter=nearest",
    "texture:texture-filter=linear",
    "texture:texture-filter=mipmap",
    "shading:shading=gouraud",
    "shading:shading=blinn-phong-inf",
    "shading:shading=phong",
    "shading:shading=cel",
    "bump:bump-render=high-poly",
    "bump:bump-render=normals",
    "bump:bump-render=height",
    "effect2d:kernel=0,1,0;1,-4,1;0,1,0;",
    "effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;",
    "pulsar:quads=5:texture=false:light=false",
    "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
    "desktop:windows=4:effect=shadow",
    "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false",
    "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false",
    "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true",
    "ideas:speed=duration",
    "jellyfish",
    "terrain",
    "shadow",
    "refract",
    "conditionals:vertex-steps=0:fragment-steps=0",
    "conditionals:vertex-steps=0:fragment-steps=5",
    "conditionals:vertex-steps=5:fragment-steps=0",
    "function:fragment-steps=5:fragment-complexity=low",
    "function:fragment-steps=5:fragment-complexity=medium",
    "loop:vertex-steps=5:fragment-steps=5:fragment-loop=false",
    "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false",
    "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true",
];

/// The programs each entry links, `<vs>+<fs>` by the first four hex digits
/// of each corpus file's hash, in the order glmark2 links them: read from a
/// PowerVR harness run's logs (`run.sh "$GLMARK2_SRC" build/glmark2-host
/// powervr`, each `logs/powervr/<n>.log`'s `gm2: shader` lines, a vertex
/// shader followed by its fragment shader), as
/// `docs/notes/mali-glmark2-forecast.md` section 1 lists them. The index
/// does not say which vertex shader went with which fragment shader.
const PROGRAMS: [&[&str]; 33] = [
    &["ace2+4748"],
    &["ace2+4748"],
    &["ace2+425b"],
    &["ace2+425b"],
    &["ace2+425b"],
    &["83cd+4748"],
    &["78e3+994f"],
    &["e5f2+8a64"],
    &["e5f2+fea0"],
    &["78e3+d442"],
    &["51d0+487d"],
    &["270a+77c5"],
    &["4623+3c3d"],
    &["4623+ba8e"],
    &["9d75+4748"],
    &["792b+9ce9", "792b+7f17", "792b+68af"],
    &["792b+9ce9"],
    &["7234+9fe6"],
    &["7234+9fe6"],
    &["7234+9fe6"],
    &[
        "bf4b+23c8",
        "c265+23c8",
        "a09e+23c8",
        "038e+23c8",
        "51b2+c565",
        "4f27+e6a9",
        "4f27+df64",
        "51b2+1aca",
        "c236+23c8",
    ],
    &["14ac+8ee5", "08a2+22fc"],
    &[
        "8b69+1637",
        "8b69+c549",
        "8b69+d697",
        "18bc+4d26",
        "8b69+e043",
        "8b69+bdfd",
        "8b69+895a",
        "8b69+bbba",
        "8b69+8c86",
    ],
    &["6964+4748", "37f4+928e", "b9d1+cf4b"],
    &["7215+3fae", "37f4+928e"],
    &["3107+6c61"],
    &["3107+a5b5"],
    &["a21c+6c61"],
    &["0162+c8ce"],
    &["0162+849c"],
    &["3cb0+e016"],
    &["3cb0+c7ea"],
    &["3cb0+3678"],
];

const LOG: usize = 4096;

fn text(log: &[c_char]) -> String {
    let s: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    s.trim().replace('\n', " / ")
}

/// The code generator's message (`MaliError::message`) out of a compile log,
/// or the whole log when the refusal is not the code generator's.
fn message(log: &str) -> String {
    for stage in ["vertex", "fragment"] {
        let pre = format!("online shaderc: the code generator refused this {stage} shader: Mali(");
        if let Some(m) = log.strip_prefix(&pre).and_then(|m| m.strip_suffix(')')) {
            return m.to_string();
        }
    }
    log.to_string()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; LOG];
    let text_z = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, text_z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(message(&text(&log)))
    }
}

/// glLinkProgram's two steps on the Mali: the vertex shader for the fragment
/// shader (`link_vs_for_fs`), then the link.
fn link(vs_src: &str, vs: ShcShader, fs: &mut ShcShader) -> Result<(), String> {
    let mut log = [0 as c_char; LOG];
    let relink = yakogl::shaderc::online::SHC_ONLINE.compile_vs_linked.expect("the online back end relinks");
    let vs_z = format!("{vs_src}\0");
    let mut out = ShcShader::ZERO;
    // SAFETY: a NUL-terminated source, no map, a valid fragment shader, a
    // real output and a log of the size announced.
    let rc = unsafe {
        relink(vs_z.as_ptr() as *const c_char, core::ptr::null(), &*fs, &mut out, log.as_mut_ptr(), log.len())
    };
    let vs = match rc {
        0 => out,
        1 => vs,
        _ => return Err(format!("the vertex shader for the program's fragment shader: {}", text(&log))),
    };
    let f = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    // SAFETY: two distinct valid shaders and a log of the size announced.
    let rc = unsafe { f(&vs, fs, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(text(&log))
    }
}

/// `<scene>-<hash>.{vert,frag}.glsl`: the stage and the hash's first four
/// hex digits.
fn short(file: &str) -> (&'static str, &str) {
    let stage = if file.contains(".vert.") { "vs" } else { "fs" };
    let hash = file.rsplit_once('-').map_or(file, |(_, h)| h);
    (stage, &hash[..4])
}

#[test]
fn mali_glmark2_verdicts_only_move_forward() {
    let named = std::env::var_os("GLMARK2_CORPUS");
    let dir = match &named {
        Some(d) => std::path::PathBuf::from(d),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("build/glmark2-host/shaders"),
    };
    let Ok(index) = std::fs::read_to_string(dir.join("index.txt")) else {
        assert!(
            named.is_none(),
            "GLMARK2_CORPUS={} has no index.txt: run tools/hostharness/glmark2/run.sh \"$GLMARK2_SRC\" first",
            dir.display()
        );
        eprintln!(
            "SKIP mali_glmark2_corpus: no corpus at {} (it is never committed; set GLMARK2_SRC and run tools/hostharness/glmark2/run.sh)",
            dir.display()
        );
        return;
    };
    /* the files, and per scene its vertex and fragment files */
    let mut files = Vec::new();
    let mut by_scene: BTreeMap<usize, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for l in index.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (f, entries) = l.split_once(": ").expect("`<file>: <entries>`");
        files.push(f.to_string());
        for e in entries.split("; ") {
            let n = ENTRIES.iter().position(|x| *x == e).unwrap_or_else(|| panic!("{e}: not a default entry"));
            let slot = by_scene.entry(n).or_default();
            if f.contains(".vert.") {
                slot.0.push(f.to_string());
            } else {
                slot.1.push(f.to_string());
            }
        }
    }
    assert_eq!(files.len(), 69, "index.txt lists 69 shaders; re-derive the pin if it changed");
    assert_eq!(by_scene.len(), ENTRIES.len(), "every default entry has shaders in index.txt");

    let read = |n: &str| std::fs::read_to_string(dir.join(n)).expect("a shader file");
    let stage_of = |n: &str| if n.contains(".vert.") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
    let mut refused: BTreeMap<String, String> = BTreeMap::new();
    for f in &files {
        yakogl::shaderc::online::backend::forget_compiled();
        if let Err(m) = compile(stage_of(f), &read(f)) {
            refused.insert(f.clone(), m);
        }
    }

    /* per scene, its programs whose two stages compile are linked; a shader
     * that compiles takes the first link refusal of a program it is in */
    let mut tried: BTreeMap<(String, String), Result<(), String>> = BTreeMap::new();
    let mut now: BTreeMap<String, String> = BTreeMap::new();
    let mut clean = Vec::new();
    for (n, (vss, fss)) in &by_scene {
        let file = |stage: &str, h: &str| -> String {
            let of: Vec<&String> = if stage == "vs" { vss } else { fss }.iter().filter(|s| short(s).1 == h).collect();
            assert_eq!(of.len(), 1, "scene {n}: one {stage} file with hash {h} in index.txt");
            of[0].clone()
        };
        let mut why: BTreeMap<String, String> = BTreeMap::new();
        let mut used = Vec::new();
        for p in PROGRAMS[*n] {
            let (vh, fh) = p.split_once('+').expect("`<vs>+<fs>`");
            let (v, f) = (file("vs", vh), file("fs", fh));
            used.push(v.clone());
            used.push(f.clone());
            if refused.contains_key(&v) || refused.contains_key(&f) {
                continue;
            }
            let key = (v.clone(), f.clone());
            if !tried.contains_key(&key) {
                yakogl::shaderc::online::backend::forget_compiled();
                let vs_src = read(&v);
                let vs = compile(SHC_STAGE_VERTEX, &vs_src).expect("compiled above");
                let mut fs = compile(SHC_STAGE_FRAGMENT, &read(&f)).expect("compiled above");
                tried.insert(key.clone(), link(&vs_src, vs, &mut fs));
            }
            if let Err(m) = &tried[&key] {
                for s in [&v, &f] {
                    why.entry(s.clone()).or_insert_with(|| format!("link {p}: {m}"));
                }
            }
        }
        for s in vss.iter().chain(fss) {
            assert!(used.contains(s), "scene {n}: {s} is in no program of PROGRAMS; re-derive the pin");
        }
        let mut lines: Vec<(String, String)> = Vec::new();
        for s in vss.iter().chain(fss) {
            let (stage, h) = short(s);
            let verdict = refused.get(s).or(why.get(s)).cloned().unwrap_or_else(|| "ok".to_string());
            lines.push((format!("{n} {} {stage} {h}", ENTRIES[*n]), verdict));
        }
        if lines.iter().all(|(_, v)| v == "ok") {
            clean.push(*n);
        }
        for (k, v) in lines {
            assert!(now.insert(k.clone(), v).is_none(), "{k}: two shaders of one scene share four hex digits");
        }
    }

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    let render = |m: &BTreeMap<String, String>| {
        let mut keys: Vec<&String> = m.keys().collect();
        keys.sort_by_key(|k| (k.split(' ').next().unwrap().parse::<usize>().unwrap(), k.to_string()));
        let mut out = String::from(
            "# tests/mali_glmark2_corpus.rs: the Mali compiler's first refusal per glmark2 scene and shader\n\
             # (glmark2 22c527cb). Regenerate with MALI_GLMARK2_WRITE=1 (the test's header).\n",
        );
        for k in keys {
            out.push_str(&format!("{k}: {}\n", m[k]));
        }
        out
    };
    println!(
        "mali: {} shaders, {} compiled; {} pairs tried, {} linked; {} of {} scenes compile-clean: {clean:?}",
        files.len(),
        files.len() - refused.len(),
        tried.len(),
        tried.values().filter(|r| r.is_ok()).count(),
        clean.len(),
        ENTRIES.len()
    );
    if std::env::var_os("MALI_GLMARK2_WRITE").is_some_and(|v| v == "1") {
        std::fs::write(&path, render(&now)).expect("write the fixture");
        println!("wrote {FIXTURE}");
        return;
    }

    let pinned_text = std::fs::read_to_string(&path).expect("the fixture (MALI_GLMARK2_WRITE=1 writes it)");
    let mut pinned: BTreeMap<String, String> = BTreeMap::new();
    for l in pinned_text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (k, v) = l.split_once(": ").expect("`<scene#> <scene> <vs|fs> <hash4>: <verdict>`");
        pinned.insert(k.to_string(), v.to_string());
    }
    let mut regressed = Vec::new();
    let mut forward = Vec::new();
    for (k, v) in &now {
        match pinned.get(k) {
            None => regressed.push(format!("{k}: not in the fixture (now `{v}`)")),
            Some(p) if p == v => {}
            Some(_) if v == "ok" => forward.push(k.clone()),
            Some(p) => regressed.push(format!("{k}: was `{p}`, now `{v}`")),
        }
    }
    for k in pinned.keys().filter(|k| !now.contains_key(*k)) {
        regressed.push(format!("{k}: in the fixture, not in the corpus"));
    }
    for k in &forward {
        println!("  now ok: {k} (rewrite the fixture with MALI_GLMARK2_WRITE=1)");
    }
    assert!(
        regressed.is_empty(),
        "mali: verdicts that moved back or changed without the fixture ({FIXTURE}):\n{}",
        regressed.join("\n")
    );
}
