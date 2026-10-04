// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#![cfg(all(feature = "shaderc-remote", feature = "shaderc-online", feature = "gpu-vivante"))]
//! What temp count does each rung of the ladder actually declare?
//!
//! The board experiment varies the SOURCE; what the GPU is told is the
//! compiler's temp count, and the two are not the same number. This prints the
//! mapping for both back ends so the board table can be read in temps rather
//! than in "how many vec4s I wrote".
use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_VERTEX};

fn temps(remote: bool, src: &str) -> Result<(u16, usize), String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let mut s = String::from(src);
    s.push('\0');
    let f = if remote {
        yakogl::shaderc::remote::SHC_REMOTE.compile.unwrap()
    } else {
        yakogl::shaderc::online::SHC_ONLINE.compile.unwrap()
    };
    let rc = unsafe { f(SHC_STAGE_VERTEX, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok((out.temps, out.usc_words().len() / 4))
    } else {
        let m: Vec<u8> = log.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        Err(String::from_utf8_lossy(&m).into_owned())
    }
}

#[test]
fn ladder_temps() {
    /* The board's ladder is GENERATED, into a gitignored directory that
     * exists only in the session that made it, so this test used to panic in
     * every other worktree. It now takes the directory from `LADDER_DIR` and
     * SKIPS when there is none -- a test that cannot find its data has not
     * failed, and one that panics instead of saying so hides whatever else
     * the run was asked to report.
     *
     * `tools/cts/wedge_probe/steps` is the committed set, and is the
     * right argument when the question is about the COMPILER rather than
     * about a particular board run. */
    let d = std::env::var("LADDER_DIR").unwrap_or_else(|_| "build/imx8mp/milestones/temps-ladder".into());
    let Ok(dir) = std::fs::read_dir(&d) else {
        println!("temps_ladder: no ladder at {d} (set LADDER_DIR); skipped");
        return;
    };
    let d = &d;
    let mut names: Vec<_> = Ok::<_, ()>(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.ends_with(".vert"))
        .collect();
    names.sort();
    println!("{:<10} {:>18} {:>18}", "rung", "online t/instr", "remote t/instr");
    for n in names {
        let src = std::fs::read_to_string(format!("{d}/{n}")).unwrap();
        let o = temps(false, &src);
        let r = temps(true, &src);
        let f = |x: &Result<(u16, usize), String>| match x {
            Ok((t, i)) => format!("{t}/{i}"),
            Err(_) => "REFUSED".into(),
        };
        println!("{:<10} {:>18} {:>18}", n, f(&o), f(&r));
        /* The reason, WHOLE and on its own line. It used to be truncated to
         * 20 characters inside an 18-wide column, which reads as "online
         * shaderc: the" and names neither the construct nor the pass -- and
         * a refusal nobody can attribute is a refusal nobody fixes. */
        for (which, x) in [("online", &o), ("remote", &r)] {
            if let Err(e) = x {
                println!("           {which}: {e}");
            }
        }
    }
}
