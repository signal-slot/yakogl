// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! No instruction the online compiler emits reads two different
//! constant-file registers.
//!
//! The reference never does: over all 13,908 instructions of its table, an
//! instruction reads at most ONE constant register (several lanes of it, as
//! often as it likes -- which is what tests/operand_banks.rs counts, and why
//! that count could not see this). We folded a uniform and a compiler
//! literal into one `mul` in front of every sin and cos, and on silicon
//! sin(pi/6) read 0.314 (gl_test 77, imx8mp-board at 7902405) while log2 and
//! exp2, which read one register, were right. The rule is the reference's,
//! and the check is on the bytes the product emits: the whole corpus plus
//! the gl_test shapes. One test in this binary: it resets the keep arena.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

use core::ffi::c_char;
use yakogl::shaderc::mesa_reference::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// (instructions, instructions reading more than one distinct constant register)
fn count(words: &[u32]) -> (usize, usize) {
    let (mut instrs, mut multi) = (0usize, 0usize);
    yakogl::shaderc::online::vivante::decode::for_each_instr(words, |_, ins| {
        instrs += 1;
        let mut regs: Vec<(u8, u16)> = Vec::new();
        for n in 0..3 {
            if let Some(s) = ins.src(n) {
                let key = (s.group, s.reg);
                if (key.0 == 2 || key.0 == 3) && !regs.contains(&key) {
                    regs.push(key);
                }
            }
        }
        multi += (regs.len() > 1) as usize;
        Ok(())
    })
    .expect("decodes");
    (instrs, multi)
}

#[test]
fn no_instruction_reads_two_constant_registers() {
    /* The rule, measured on the reference: never. */
    let (ref_instrs, ref_multi) =
        SHC_BLOB_TABLE.iter().map(|b| count(b.shader.usc_words())).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    assert!(ref_instrs > 5_000, "the reference table shrank: {ref_instrs}");
    assert_eq!(ref_multi, 0, "the reference reads two constant registers in one instruction after all");

    let mut sources: Vec<(String, String)> =
        SHC_SOURCE_TABLE.iter().map(|e| (e.name.to_string(), e.text.trim_end_matches('\0').to_string())).collect();
    for (n, t) in [
        ("es100/transc.frag", include_str!("../tools/shaders/es100/transc.frag")),
        ("es100/vtransc.frag", include_str!("../tools/shaders/es100/vtransc.frag")),
        ("es100/idiv.frag", include_str!("../tools/shaders/es100/idiv.frag")),
        ("es100/immneg.frag", include_str!("../tools/shaders/es100/immneg.frag")),
        ("es100/immneg.vert", include_str!("../tools/shaders/es100/immneg.vert")),
        ("es100/intlit.frag", include_str!("../tools/shaders/es100/intlit.frag")),
    ] {
        sources.push((n.to_string(), t.to_string()));
    }
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    let (mut compiled, mut bad) = (0usize, Vec::new());
    for (name, text) in &sources {
        let stage = if name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        yakogl::shaderc::online::backend::forget_compiled();
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 256];
        let src = format!("{text}\0");
        // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
        if unsafe { compile(stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) } != 0 {
            continue;
        }
        compiled += 1;
        let (_, multi) = count(out.usc_words());
        if multi > 0 {
            bad.push(format!("{name}: {multi}"));
        }
    }
    println!("{compiled} programs compiled; reading two constant registers: {bad:?}");
    assert!(compiled > 100, "the population collapsed: {compiled}");
    assert!(bad.is_empty(), "{} programs read two constant registers in one instruction", bad.len());
}
