// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! gl_test 108's program D as this compiler now reads a run-time-indexed
//! uniform (select.rs LoadUniformIdx), and the one measurement knob left
//! (`shc_debug_uniidx`), in a binary of their own: the knob is
//! process-global, and a test compiling in parallel elsewhere would read it.
#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online"))]
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::select::{shc_debug_set_uniidx, UNIIDX_NO_CLAMP};
use yakogl::shaderc::online::vivante::{decode, listing};

/// 108 D's fragment shader, compiled with `bits`: its listing, and its
/// constant table's kinds.
fn compile(bits: i32) -> (Vec<String>, Vec<u8>) {
    let text = include_str!("../tools/shaders/es100/uniidx_d.frag");
    shc_debug_set_uniidx(bits);
    forget_compiled();
    let src = std::ffi::CString::new(text).unwrap();
    let mut sh = yakogl::shaderc::ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    // SAFETY: a live source, output and log for the length of the call.
    let rc = unsafe { (SHC_ONLINE.compile.unwrap())(1, src.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    shc_debug_set_uniidx(0);
    assert_eq!(rc, 0, "D compiles with knobs {bits}");
    // SAFETY: the compiler's output: `usc_dw` words at `usc`, `const_count` kinds.
    let (words, kinds) = unsafe {
        (
            core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize),
            core::slice::from_raw_parts(sh.viv.const_contents, sh.viv.const_count as usize),
        )
    };
    let lines = words
        .chunks(4)
        .enumerate()
        .map(|(i, c)| {
            let mut t = String::new();
            listing::instr_text(&mut t, i, &decode::decode_instr(&c.try_into().unwrap()).unwrap()).unwrap();
            t.trim_end().to_string()
        })
        .collect();
    (lines, kinds.to_vec())
}

/// Mesa's etnaviv reads gl_test 108 D's `fcols[fidx]` as `imullo0.s32` (the
/// byte offset) then `load.u32` from the uniform block's address -- its
/// golden stream, cs_golden mode 25 on the GC7000 rev 6204 shim
/// (2026-09-25) -- and never through the address register. So does this
/// compiler now: an offset multiply and a load, the address a
/// `SHC_VIV_CONST_UBO_ADDR` constant, no movar, no relative constant read,
/// and no second copy of the array in the constant file. One test: the knob
/// is one process-global int.
#[test]
fn a_run_time_indexed_uniform_is_a_load_from_the_blocks_copy_in_memory() {
    let has = |l: &[String], what: &str| l.iter().any(|x| x.contains(what));
    let (d, kinds) = compile(0);
    assert!(has(&d, "imullo0.s32") && has(&d, "load"), "offset multiply and load: {d:#?}");
    assert!(!has(&d, "movar") && !has(&d, "[a.x]"), "no address register: {d:#?}");
    assert!(has(&d, "cmp.lt.s32"), "the clamp (ir.md) stays by default: {d:#?}");
    assert_eq!(
        kinds.iter().filter(|k| **k == yakogl::shaderc::SHC_VIV_CONST_UBO_ADDR).count(),
        1,
        "one UBO_ADDR constant: {kinds:?}"
    );
    /* fcols (16 dwords) and fidx (1), rounded to five registers, then the
     * UBO_ADDR register: no per-array copy after them */
    assert!(kinds.len() <= 6 * 4, "no second copy of fcols: {} constants", kinds.len());

    let (a, _) = compile(UNIIDX_NO_CLAMP);
    assert!(!has(&a, "cmp") && !has(&a, "select"), "no clamp: {a:#?}");
    assert!(has(&a, "imullo0.s32") && has(&a, "load"), "still the load: {a:#?}");
}
