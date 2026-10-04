// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#![cfg(all(feature = "shaderc-remote", feature = "shaderc-online", feature = "gpu-vivante"))]
//! The GPU-wedging pair: a six-arm if/else-if chain in a VERTEX shader.
//!
//! `dEQP-GLES2.functional.shaders.algorithm.hsl_to_rgb_vertex` and
//! `rgb_to_hsl_vertex` wedge the GC7000 when compiled by the ONLINE pair and
//! pass on the remote pair. This reconstructs the shape from
//! `es2fShaderAlgorithmTests.cpp:173` -- a six-arm if/else-if chain, no loop --
//! and puts both back ends' output side by side, with the same arithmetic as a
//! fragment shader as the internal control, because the `*_fragment` variants
//! never wedge.
//!
//! **Three hypotheses this refuted**, each of which would otherwise have been
//! worked on:
//!
//! 1. *A branch target that makes a loop.* There are **no branches at all** in
//!    our output: a vertex shader has no `discard` and no texture fetch, so
//!    `must_not_speculate` is false and the chain is if-converted.
//! 2. *Instruction count or temps.* The same arithmetic as a FRAGMENT shader
//!    is the same 153 instructions with temps 43, and never wedges.
//! 3. *Touching a register beyond the declared `temps`* -- which `abi.md`
//!    says corrupts ANOTHER TASK's registers, a plausible way to hang a GPU.
//!    The highest register touched is 41 against 42 declared; the fragment
//!    control is 42 against 43. Both within.
//! 4. *A load-balancing word outside the domain it was measured in.* The
//!    shipped vertex blobs reach temps 42, and the band `0x0f3f0321` spans
//!    temps 3..42, so the value does not depend on temps and our 41 is inside
//!    the shipped range.
//!
//! Needs the compile server; run it by name.
use core::ffi::c_char;
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

const HSL: &str = "attribute vec4 a_position;\nattribute vec4 a_color;\nvarying vec4 v_color;\nvoid main() {\n  gl_Position = a_position;\n  float h = a_color.x * 6.0;\n  float s = a_color.y;\n  float l = a_color.z;\n  float c = (1.0 - abs(2.0*l - 1.0)) * s;\n  float x = c * (1.0 - abs(mod(h, 2.0) - 1.0));\n  vec3 rgb;\n  if (h < 1.0) rgb = vec3(c, x, 0.0);\n  else if (h < 2.0) rgb = vec3(x, c, 0.0);\n  else if (h < 3.0) rgb = vec3(0.0, c, x);\n  else if (h < 4.0) rgb = vec3(0.0, x, c);\n  else if (h < 5.0) rgb = vec3(x, 0.0, c);\n  else rgb = vec3(c, 0.0, x);\n  v_color = vec4(rgb, 1.0);\n}\n";

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
    let mut bad = Vec::new();
    for (i, q) in w.as_chunks::<4>().0.iter().enumerate() {
        let Ok(ins) = decode_instr(q) else {
            println!("     [{i}] DID NOT DECODE");
            continue;
        };
        let name = ins.op().name;
        if let Some(t) = ins.get("TARGET") {
            let back = (t as usize) <= i;
            println!("     [{i:3}] {name:<14} TARGET={t}{}", if back { "   <== BACKWARD OR SELF" } else { "" });
            if back || t == 0 {
                bad.push((i, name, t));
            }
        } else if name.starts_with("branch") {
            println!("     [{i:3}] {name:<14} (no TARGET field?)");
        }
    }
    println!("     branches with a backward/zero target: {}", bad.len());
    /* Does anything touch a register at or beyond the declared temps?
     * `abi.md`: a shader that uses more temps than it declares corrupts
     * ANOTHER TASK's registers -- which is a plausible way to hang a GPU. */
    let mut highest = 0u32;
    for q in w.as_chunks::<4>().0 {
        let Ok(i) = decode_instr(q) else { continue };
        if i.get("DST_USE") == Some(1) {
            if let Some(d) = i.get("DST") {
                highest = highest.max((d >> 3) & 0x7f);
            }
        }
        for n in 0..3 {
            if i.get(&format!("SRC{n}_USE")) == Some(1) && i.get(&format!("SRC{n}_RGROUP")) == Some(0) {
                if let Some(r) = i.get(&format!("SRC{n}_REG")) {
                    highest = highest.max(r);
                }
            }
        }
    }
    println!(
        "     highest register touched: {highest}; declared temps {} -> {}",
        sh.temps,
        if highest + 1 > u32::from(sh.temps) { "OVER-RUNS ITS DECLARATION" } else { "within" }
    );
    let mut hist = std::collections::BTreeMap::new();
    for q in w.as_chunks::<4>().0 {
        if let Ok(i) = decode_instr(q) {
            *hist.entry(i.op().name).or_insert(0usize) += 1;
        }
    }
    println!("     ops: {hist:?}");
    println!("     last 4:");
    for (i, q) in w.as_chunks::<4>().0.iter().enumerate().skip(n.saturating_sub(4)) {
        match decode_instr(q) {
            Ok(ins) => {
                let f: Vec<String> = ins
                    .op()
                    .fields
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| ins.vals[*k] != 0)
                    .map(|(k, fl)| format!("{}={:#x}", fl.name, ins.vals[k]))
                    .collect();
                println!("       [{i:3}] {:<10} {}", ins.op().name, f.join(" "));
            }
            Err(e) => println!("       [{i:3}] <{e:?}>"),
        }
    }
}

fn run(remote: bool, src: &str) -> Result<ShcShader, String> {
    run_stage(SHC_STAGE_VERTEX, remote, src)
}

/// The same arithmetic as a FRAGMENT shader: the internal control, because
/// the *_fragment dEQP variants never wedge and the *_vertex ones always do.
const HSL_FRAG: &str = "precision mediump float;\nvarying vec4 v_in;\nvoid main() {\n  float h = v_in.x * 6.0;\n  float s = v_in.y;\n  float l = v_in.z;\n  float c = (1.0 - abs(2.0*l - 1.0)) * s;\n  float x = c * (1.0 - abs(mod(h, 2.0) - 1.0));\n  vec3 rgb;\n  if (h < 1.0) rgb = vec3(c, x, 0.0);\n  else if (h < 2.0) rgb = vec3(x, c, 0.0);\n  else if (h < 3.0) rgb = vec3(0.0, c, x);\n  else if (h < 4.0) rgb = vec3(0.0, x, c);\n  else if (h < 5.0) rgb = vec3(x, 0.0, c);\n  else rgb = vec3(c, 0.0, x);\n  gl_FragColor = vec4(rgb, 1.0);\n}\n";

#[test]
fn the_wedging_shader() {
    println!("\n=== hsl_to_rgb, VERTEX, six-arm if/else-if chain ===");
    match (run(false, HSL), run(true, HSL)) {
        (Ok(o), Ok(r)) => {
            report("OURS", &o);
            report("REF ", &r);
        }
        (a, b) => println!("  ours {:?}\n  ref ok={}", a.err(), b.is_ok()),
    }
    println!("\n=== the SAME arithmetic as a FRAGMENT shader (never wedges) ===");
    match (run_stage(SHC_STAGE_FRAGMENT, false, HSL_FRAG), run_stage(SHC_STAGE_FRAGMENT, true, HSL_FRAG)) {
        (Ok(o), Ok(r)) => {
            report("OURS-frag", &o);
            report("REF -frag", &r);
        }
        (a, b) => println!("  ours {:?}\n  ref ok={}", a.err(), b.is_ok()),
    }
}
