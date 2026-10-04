// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#![cfg(all(feature = "shaderc-remote", feature = "shaderc-online", feature = "gpu-vivante"))]
//! The REAL dEQP sources for the GPU-wedging pair, through both back ends.
//!
//! `tests/wedge_probe.rs` reconstructs the shape of these shaders. This one
//! compiles the **exact text dEQP sends**, read from
//! `tools/cts/testdata/wedge-shaders/`: generated from dEQP's
//! `createExpressionCase()` template (the body is TAB-indented inside a
//! space-indented `main()`), and proven byte-exact because every one of the
//! four came back from the shader server as `{"cached": true}` -- the cache key
//! hashes the source, and the only thing that had put those texts in that cache
//! was the board run of those same four cases.
//!
//! On the board (i.MX8MP, gcgpu 469e79b2f91d, online pair):
//!   hsl_to_rgb_vertex    wedges ALONE on a fresh module   2/2
//!   rgb_to_hsl_vertex    Fail alone 3/3; wedges after ANY prior kick 3/3
//!   *_fragment variants  never wedge  <-- the internal control
//! The remote pair passes all four with an empty kernel log.
//!
//! Run it by name; needs the compile server for the remote arm.
use core::ffi::c_char;
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

const D: &str = "tools/cts/testdata/wedge-shaders";

fn run_stage(stage: u32, remote: bool, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let mut s = String::from(src);
    s.push('\0');
    let f = if remote {
        yakogl::shaderc::remote::SHC_REMOTE.compile.unwrap()
    } else {
        yakogl::shaderc::online::SHC_ONLINE.compile.unwrap()
    };
    let rc = unsafe { f(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        let m: Vec<u8> = log.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        Err(String::from_utf8_lossy(&m).into_owned())
    }
}

fn report(tag: &str, sh: &ShcShader) {
    let w = sh.usc_words();
    let n = w.len() / 4;
    println!(
        "  {tag}: {n} instructions, temps {} vs_output_size {} uses_w {} pos_reg {}",
        sh.temps, sh.vs_output_size, sh.uses_w, sh.viv.vs_pos_out_reg
    );
    let mut highest = 0u32;
    let mut branches = 0usize;
    for q in w.as_chunks::<4>().0 {
        let Ok(i) = decode_instr(q) else { continue };
        if i.op().name.starts_with("branch") || i.get("TARGET").is_some() {
            branches += 1;
        }
        if i.get("DST_USE") == Some(1) {
            if let Some(d) = i.get("DST") {
                highest = highest.max((d >> 3) & 0x7f);
            }
        }
        for k in 0..3 {
            if i.get(&format!("SRC{k}_USE")) == Some(1) && i.get(&format!("SRC{k}_RGROUP")) == Some(0) {
                if let Some(r) = i.get(&format!("SRC{k}_REG")) {
                    highest = highest.max(r);
                }
            }
        }
    }
    println!(
        "     branches {branches}; highest register touched {highest} vs declared temps {} -> {}",
        sh.temps,
        if highest + 1 > u32::from(sh.temps) { "OVER-RUNS" } else { "within" }
    );
    // The words, so the receivers can disassemble with `yakogl-tools etna-dis
    // --hex` and diff the two back ends without re-running anything.
    let hex: Vec<String> = w.iter().map(|x| format!("{x:08x}")).collect();
    println!("     words[{}]: {}", w.len(), hex.join(" "));
}

#[test]
fn real_sources_both_backends() {
    for case in ["hsl_to_rgb_vertex", "rgb_to_hsl_vertex", "hsl_to_rgb_fragment", "rgb_to_hsl_fragment"] {
        let vs = std::fs::read_to_string(format!("{D}/{case}.vert")).unwrap();
        let fs = std::fs::read_to_string(format!("{D}/{case}.frag")).unwrap();
        println!(
            "================ {case}   (wedges: {})",
            if case.ends_with("_vertex") { "YES on the online pair" } else { "no -- the internal control" }
        );
        for (stage, name, src) in [(SHC_STAGE_VERTEX, "VS", &vs), (SHC_STAGE_FRAGMENT, "FS", &fs)] {
            for (remote, who) in [(false, "online"), (true, "remote")] {
                match run_stage(stage, remote, src) {
                    Ok(sh) => report(&format!("{name} {who}"), &sh),
                    Err(e) => println!("  {name} {who}: REFUSED: {e}"),
                }
            }
        }
    }
}

#[test]
fn gles2_core_probe_variants() {
    // The two-axis experiment gles2-core landed, so the HOST numbers are on
    // record next to the board's before I run them: if the staged pair reports
    // different ones, that mismatch matters more than the wedge result.
    for f in ["a_many_temps", "b_many_instructions"] {
        let src = std::fs::read_to_string(format!("tools/cts/wedge_probe/{f}.vert")).unwrap();
        println!("================ wedge_probe/{f}.vert");
        for (remote, who) in [(false, "online"), (true, "remote")] {
            match run_stage(SHC_STAGE_VERTEX, remote, &src) {
                Ok(sh) => report(&format!("VS {who}"), &sh),
                Err(e) => println!("  VS {who}: REFUSED: {e}"),
            }
        }
    }
}
