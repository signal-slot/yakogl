// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An A/B instrument for a compiler change that must not change its output
//! (issue #101): every corpus source through this build's online compiler,
//! written to a file to diff against the other arm. A test binary of its
//! own, because it resets the compiler (`src/test_rules.rs`).
#![cfg(all(feature = "shaderc-online", any(feature = "gpu-powervr", feature = "gpu-vivante")))]
use core::ffi::c_char;
use std::path::{Path, PathBuf};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

const GPU: &str = if cfg!(feature = "gpu-powervr") { "powervr" } else { "gc7000" };

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("tools/shaders is readable") {
        let p = e.expect("a directory entry").path();
        if p.is_dir() {
            sources(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("vert" | "frag")) {
            out.push(p);
        }
    }
}

/// The bytes this build's online compiler emits for every corpus source, for
/// an A/B of a change that must not alter them: one line per source with the
/// program's dwords and the fields the driver reads (or the refusal), and for
/// a fragment shader that reads `gl_FrontFacing` its front-flipped variant
/// too. Written to the file `ONLINE_CORPUS_DUMP` names; diff two runs. Each
/// compile's scratch high-water goes to the same name plus `.scratch`, apart,
/// because a change that keeps the bytes may still move it.
///
/// ```text
/// ONLINE_CORPUS_DUMP=build/before.txt cargo test --no-default-features \
///   --features host-test,gpu-vivante,shaderc-online --test online_corpus_dump -- --ignored
/// ```
#[test]
#[ignore = "an A/B instrument: writes the file ONLINE_CORPUS_DUMP names"]
fn dump_every_compiled_program() {
    use yakogl::shaderc::online::store;
    let Some(out_path) = std::env::var_os("ONLINE_CORPUS_DUMP") else {
        panic!("set ONLINE_CORPUS_DUMP to the file to write");
    };
    fn line(sh: &ShcShader) -> String {
        // SAFETY: a program the online back end just filled: `usc` holds `usc_dw` dwords.
        let usc = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        let mut s = format!(
            "dw={} temps={} uniform_dwords={} uniform_sh_dwords={} varyings={} attribs={} uniforms={} uses_discard={} front={} blend_key={} usc=",
            sh.usc_dw,
            sh.temps,
            sh.uniform_dwords,
            sh.uniform_sh_dwords,
            sh.num_varyings,
            sh.num_attribs,
            sh.num_uniforms,
            sh.uses_discard,
            sh.reads_front_facing,
            sh.blend_key
        );
        for w in usc {
            s.push_str(&format!("{w:08x}"));
        }
        for v in &sh.varyings[..sh.num_varyings as usize] {
            s.push_str(&format!(" v{}:{}:{}:{}", v.location, v.components, v.interp, v.vtxout_off));
        }
        for a in &sh.attribs[..sh.num_attribs as usize] {
            s.push_str(&format!(" a{}:{}:{}:{}", a.location, a.components, a.vi_reg, a.format));
        }
        s
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
    let mut files = Vec::new();
    sources(&root, &mut files);
    files.sort();
    let backend = &yakogl::shaderc::online::SHC_ONLINE;
    let (compile, variant) = (backend.compile.expect("compiles"), backend.blend_variant.expect("has variants"));
    let (mut dumped, mut refused, mut variants) = (0usize, 0usize, 0usize);
    let mut text = String::new();
    let mut scratch = String::new();
    let mut peak = (0usize, String::new());
    for p in &files {
        let rel = p.strip_prefix(&root).expect("under the root").to_string_lossy().into_owned();
        let stage = if rel.ends_with(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let src = format!("{}\0", std::fs::read_to_string(p).expect("a UTF-8 source"));
        yakogl::shaderc::online::backend::forget_compiled();
        let mut sh = ShcShader::ZERO;
        let mut log = [0 as c_char; 1024];
        // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
        let rc = unsafe { compile(stage, src.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) };
        let hw = store::last_scratch_peak();
        scratch.push_str(&format!("{rel}: {hw}\n"));
        if hw > peak.0 {
            peak = (hw, rel.clone());
        }
        if rc != 0 {
            let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
            text.push_str(&format!("{rel}: refused {}\n", msg.trim_end().replace('\n', " / ")));
            refused += 1;
            continue;
        }
        text.push_str(&format!("{rel}: {}\n", line(&sh)));
        dumped += 1;
        if stage == SHC_STAGE_FRAGMENT && sh.reads_front_facing != 0 {
            let mut flipped = ShcShader::ZERO;
            let key = yakogl::shaderc::SHC_VIV_KEY_FRONT_FLIPPED;
            // SAFETY: a shader this back end compiled, a real output, a log of the size announced.
            let rc = unsafe { variant(&sh, key, &mut flipped, log.as_mut_ptr(), log.len()) };
            let body = if rc == 0 { line(&flipped) } else { format!("variant refused rc={rc}") };
            text.push_str(&format!("{rel} @front_flipped: {body}\n"));
            variants += 1;
        }
    }
    println!(
        "{GPU}: {} sources: {dumped} programs dumped, {refused} refused, {variants} front-flipped variants",
        files.len()
    );
    println!("{GPU}: largest scratch high-water {} bytes ({}) of {}", peak.0, peak.1, store::SCRATCH_BYTES);
    assert_eq!(dumped + refused, files.len(), "every source is either dumped or refused");
    assert!(dumped > 0, "a dump of no program compares nothing");
    std::fs::write(&out_path, text).expect("the dump file is writable");
    let mut scratch_path = out_path.clone();
    scratch_path.push(".scratch");
    std::fs::write(&scratch_path, scratch).expect("the scratch file is writable");
}
