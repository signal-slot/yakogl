// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 shadow's and refract's eight application shaders, and the
//! one-statement shaders that isolate the causes of their length, through the
//! GC7000 online compiler: the instruction and temporary counts pinned
//! (`docs/notes/gc-shadow-refract-shaders.md`, plan #328 item
//! sr-compiler-fix). Mesa etnaviv's `etna_shc` counts for the GC7000UL are
//! the comments; they are the target, not a pin. The comments also give the
//! count with the note's "The patch" (a copy fold and a scalar lane merge in
//! `src/shaderc/online/vivante/`), measured and not landed: it moves the
//! shared `tests/fixtures/precompile_counts.txt`. The item that lands it
//! re-pins these numbers to the "patch" column.
//!
//! The glmark2 texts are GPL-3.0 and never committed: they are read from the
//! host harness's corpus (`GLMARK2_CORPUS`, by default
//! `build/glmark2-host/shaders`, which `tools/hostharness/glmark2/run.sh
//! "$GLMARK2_SRC"` writes), found by the text's hash whatever scene first
//! compiled it. With no corpus at the default place the glmark2 half prints a
//! SKIP line; a `GLMARK2_CORPUS` that lacks a file fails. The micro-shaders
//! are this repository's own and always run.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_sr_shader_count -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use std::path::Path;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// Instructions and temporaries of one compile.
fn count(stage: u32, src: &str) -> (u32, u32) {
    yakogl::shaderc::online::backend::forget_compiled();
    let (mut sh, mut log) = (ShcShader::ZERO, [0 as c_char; 1024]);
    let text = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, text.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) };
    let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "compile failed: {msg}");
    (sh.usc_dw / 4, sh.temps.into())
}

/// (text hash, stage, ours: instructions and temps, etna_shc's in the comment)
const GLMARK2: [(&str, &str, u32, u32); 8] = [
    ("37f46ae8961221f2", "vert", 6, 3),   // depth vertex; etna 4, 3; patch 5, 3
    ("928e172a68b7f175", "frag", 3, 3),   // depth fragment; etna 2, 2; patch 3, 3
    ("b9d18073e5979540", "vert", 12, 4),  // ground vertex; etna 7, 4; patch 11, 5
    ("cf4bf899d84af04d", "frag", 16, 5),  // ground fragment; etna 11, 4; patch 13, 5
    ("6964fcf6aaac9739", "vert", 23, 5),  // horse vertex; etna 14, 4; patch 21, 5
    ("474800a67afed087", "frag", 1, 2),   // horse fragment; etna 1, 2; patch 1, 2
    ("7215f4f5052a383c", "vert", 23, 5),  // refract vertex; etna 19, 5; patch 21, 5
    ("3fae89c9ab969ea3", "frag", 86, 12), // refract fragment; etna 48, 6; patch 71, 12
];
/// The eight's instructions: etna_shc 105; the patch 146.
const GLMARK2_TOTAL: u32 = 170;

const HEAD: &str = "precision mediump float;\nvarying vec4 a;\nvarying vec4 b;\n";

/// The note's micro-shaders (fragment stage unless named), the counts ours.
fn micro() -> Vec<(&'static str, u32, String, u32)> {
    let fs = |body: &str| format!("{HEAD}void main() {{ {body} }}\n");
    vec![
        // m1: etna 2 (one rcp, one mul); patch 2
        ("m1", SHC_STAGE_FRAGMENT, fs("gl_FragColor = a / a.w;"), 5),
        // m1b: etna 3; patch 4
        ("m1b", SHC_STAGE_FRAGMENT, fs("gl_FragColor = vec4(a.xyz / a.w, 1.0);"), 6),
        // m13: etna 3 (mad); patch 6 (no contraction)
        ("m13", SHC_STAGE_FRAGMENT, fs("vec3 t = a.xyz * b.x + b.xyz; gl_FragColor = vec4(t * 0.5 + 0.5, 1.0);"), 6),
        // m9: etna 7; patch 10
        (
            "m9",
            SHC_STAGE_FRAGMENT,
            fs("const vec4 L = vec4(1.0, 2.0, 3.0, 2.0); gl_FragColor = vec4(normalize(a.xyz / a.w - L.xyz / L.w), 1.0);"),
            17,
        ),
        // m5: etna 10; patch 17
        ("m5", SHC_STAGE_FRAGMENT, fs("gl_FragColor = vec4(refract(a.xyz, b.xyz, 1.2), 1.0);"), 20),
        // m8: etna 4; patch 7
        ("m8", SHC_STAGE_FRAGMENT, fs("float k = dot(a.xyz, b.xyz); gl_FragColor = vec4(vec3(sqrt(1.0 - k * k)), 1.0);"), 8),
        // m4 (vertex): etna 4; patch 5
        (
            "m4",
            SHC_STAGE_VERTEX,
            "attribute vec3 position;\nuniform mat4 M;\nvoid main() { gl_Position = M * vec4(position, 1.0); }\n".into(),
            6,
        ),
        // the difference of two varyings: `mov t, -b` and an add in ours; patch 1
        ("neg", SHC_STAGE_FRAGMENT, fs("gl_FragColor = a - b;"), 2),
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
            "SKIP gc_sr_shader_count glmark2 half: no corpus at {} (set GLMARK2_SRC and run tools/hostharness/glmark2/run.sh)",
            dir.display()
        );
    } else {
        let mut total = 0;
        for (hash, ext, want_n, want_t) in GLMARK2 {
            let suffix = format!("-{hash}.{ext}.glsl");
            let f = files
                .iter()
                .find(|f| f.ends_with(&suffix))
                .unwrap_or_else(|| panic!("{} has no *{suffix}", dir.display()));
            let src = std::fs::read_to_string(dir.join(f)).expect("a shader file");
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
