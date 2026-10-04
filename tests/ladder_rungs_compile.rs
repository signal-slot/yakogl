// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
//! Every ladder rung must COMPILE, or the rung measured a refusal.
//!
//! The wedge ladders (tools/cts/RESULTS.md) read a rung as "drew" or "wedged"
//! from a board run. A third outcome exists and looks like neither in the
//! durable record: the compiler refusing the source, after which the probe
//! never reaches the draw at all. A drawing rung leaves a `.out` file proving
//! it got that far; a WEDGED rung leaves nothing, because the run is killed --
//! so on disk, "wedged" and "was refused" are the same absence.
//!
//! gles2-core reported (2026-09-22) that `s20`..`s36` are refused while
//! `s08`/`s12`/`s16` compile. The cause was named afterwards and it is NOT the
//! front end: it is `abi.rs`'s VS_LOAD_BALANCING temps ceiling
//! (`LoadBalancingTempsUnmeasured { num_out: 1, temps: 24, temps_max: 21 }`),
//! our own wedge mitigation. Those are not these sources, but
//! the same trap applies to any rung, and it is cheap to close: compile every
//! rung here and fail if one is refused.
use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_VERTEX};

const RUNGS: &[(&str, &str)] = &[
    ("n4_k20", include_str!("../tools/cts/testdata/ladder/n4_k20.vert")),
    ("n4_k21", include_str!("../tools/cts/testdata/ladder/n4_k21.vert")),
    ("n4_k22", include_str!("../tools/cts/testdata/ladder/n4_k22.vert")),
    ("n4_k23", include_str!("../tools/cts/testdata/ladder/n4_k23.vert")),
    ("n4_k24", include_str!("../tools/cts/testdata/ladder/n4_k24.vert")),
    ("n4_k26", include_str!("../tools/cts/testdata/ladder/n4_k26.vert")),
    ("n4_k28", include_str!("../tools/cts/testdata/ladder/n4_k28.vert")),
    ("n4_k30", include_str!("../tools/cts/testdata/ladder/n4_k30.vert")),
];

#[test]
fn every_ladder_rung_compiles() {
    let mut refused = Vec::new();
    for (name, src) in RUNGS {
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let mut s = String::from(*src);
        s.push('\0');
        let f = yakogl::shaderc::online::SHC_ONLINE.compile.unwrap();
        let rc = unsafe { f(SHC_STAGE_VERTEX, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        if rc == 0 {
            println!("{name}: temps {}, {} instruction words", out.temps, out.usc_words().len() / 4);
        } else {
            let m: Vec<u8> = log.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
            let m = String::from_utf8_lossy(&m).into_owned();
            println!("{name}: REFUSED -- {m}");
            refused.push(format!("{name}: {m}"));
        }
    }
    assert!(
        refused.is_empty(),
        "these ladder rungs are refused by the online compiler, so any board \
             reading of them measured a refusal and not a wedge:\n  {}",
        refused.join("\n  ")
    );
}
