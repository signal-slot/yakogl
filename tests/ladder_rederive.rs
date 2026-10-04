// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
//! Re-derive the wedge ladder so its rungs are named by MEASURED temps.
//!
//! The first ladder (tools/cts/testdata/ladder/n4_k*.vert) stopped measuring
//! anything: at ccb2e81 its rungs allocated 27..37 temps, and at head all of
//! them allocate 8. The sources were the reason. Each value was an independent
//! function of a_position and they were consumed by a COMMUTATIVE SUM:
//!
//!     vec4 t0 = p * 1.00 + vec4(0.00);   ...   vec4 t19 = p * 1.19 + ...;
//!     vec4 s  = t0 + t1 + t2 + ... + t19;
//!
//! A scheduler may compute t0, fold it into the accumulator, kill it, then do
//! t1, and so on: peak liveness 2, whatever k is. That is a correct and good
//! optimisation, and it means the file name says 20 values while the hardware
//! is told 8 registers.
//!
//! A ladder has to force the register count on whatever compiler builds it, so
//! this generator makes the values simultaneously live BY CONSTRUCTION:
//!
//!   1. Each value depends on the previous one, so no definition can be sunk
//!      past its successor.
//!   2. They are consumed by a LEFT-ASSOCIATED SUBTRACTION chain, in reverse
//!      order of definition. Subtraction is neither commutative nor
//!      associative in floating point, so a correct compiler may not reorder
//!      the consumption; and because t0 is consumed LAST, every value stays
//!      live from its definition to the end of the chain.
//!
//! The temps are then READ BACK from the compiler rather than assumed, which
//! is the rule this whole exercise produced: a rung is named by the temps the
//! pair's compiler reports for it, never by the generator's parameter.
use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_VERTEX};

/// `k` values, `num_out` varyings.
pub fn gen(k: usize, num_out: usize, vec3: bool) -> String {
    let ty = if vec3 { "vec3" } else { "vec4" };
    let mut s = String::from("attribute vec4 a_position;\n");
    for i in 0..num_out {
        s.push_str(&format!("varying {ty} v{i};\n"));
    }
    s.push_str("void main() {\n  vec4 p = a_position;\n");
    // (1) a chain: t(i) reads t(i-1), so definitions cannot be reordered.
    s.push_str("  vec4 t0 = p * 1.000 + vec4(0.125);\n");
    for i in 1..k {
        let c = 1.0 + (i as f32) * 0.01;
        let d = 0.125 + (i as f32) * 0.007;
        s.push_str(&format!("  vec4 t{i} = p * {c:.3} + t{prev} * {d:.3};\n", prev = i - 1));
    }
    // (2) consume in REVERSE by left-associated subtraction: not reorderable,
    // and t0 is used last, so all k values are live at the first subtraction.
    s.push_str(&format!("  vec4 s = t{};\n", k - 1));
    for i in (0..k - 1).rev() {
        s.push_str(&format!("  s = s - t{i};\n"));
    }
    s.push_str("  gl_Position = a_position;\n");
    for i in 0..num_out {
        let swz = if vec3 { ".xyz" } else { "" };
        s.push_str(&format!("  v{i} = (s * {}.0){swz};\n", i + 1));
    }
    s.push_str("}\n");
    s
}

fn compile(stage: u32, src: &str) -> Result<(u16, usize), String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let mut s = String::from(src);
    s.push('\0');
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.unwrap();
    let rc = unsafe { f(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok((out.temps, out.usc_words().len() / 4))
    } else {
        let m: Vec<u8> = log.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        Err(String::from_utf8_lossy(&m).into_owned())
    }
}

/// ONE test in this binary, deliberately.
///
/// The compiler's scratch arena is global and not reentrant, so two `#[test]`s
/// in one binary run concurrently and one of them fails with "a compile was
/// started inside a compile". This file hit it on its first gate run, with both halves as separate tests.
/// They are one test now, and the shape check runs first because the table is
/// only meaningful if the generator really forces liveness.
#[test]
fn rederive_ladder_tables() {
    shape_forces_temps();

    // num_out 2 and 3 are included because the ABI guard's ceiling is a
    // function of num_out (src/shaderc/online/abi.rs): 21 at 0|1, 42 at 2|3,
    // 16 at 4|5 and 6|7. The wedge boundaries at temps 26..29 are ABOVE the
    // ceiling at num_out 4 and 1, so no rung there can reach them at all --
    // and reachable at num_out 2|3, which is where a head-pair rung has to be
    // built.
    for (label, num_out, vec3) in
        [("n4", 4usize, false), ("n2", 2usize, false), ("n3", 3usize, false), ("vec3-n1", 1usize, true)]
    {
        println!("\n== ladder {label} (num_out {num_out}, {}) ==", if vec3 { "vec3" } else { "vec4" });
        println!("   k  online");
        let mut seen: Vec<(usize, u16)> = Vec::new();
        for k in 2..=40 {
            let src = gen(k, num_out, vec3);
            let online = match compile(SHC_STAGE_VERTEX, &src) {
                Ok((t, w)) => {
                    seen.push((k, t));
                    format!("temps {t:3}, {w:4} words")
                }
                // The ceiling is our own wedge mitigation (abi.rs,
                // LoadBalancingTempsUnmeasured), NOT a front-end gap: say so
                // in the table rather than printing a bare "REFUSED" that the
                // next reader has to attribute again.
                Err(e) if e.contains("LoadBalancingTempsUnmeasured") => "REFUSED BY CEILING".to_string(),
                Err(e) => format!("REFUSED: {}", e.trim()),
            };
            println!("  {k:2}   {online}");
        }
        // Name the rungs that straddle the two boundaries on THIS pair: the
        // same source is a different rung on another compiler, so a file name
        // is not a rung and never was.
        for (path, rows) in [("online", &seen)] {
            for target in [26u16, 28] {
                let lo = rows.iter().find(|(_, t)| *t == target);
                let hi = rows.iter().find(|(_, t)| *t == target + 1);
                match (lo, hi) {
                    (Some((a, _)), Some((b, _))) => println!(
                        "  {path}: straddles {}/{} at k={} and k={}",
                        target,
                        target + 1,
                        a,
                        b
                    ),
                    _ => println!(
                        "  {path}: {}/{} NOT REACHABLE with k<=40 (refused by the ceiling, or the allocator never gets there)",
                        target,
                        target + 1
                    ),
                }
            }
        }
    }
}

fn shape_forces_temps() {
    // The generator is only worth anything if its temps TRACK k. The old one
    // did not, which is the whole finding: it reported 8 for every rung.
    // Compare the two shapes on the same pair, same k.
    // num_out 2 (ceiling 42) and a k whose temps sit under it: this test is
    // about the SHAPE forcing liveness, not about the ABI guard.
    let k = 20;
    let new = compile(SHC_STAGE_VERTEX, &gen(k, 2, false)).expect("new shape compiles");
    let mut old = String::from("attribute vec4 a_position;\nvarying vec4 v0;\nvoid main() {\n  vec4 p = a_position;\n");
    for i in 0..k {
        old.push_str(&format!("  vec4 t{i} = p * {:.3} + vec4({:.3});\n", 1.0 + i as f32 * 0.01, i as f32 * 0.01));
    }
    old.push_str("  vec4 s = t0");
    for i in 1..k {
        old.push_str(&format!(" + t{i}"));
    }
    old.push_str(";\n  gl_Position = a_position;\n  v0 = s;\n}\n");
    let old = compile(SHC_STAGE_VERTEX, &old).expect("old shape compiles");
    println!("k={k}: old (commutative sum) temps {}, new (pinned chain, reverse subtraction) temps {}", old.0, new.0);
    assert!(
        new.0 > old.0,
        "the new shape must force more temps than the sum shape it replaces \
             (old {}, new {}) -- if it does not, the generator is not a ladder",
        old.0,
        new.0
    );
    assert!(
        new.0 as usize >= k,
        "temps ({}) should track k ({}): the values are meant to be \
             simultaneously live by construction",
        new.0,
        k
    );
}
