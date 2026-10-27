// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The shaders gl_test cases 69, 70, 71, 74, 75, 77, 78, 79, 81, 84 and 86 draw with, compiled the way the board
//! compiles them. `alloc_clobber.rs` checks what the allocator did to them;
//! this checks that the product accepts them at all.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online"))]

/// Cases 69, 70, 71, 74, 75, 77, 78, 79, 81, 84 and 86 draw these through the whole chain on the board, so
/// the whole chain -- selection, the passes, allocation, encoding and the ABI
/// -- has to take them here first. One test in this binary, because it resets
/// the keep arena (`test_rules`).
#[test]
fn the_gl_test_shapes_compile_through_the_product_chain() {
    use core::ffi::c_char;
    use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
    for (stage, src) in [
        (SHC_STAGE_VERTEX, include_str!("../tools/shaders/es100/ioswap.vert")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/loopcarry.frag")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/deadloop.frag")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/ifoutput.frag")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/ifoutput2.frag")),
        (SHC_STAGE_VERTEX, include_str!("../tools/shaders/es100/immneg.vert")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/immneg.frag")),
        (SHC_STAGE_VERTEX, include_str!("../tools/shaders/es100/matvary.vert")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/matvary.frag")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/idiv.frag")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/transc.frag")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/vtransc.frag")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/branchquad.frag")),
        (SHC_STAGE_VERTEX, include_str!("../tools/shaders/es100/anyall.vert")),
        (SHC_STAGE_FRAGMENT, include_str!("../tools/shaders/es100/fragcoord.frag")),
    ] {
        yakogl::shaderc::online::backend::forget_compiled();
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let text = format!("{src}\0");
        let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
        let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
        assert_eq!(rc, 0, "refused: {msg}");
        assert!(out.temps > 0 && !out.usc_words().is_empty(), "compiled to nothing");
        /* A subtracted literal (case 75) is encoded as the NEGATIVE number:
         * no f20 immediate in these programs may be a tiny positive. */
        if src.contains("- 1.0") || src.contains("- 0.5") {
            let mut negatives = 0usize;
            yakogl::shaderc::online::vivante::decode::for_each_instr(out.usc_words(), |_, ins| {
                for n in 0..3 {
                    if let Some(s) = ins.src(n) {
                        if s.is_immediate() && (s.amode >> 1) & 3 == 0 {
                            let v = f32::from_bits(s.imm_payload_bits().unwrap_or(0) << 12);
                            assert!(
                                v == 0.0 || v.abs() >= 1e-6,
                                "an f20 immediate decodes to {v}: a literal lost its sign"
                            );
                            negatives += (v < 0.0) as usize;
                        }
                    }
                }
                Ok(())
            })
            .expect("our own code decodes");
            assert!(negatives > 0, "no negative immediate where a literal is subtracted");
        }
        /* The colour written inside an `if` (case 74) must be written under a
         * branch: with both stores unconditional, the later one always won. */
        if src.contains("gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);\n\t}") {
            let mut branches = 0usize;
            yakogl::shaderc::online::vivante::decode::for_each_instr(out.usc_words(), |_, ins| {
                branches += (ins.op().name == "branch_unary") as usize;
                Ok(())
            })
            .expect("our own code decodes");
            assert!(branches > 0, "a colour written inside an `if` is stored unconditionally");
        }
    }
}
