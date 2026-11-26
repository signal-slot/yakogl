// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `fbo.depth.depth_{write,test}_clamp.*` on the GC7000UL (cause
//! `gc-depth-clamp`, 10 board Fails at campaign commit 9bc82915), on the rig
//! of `backend_tests.rs`.
//!
//! Both cases draw dEQP's `DepthGradientShader` (es3fFboTestUtil.cpp), whose
//! fragment shader writes a per-fragment `gl_FragDepth` gradient from -1 to 2,
//! first with the depth test ALWAYS, then (`depth_test_clamp`) against what
//! the first draw left with LESS. ES 3.0 clamps the written depth to [0, 1]
//! before the test and the write, for the fixed-point formats and for
//! DEPTH_COMPONENT32F and DEPTH32F_STENCIL8 alike.
//!
//! Why the board failed them: at 9bc82915 the GC online compiler refused every
//! `gl_FragDepth` store ("this output is not selected yet", select.rs), so the
//! gradient program never linked and all ten cases failed on that, whatever
//! the depth format. The host column the comparison table set against it was
//! taken at 5454be1b, after the three commits that make the write work:
//! `70a2050a` (the selector writes `min(max(x, 0), 1)` into r0.z alone,
//! `ps_depth_out_reg` 0), `5fa91ff6` (the early-depth rule keeps a depth
//! writer late) and `b3f42d14` (the rule wired into `emit_pe_state` and
//! `emit_ra_state`). So the cause read "board-only", and the clamp itself
//! lives in the shader, not in the back end.
//!
//! What the back end must hold for the clamp to reach memory, pinned here:
//! - the program the compiler makes of dEQP's gradient shader names r0 as its
//!   depth register and writes the clamp's upper half to r0.z
//!   alone (`the_deqp_gradient_shader_builds_and_clamps_into_r0_z`);
//! - its draws keep the depth test and write in the late pixel engine (no
//!   EARLY_Z, no DISABLE_ZS, RA early write off), so the PE tests and stores
//!   the shader's clamped r0.z and not the rasteriser's interpolated z;
//! - PE_DEPTH_NEAR / FAR stay 0 / 1 and PE_DEPTH_NORMALIZE 2^24 - 1 whatever
//!   glDepthRangef says: the written depth is clamped to [0, 1], not to the
//!   depth range, and every GL depth format (16, 24, 32F, with or without
//!   stencil) is stored as D24S8 (the comment above `pe_depth` in
//!   backend.rs; `depth_format.rs`'s D16 surface is not called yet), so one
//!   normalisation covers the five formats the cases name. The application's range moves PA_VIEWPORT_*_Z only.

use super::*;

/// The fragment shader of dEQP's `DepthGradientShader` for an RGBA8 colour
/// target (`glu::TYPE_FLOAT_VEC4`), after es3fFboTestUtil.cpp. Written out
/// from that file's text, not copied: the CTS sources are not in this tree,
/// and the shape that matters is a per-fragment, uniform-scaled write.
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
const DEPTH_GRADIENT_FS: &str = "#version 300 es\n\
    in highp vec4 v_coord;\n\
    uniform highp float u_minGradient;\n\
    uniform highp float u_maxGradient;\n\
    uniform highp vec4 u_color;\n\
    layout(location = 0) out highp vec4 o_color;\n\
    void main (void)\n\
    {\n\
    \thighp float x = v_coord.x;\n\
    \thighp float y = v_coord.y;\n\
    \thighp float f0 = (x + y) * 0.5;\n\
    \tgl_FragDepth = u_minGradient + (u_maxGradient-u_minGradient)*f0;\n\
    \to_color = u_color;\n\
    }\n";

/// The rig's fragment program, said to write `gl_FragDepth` to r0 as the
/// compiler's record says (`ps_depth_out_reg` 0, `writes_depth` 1).
fn depth_writer(rig: &Rig) -> ShcShader {
    let mut fs = *rig.fs;
    fs.viv.ps_depth_out_reg = 0;
    fs.writes_depth = 1;
    fs
}

/// The two draws of the clamp cases: the gradient with the test ALWAYS, then
/// (`depth_test_clamp`) the same program tested LESS against it; both write.
fn clamp_draws(rig: &Rig, fs: &ShcShader) -> [(&'static str, GlesDraw); 2] {
    let mut d = rig.draw(6);
    d.fs = fs;
    d.depth_test = true;
    d.depth_write = true;
    d.depth_func = GLES_CMP_ALWAYS;
    let mut t = d;
    t.depth_func = GLES_CMP_LESS;
    [("the fill, ALWAYS", d), ("the test, LESS", t)]
}

/// The late depth words of a depth-testing, depth-writing draw of a program
/// that writes `gl_FragDepth`: D24S8, the draw's function, the write on, and
/// neither half of early depth.
fn assert_late_depth(s: &Stream, what: &str, func: GlesCmp) {
    let pe = s.reg(GC_PE_DEPTH_CONFIG);
    assert_eq!(pe & 0xf, GC_PE_DEPTH_MODE_Z, "{what}: depth mode Z");
    assert_eq!(pe & 0xf0, GC_PE_DEPTH_FORMAT_D24S8, "{what}: every GL depth format is stored D24S8");
    assert_eq!((pe >> 8) & 0x7, func, "{what}: the draw's function");
    assert_ne!(pe & GC_PE_DEPTH_WRITE_ENABLE, 0, "{what}: the PE writes the depth");
    assert_eq!(pe & GC_PE_DEPTH_EARLY_Z, 0, "{what}: no early test of the rasteriser's z");
    assert_eq!(pe & GC_PE_DEPTH_DISABLE_ZS, 0, "{what}: the late stage tests and writes");
    let ra = s.reg(GC_RA_EARLY_DEPTH);
    assert_eq!(ra & GC_RA_EARLY_DEPTH_TEST_ENABLE, 0, "{what}: RA early test off");
    assert_ne!(ra & GC_RA_EARLY_DEPTH_WRITE_DISABLE, 0, "{what}: RA early write off");
    assert_eq!(ra, 0x1500_0030, "{what}: RA_EARLY_DEPTH as tests/es3_fragdepth_gc.rs pins it");
}

/// The pixel engine's own depth bounds and normalisation.
fn pe_bounds(s: &Stream) -> (f32, f32, f32) {
    (
        f32::from_bits(s.reg(GC_PE_DEPTH_NEAR)),
        f32::from_bits(s.reg(GC_PE_DEPTH_FAR)),
        f32::from_bits(s.reg(GC_PE_DEPTH_NORMALIZE)),
    )
}

/// The clamp cases' draws test and write late, into D24S8, with the PE's
/// bounds at [0, 1]. On a linear target early depth is never on, so these
/// words match a plain program's; they are pinned so that a change to the
/// linear-target rule which turns early depth on for a depth writer, or sets
/// DISABLE_ZS, fails here.
#[test]
fn the_clamp_cases_draws_test_and_write_the_shaders_depth_late() {
    let rig = Rig::new();
    let fs = depth_writer(&rig);
    for (what, d) in clamp_draws(&rig, &fs) {
        let s = rig.submit(&d);
        assert_late_depth(&s, what, d.depth_func);
        assert_eq!(pe_bounds(&s), (0.0, 1.0, 16_777_215.0), "{what}: PE depth bounds and normalisation");
    }
    assert!(rig.faults().is_empty());
}

/// glDepthRangef moves the viewport's z (scale far - near, offset near) and
/// nothing in the pixel engine: the written depth is clamped to [0, 1], not
/// to the range, so PE_DEPTH_NEAR / FAR stay 0 / 1, an inverted range
/// included. Revert check: feed `gl.depth_range` into PE_DEPTH_NEAR / FAR
/// and the first non-default range fails.
#[test]
fn the_pe_depth_bounds_stay_zero_one_under_any_depth_range() {
    let rig = Rig::new();
    let fs = depth_writer(&rig);
    for range in [[0.0f32, 1.0], [0.25, 0.75], [1.0, 0.0], [0.5, 0.5]] {
        for (what, mut d) in clamp_draws(&rig, &fs) {
            d.depth_range = range;
            let s = rig.submit(&d);
            let what = format!("{what}, glDepthRangef({}, {})", range[0], range[1]);
            assert_late_depth(&s, &what, d.depth_func);
            assert_eq!(pe_bounds(&s), (0.0, 1.0, 16_777_215.0), "{what}: PE depth bounds");
            assert_eq!(
                (f32::from_bits(s.reg(GC_PA_VIEWPORT_SCALE_Z)), f32::from_bits(s.reg(GC_PA_VIEWPORT_OFFSET_Z))),
                (range[1] - range[0], range[0]),
                "{what}: the viewport's z"
            );
        }
    }
    assert!(rig.faults().is_empty());
}

/// A program that does not write depth, drawn after one that does, gets the
/// same late words on a linear target and the same bounds: the depth writer
/// changes nothing the next draw inherits. And a depth writer drawn after a
/// plain program is not left with its words.
#[test]
fn a_depth_writer_and_a_plain_program_alternate_cleanly() {
    let rig = Rig::new();
    let writer = depth_writer(&rig);
    let [(_, w), _] = clamp_draws(&rig, &writer);
    let mut plain = w;
    plain.fs = &*rig.fs;
    for (what, d) in [("writer", w), ("plain after writer", plain), ("writer after plain", w)] {
        let s = rig.submit(&d);
        assert_late_depth(&s, what, GLES_CMP_ALWAYS);
        assert_eq!(pe_bounds(&s), (0.0, 1.0, 16_777_215.0), "{what}: PE depth bounds");
    }
    assert!(rig.faults().is_empty());
}

/// The program the online compiler makes of dEQP's gradient shader, the one
/// 9bc82915 refused: it builds, names r0 as its depth register, and exactly
/// one instruction writes r0, lane .z alone: `select.gt` against the literal
/// 1.0 (the clamp's `min`) of the temporary that the instruction right before
/// it wrote with `select.lt` against 0.0 (the `max`). The colour is written
/// after it, as the source orders the two. Its record then drives the rig's
/// draws into the late words above.
#[test]
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
fn the_deqp_gradient_shader_builds_and_clamps_into_r0_z() {
    use crate::shaderc::online::vivante::{decode, listing};

    let rig = Rig::new();
    let _serial = crate::shaderc::online::store::test_lock();
    let mut real = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 1024];
    let text = format!("{DEPTH_GRADIENT_FS}\0");
    let compile = crate::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: NUL-terminated source, a real output, a log of the announced size.
    let rc = unsafe {
        compile(
            crate::shaderc::SHC_STAGE_FRAGMENT,
            text.as_ptr() as *const core::ffi::c_char,
            &mut real,
            log.as_mut_ptr(),
            log.len(),
        )
    };
    let log_text: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "the gradient shader compiles (9bc82915 refused it): {log_text}");
    assert_eq!((real.viv.ps_depth_out_reg, real.writes_depth), (0, 1), "(ps_depth_out_reg, writes_depth)");

    let code: Vec<_> = real
        .usc_words()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|q| decode::decode_instr(q).expect("our own output decodes"))
        .collect();
    let mut text = String::new();
    for (i, ins) in code.iter().enumerate() {
        listing::instr_text(&mut text, i, ins).expect("listing");
    }
    let r0: Vec<(usize, u8)> = code
        .iter()
        .enumerate()
        .filter(|(_, ins)| ins.get("DST_USE").unwrap_or(0) != 0)
        .filter_map(|(i, ins)| ins.dst().filter(|d| d.reg == 0 && d.amode == 0).map(|d| (i, d.write_mask)))
        .collect();
    assert_eq!(r0.len(), 1, "one instruction writes r0:\n{text}");
    let (at, mask) = r0[0];
    assert_eq!(mask, 0b0100, "r0 is written in lane .z alone:\n{text}");
    assert!(at > 0, "the clamp's two halves:\n{text}");
    let lines: Vec<&str> = text.lines().collect();
    let (hi, lo) = (lines[at], lines[at - 1]);
    let lo_dst = code[at - 1].dst().expect("the max writes a register");
    assert_eq!(lo_dst.write_mask, 0b0100, "the max is computed in lane .z:\n{text}");
    let lo_reg = format!("t{}.", lo_dst.reg);
    assert!(hi.contains("select.gt") && hi.contains("1.0:f20"), "min(.., 1.0) into r0.z:\n{text}");
    assert_eq!(hi.matches(&lo_reg).count(), 2, "of the max's register ({lo_reg}):\n{text}");
    assert!(lo.contains("select.lt") && lo.contains("0.0:f20"), "max(.., 0.0) right before it:\n{text}");
    drop(_serial);

    let mut fs = *rig.fs;
    (fs.viv.ps_depth_out_reg, fs.writes_depth) = (real.viv.ps_depth_out_reg, real.writes_depth);
    for (what, d) in clamp_draws(&rig, &fs) {
        let s = rig.submit(&d);
        assert_late_depth(&s, what, d.depth_func);
        assert_eq!(pe_bounds(&s), (0.0, 1.0, 16_777_215.0), "{what}: PE depth bounds");
    }
    assert!(rig.faults().is_empty());
}
