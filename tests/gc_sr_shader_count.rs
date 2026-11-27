// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 shadow's and refract's eight application shaders, and the
//! one-statement shaders that isolate the causes of their length, through the
//! GC7000 online compiler: the instruction and temporary counts pinned
//! (`docs/notes/gc-shadow-refract-shaders.md`, plan #328 item
//! sr-compiler-fix). Mesa etnaviv's `etna_shc` counts for the GC7000UL are
//! the comments; they are the target, not a pin. The pins include the note's
//! "The patch" (a copy fold and a scalar lane merge,
//! `src/shaderc/online/vivante/peephole.rs`) and rank 3 of
//! `docs/notes/gc-compiler-ranks-3-5.md` (a `mul` read once by an `add` is a
//! `mad`, `select.rs` `fuse_mul_add`); the comments give the count before
//! them.
//!
//! The glmark2 texts are GPL-3.0 and never committed: they are read from the
//! host harness's corpus (`GLMARK2_CORPUS`, by default
//! `build/glmark2-host/shaders`, which `tools/hostharness/glmark2/run.sh
//! "$GLMARK2_SRC"` writes), found by the text's hash whatever scene first
//! compiled it. With no corpus at the default place the glmark2 half prints a
//! SKIP line; a `GLMARK2_CORPUS` that lacks a file fails. The micro-shaders
//! are this repository's own and always run.
//!
//! The four fragment programs are also held against W6's fallback rule
//! (`tiled_rt::long_program` at `LONG_PROGRAM_INSTRS`): from the pins, always,
//! and from the compiler's output over the corpus, under the same SKIP rule.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_sr_shader_count -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use std::path::Path;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// Dwords of shader code and temporaries of one compile.
fn compile(stage: u32, src: &str) -> (u32, u32) {
    // The online back end keeps one compile state for the process, and this
    // file's tests run on threads of their own: one compile at a time.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let (mut sh, mut log) = (ShcShader::ZERO, [0 as c_char; 1024]);
    let text = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, text.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) };
    let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "compile failed: {msg}");
    (sh.usc_dw, sh.temps.into())
}

/// Instructions and temporaries of one compile.
fn count(stage: u32, src: &str) -> (u32, u32) {
    let (dw, temps) = compile(stage, src);
    (dw / 4, temps)
}

/// The corpus directory and its file names; `None` (after a SKIP line naming
/// `what`) when `GLMARK2_CORPUS` is unset and the default place holds none.
fn corpus(what: &str) -> Option<(std::path::PathBuf, Vec<String>)> {
    let named = std::env::var_os("GLMARK2_CORPUS");
    let dir = match &named {
        Some(d) => std::path::PathBuf::from(d),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("build/glmark2-host/shaders"),
    };
    let files: Vec<String> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    if files.is_empty() && named.is_none() {
        eprintln!(
            "SKIP gc_sr_shader_count {what}: no corpus at {} (set GLMARK2_SRC and run tools/hostharness/glmark2/run.sh)",
            dir.display()
        );
        return None;
    }
    Some((dir, files))
}

/// The text of the corpus file whose name ends in `-<hash>.<ext>.glsl`, with its name.
fn corpus_text(dir: &Path, files: &[String], hash: &str, ext: &str) -> (String, String) {
    let suffix = format!("-{hash}.{ext}.glsl");
    let f = files.iter().find(|f| f.ends_with(&suffix)).unwrap_or_else(|| panic!("{} has no *{suffix}", dir.display()));
    (f.clone(), std::fs::read_to_string(dir.join(f)).expect("a shader file"))
}

/// (text hash, stage, ours: instructions and temps, etna_shc's in the comment)
const GLMARK2: [(&str, &str, u32, u32); 8] = [
    ("37f46ae8961221f2", "vert", 4, 3), // depth vertex; etna 4, 3; before 6, 3; 5, 3 before rank 4c
    ("928e172a68b7f175", "frag", 3, 3), // depth fragment; etna 2, 2; before 3, 3
    ("b9d18073e5979540", "vert", 9, 4), // ground vertex; etna 7, 4; before 12, 4; 11, 5 before rank 4c
    ("cf4bf899d84af04d", "frag", 13, 5), // ground fragment; etna 11, 4; before 16, 5
    ("6964fcf6aaac9739", "vert", 19, 5), // horse vertex; etna 14, 4; before 23, 5; 21, 5 before rank 4c
    ("474800a67afed087", "frag", 1, 2), // horse fragment; etna 1, 2; before 1, 2
    ("7215f4f5052a383c", "vert", 19, 5), // refract vertex; etna 19, 5; before 23, 5; 21, 5 before rank 4c
    ("3fae89c9ab969ea3", "frag", 55, 11), // refract fragment; etna 48, 6; before 86, 12; 71, 12 before rank 4, 64 before rank 3
];
/// The eight's instructions: etna_shc 105; before the patch 170; 146 before
/// rank 4 (float constant folding, `docs/notes/gc-compiler-ranks-3-5.md`),
/// 139 before rank 4c (a constructor's constant lane read as the constant),
/// 132 before rank 3 (a `mul` read once by an `add` is a `mad`).
const GLMARK2_TOTAL: u32 = 123;

const HEAD: &str = "precision mediump float;\nvarying vec4 a;\nvarying vec4 b;\n";

/// The note's micro-shaders (fragment stage unless named), the counts ours.
fn micro() -> Vec<(&'static str, u32, String, u32)> {
    let fs = |body: &str| format!("{HEAD}void main() {{ {body} }}\n");
    vec![
        // m1: etna 2 (one rcp, one mul); before the patch 5
        ("m1", SHC_STAGE_FRAGMENT, fs("gl_FragColor = a / a.w;"), 2),
        // m1b: etna 3; before the patch 6
        ("m1b", SHC_STAGE_FRAGMENT, fs("gl_FragColor = vec4(a.xyz / a.w, 1.0);"), 4),
        // m13: etna 3 (mad); before the patch 6 (the patch has no contraction), 6 before rank 3
        ("m13", SHC_STAGE_FRAGMENT, fs("vec3 t = a.xyz * b.x + b.xyz; gl_FragColor = vec4(t * 0.5 + 0.5, 1.0);"), 4),
        // m9: etna 7; before the patch 17; 10 before rank 4 (`L.xyz / L.w` is a constant now), 8 before rank 3
        (
            "m9",
            SHC_STAGE_FRAGMENT,
            fs("const vec4 L = vec4(1.0, 2.0, 3.0, 2.0); gl_FragColor = vec4(normalize(a.xyz / a.w - L.xyz / L.w), 1.0);"),
            7,
        ),
        // m5: etna 10; before the patch 20; 17 before rank 4 (`1.0 - eta * eta` folds), 16 before rank 3
        ("m5", SHC_STAGE_FRAGMENT, fs("gl_FragColor = vec4(refract(a.xyz, b.xyz, 1.2), 1.0);"), 15),
        // m8: etna 4; before the patch 8
        ("m8", SHC_STAGE_FRAGMENT, fs("float k = dot(a.xyz, b.xyz); gl_FragColor = vec4(vec3(sqrt(1.0 - k * k)), 1.0);"), 7),
        // m4 (vertex): etna 4; before the patch 6; 5 before rank 4c (the `1.0` lane read as the constant)
        (
            "m4",
            SHC_STAGE_VERTEX,
            "attribute vec3 position;\nuniform mat4 M;\nvoid main() { gl_Position = M * vec4(position, 1.0); }\n".into(),
            4,
        ),
        // the difference of two varyings: one add with a negated source; before
        // the patch 2 (`mov t, -b` and the add)
        ("neg", SHC_STAGE_FRAGMENT, fs("gl_FragColor = a - b;"), 1),
    ]
}

#[test]
fn shadow_and_refract_shaders_count() {
    let mut wrong = Vec::new();
    for (name, stage, src, want) in micro() {
        let (n, t) = count(stage, &src);
        println!("gc7000 micro {name}: {n} instructions, {t} temps (pinned {want})");
        if n != want {
            wrong.push(format!("{name}: {n} instructions, pinned {want}"));
        }
    }

    if let Some((dir, files)) = corpus("glmark2 half") {
        let mut total = 0;
        for (hash, ext, want_n, want_t) in GLMARK2 {
            let (f, src) = corpus_text(&dir, &files, hash, ext);
            let stage = if ext == "vert" { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
            let (n, t) = count(stage, &src);
            println!("gc7000 {f}: {n} instructions, {t} temps (pinned {want_n}, {want_t})");
            if (n, t) != (want_n, want_t) {
                wrong.push(format!("{f}: {n} instructions {t} temps, pinned {want_n} {want_t}"));
            }
            total += n;
        }
        println!("gc7000 shadow and refract: {total} instructions in the eight (pinned {GLMARK2_TOTAL}; etna_shc 105)");
        if total != GLMARK2_TOTAL {
            wrong.push(format!("total {total}, pinned {GLMARK2_TOTAL}"));
        }
    }
    assert!(wrong.is_empty(), "counts moved; re-pin with the note's table:\n{}", wrong.join("\n"));
}

/// The four fragment programs of [`GLMARK2`] against W6's fallback rule
/// (`tiled_rt::long_program` at `LONG_PROGRAM_INSTRS`, 64 instructions,
/// inclusive; `docs/notes/gc-tiled-wire.md` W6): (name, text hash, verdict).
/// None qualifies. Refract's fragment program was 64 instructions, exactly on
/// the threshold, before rank 3 (a `mul` read once by an `add` is a `mad`) and
/// is 55 since, so the narrow rule no longer takes refract's pass; shadow's
/// ground (13), depth (3) and horse (1) passes never did. The instruction
/// counts are [`GLMARK2`]'s pins, read from that table, so a re-pin there
/// moves this test with it and only a verdict that flips needs a decision.
const LONG: [(&str, &str, bool); 4] = [
    ("refract", "3fae89c9ab969ea3", false),
    ("ground", "cf4bf899d84af04d", false),
    ("depth", "928e172a68b7f175", false),
    ("horse", "474800a67afed087", false),
];
/// Refract's fragment program, in instructions: [`GLMARK2`]'s pin, named here
/// because W6's narrow rule turns on it (9 below `LONG_PROGRAM_INSTRS`).
const REFRACT_FRAG_INSTRS: u32 = 55;

/// The pinned sizes against the rule, with no corpus: the verdicts of
/// [`LONG`] follow from [`GLMARK2`]'s pins and the threshold, and the table
/// names refract at [`REFRACT_FRAG_INSTRS`]. This half always runs.
#[test]
fn pinned_fragment_sizes_against_long_program() {
    use yakogl::vivante::tiled_rt::{long_program, LONG_PROGRAM_INSTRS};
    assert_eq!(LONG_PROGRAM_INSTRS, 64, "LONG's verdicts were read at the 64 threshold");
    for (name, hash, long) in LONG {
        let (_, ext, instrs, _) =
            GLMARK2.iter().find(|g| g.0 == hash).unwrap_or_else(|| panic!("{name}: not in GLMARK2"));
        assert_eq!(*ext, "frag", "{name}");
        assert_eq!(
            long_program(instrs * 4, LONG_PROGRAM_INSTRS),
            long,
            "{name}: {instrs} instructions against {LONG_PROGRAM_INSTRS}; W6's narrow rule changes for this pass"
        );
        if name == "refract" {
            assert_eq!(
                *instrs, REFRACT_FRAG_INSTRS,
                "refract's fragment pin moved; restate its distance to the threshold"
            );
        }
    }
    assert!(!long_program(REFRACT_FRAG_INSTRS * 4, LONG_PROGRAM_INSTRS));
    // where refract stood before rank 3: exactly on the inclusive threshold
    assert!(long_program(64 * 4, LONG_PROGRAM_INSTRS));
}

/// The same verdicts from the compiler's own output (`usc_dw`, the value W6
/// would hand `long_program`) for the corpus texts. Same corpus rules as the
/// test above: a SKIP line without one.
#[test]
fn glmark2_fragment_programs_against_long_program() {
    use yakogl::vivante::tiled_rt::{long_program, LONG_PROGRAM_INSTRS};
    let Some((dir, files)) = corpus("long_program half") else { return };
    for (name, hash, long) in LONG {
        let (f, src) = corpus_text(&dir, &files, hash, "frag");
        let (dw, _) = compile(SHC_STAGE_FRAGMENT, &src);
        let want = GLMARK2.iter().find(|g| g.0 == hash).expect("in GLMARK2").2;
        println!("gc7000 {f}: {name} fragment, {} instructions ({dw} dwords), long_program {long}", dw / 4);
        assert_eq!(dw % 4, 0, "{name}: whole instructions");
        assert_eq!(dw / 4, want, "{name}: instructions against GLMARK2's pin");
        assert_eq!(long_program(dw, LONG_PROGRAM_INSTRS), long, "{name}: long_program at {LONG_PROGRAM_INSTRS}");
    }
}
