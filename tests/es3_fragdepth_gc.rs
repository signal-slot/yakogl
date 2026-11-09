// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `gl_FragDepth` on the GC7000UL: the compiler half.
//!
//! dEQP-GLES3 `shaders.fragdepth.*` was refused at compile on the GC7000UL
//! with "this output is not selected yet". The fragment shader now writes the
//! depth, clamped to [0, 1], to `r0.z` -- the lane the rasteriser put the
//! pixel's depth in, where Mesa's etnaviv writes it (etnaviv_compiler_nir_ra.c
//! REG_FRAG_DEPTH, `ps_depth_out_reg = 0` in etnaviv_compiler_nir.c) -- and the
//! program says so: `viv.ps_depth_out_reg` is 0 and `writes_depth` is 1.
//!
//! `gl_FragCoord` arrives in that same register, so a shader that reads it
//! after writing the depth would read the written value. Selection copies r0
//! before the first instruction when a shader does both, and every
//! `gl_FragCoord` read takes the copy: the pixel's depth, as GLSL says.
//!
//! The values are checked by running the ENCODED program in the
//! lane-faithful interpreter of `tests/common`. The PE and early-Z words a
//! depth-writing draw needs are `src/vivante`'s (item gc-fragdepth-earlyz).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_fragdepth_gc
//! ```
#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

mod common;

use std::sync::Mutex;

use common::{run, Run};
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::isa::Instr;
use yakogl::shaderc::online::vivante::{decode, listing};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_VIV_NO_REG};

/// The compiler keeps its output in one arena that every compile resets.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// One compiled fragment shader: what the record says, decoded.
struct Compiled {
    code: Vec<Instr>,
    /// The constant file, four dwords per register, as the driver fills it
    /// for a target drawn upright (gl_FragCoord's y scale 1, offset 0).
    consts: Vec<[u32; 4]>,
    ps_depth_out_reg: u8,
    ps_color_out_reg: u8,
    writes_depth: u8,
    /// The register the varying `v_d` arrives in.
    v_d_reg: Option<u8>,
}

fn io_name(name: &[core::ffi::c_char; 32]) -> String {
    name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// Compile through `SHC_ONLINE`, the path glCompileShader takes.
fn compile(src: &str) -> Result<Compiled, String> {
    let _serial = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    forget_compiled();
    let text = std::ffi::CString::new(src).unwrap();
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(SHC_STAGE_FRAGMENT, text.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect());
    }
    // SAFETY: the record's code, `usc_dw` dwords, alive until the next compile.
    let words = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
    let code = words.chunks(4).map(|c| decode::decode_instr(&c.try_into().unwrap()).expect("decodes")).collect();
    let mut consts = vec![[0u32; 4]; 512];
    for (k, (&kind, &data)) in sh.viv.const_contents().iter().zip(sh.viv.const_data()).enumerate() {
        consts[k / 4][k % 4] = match kind {
            yakogl::shaderc::SHC_VIV_CONST_LITERAL => data,
            yakogl::shaderc::SHC_VIV_CONST_UNUSED => 0,
            yakogl::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_SCALE => 1.0f32.to_bits(),
            yakogl::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_OFFSET => 0,
            /* a uniform: zero, which only the build-only dEQP test reads */
            _ => 0,
        };
    }
    let v_d_reg = sh.viv.in_[..sh.viv.num_in as usize].iter().find(|x| io_name(&x.name) == "v_d").map(|x| x.reg);
    Ok(Compiled {
        code,
        consts,
        ps_depth_out_reg: sh.viv.ps_depth_out_reg,
        ps_color_out_reg: sh.viv.ps_color_out_reg,
        writes_depth: sh.writes_depth,
        v_d_reg,
    })
}

fn listing_of(c: &Compiled) -> String {
    let mut t = String::new();
    for (i, ins) in c.code.iter().enumerate() {
        listing::instr_text(&mut t, i, ins).unwrap();
    }
    t
}

/// The instructions that write register `reg`, with their write masks.
fn writes_to(c: &Compiled, reg: u8) -> Vec<(usize, u8)> {
    c.code
        .iter()
        .enumerate()
        .filter(|(_, ins)| ins.get("DST_USE").unwrap_or(0) != 0)
        .filter_map(|(i, ins)| ins.dst().filter(|d| d.reg == reg && d.amode == 0).map(|d| (i, d.write_mask)))
        .collect()
}

/// Run the program for one pixel: gl_FragCoord in r0, `v_d` in its row.
/// Gives r0 and the colour register after the last instruction.
fn shade(c: &Compiled, frag_coord: [f32; 4], v_d: f32) -> ([f32; 4], [f32; 4]) {
    let mut regs = vec![[0u32; 4]; 128];
    regs[0] = frag_coord.map(f32::to_bits);
    if let Some(r) = c.v_d_reg {
        regs[r as usize] = [v_d.to_bits(); 4];
    }
    match run(&c.code, &regs, &c.consts) {
        Run::Done { regs, .. } => (regs[0].map(f32::from_bits), regs[c.ps_color_out_reg as usize].map(f32::from_bits)),
        Run::Unsupported(why) => panic!("the interpreter cannot run it: {why}\n{}", listing_of(c)),
    }
}

const WRITE: &str = "#version 300 es\nprecision highp float;\nin float v_d;\nout vec4 o_color;\n\
    void main()\n{\n    o_color = vec4(0.25, 0.5, 0.75, 1.0);\n    gl_FragDepth = v_d;\n}\n";

/// dEQP fragdepth.compare's shape: the depth read back through gl_FragCoord
/// AFTER gl_FragDepth is written.
const READ_AFTER: &str = "#version 300 es\nprecision highp float;\nin float v_d;\nout vec4 o_color;\n\
    void main()\n{\n    gl_FragDepth = v_d;\n    o_color = vec4(gl_FragCoord.z, gl_FragCoord.x, 0.0, 1.0);\n}\n";

/// The same in a loop: the second trip reads gl_FragCoord after the first
/// trip's depth write, which no ordering of one body can avoid.
const READ_IN_LOOP: &str = "#version 300 es\nprecision highp float;\nin float v_d;\n\
    out vec4 o_color;\nvoid main()\n{\n    float acc = 0.0;\n    for (int i = 0; i < 2; i++) {\n        \
    acc += gl_FragCoord.z;\n        gl_FragDepth = v_d + float(i);\n    }\n    o_color = vec4(acc);\n}\n";

/// The pixel's own position and depth, as the rasteriser leaves them in r0.
const PIXEL: [f32; 4] = [12.5, 7.5, 0.375, 1.0];

/// A shader writing gl_FragDepth compiles for the GC7000UL, and its record
/// names r0 as the depth register and says it writes depth.
#[test]
fn a_fragdepth_shader_builds_with_r0_as_its_depth_register() {
    let c = compile(WRITE).unwrap_or_else(|e| panic!("refused: {e}"));
    assert_eq!(c.ps_depth_out_reg, 0, "ps_depth_out_reg");
    assert_eq!(c.writes_depth, 1, "writes_depth");
    assert_ne!(c.ps_color_out_reg, 0, "the colour is never in r0");
}

/// A shader that leaves gl_FragDepth alone keeps the record it had.
#[test]
fn a_shader_without_fragdepth_names_no_depth_register() {
    let src = "#version 300 es\nprecision highp float;\nin float v_d;\nout vec4 o_color;\n\
        void main()\n{\n    o_color = vec4(v_d);\n}\n";
    let c = compile(src).unwrap_or_else(|e| panic!("refused: {e}"));
    assert_eq!(c.ps_depth_out_reg, SHC_VIV_NO_REG, "ps_depth_out_reg");
    assert_eq!(c.writes_depth, 0, "writes_depth");
    assert!(writes_to(&c, 0).is_empty(), "nothing writes r0:\n{}", listing_of(&c));
}

/// The code: exactly one instruction writes r0, the last one, lane .z alone
/// (the clamp's upper half; that it clamps is the next test's).
#[test]
fn the_depth_is_written_to_r0_z_alone_by_the_last_instruction() {
    let c = compile(WRITE).unwrap_or_else(|e| panic!("refused: {e}"));
    let w = writes_to(&c, 0);
    assert_eq!(w, vec![(c.code.len() - 1, 0b0100)], "writes to r0:\n{}", listing_of(&c));
}

/// The clamp, by running the program: below 0, inside and above 1, and
/// r0's x, y and w are what the rasteriser left.
#[test]
fn the_written_depth_is_clamped_to_zero_one() {
    let c = compile(WRITE).unwrap_or_else(|e| panic!("refused: {e}"));
    for (v, want) in [(-0.5f32, 0.0f32), (0.0, 0.0), (0.625, 0.625), (1.0, 1.0), (3.0, 1.0)] {
        let (r0, colour) = shade(&c, PIXEL, v);
        assert_eq!(r0[2], want, "gl_FragDepth = {v}:\n{}", listing_of(&c));
        assert_eq!([r0[0], r0[1], r0[3]], [PIXEL[0], PIXEL[1], PIXEL[3]], "r0.xyw for {v}");
        assert_eq!(colour, [0.25, 0.5, 0.75, 1.0], "the colour for {v}");
    }
}

/// The pitfall: gl_FragCoord read after gl_FragDepth is written reads the
/// pixel's depth, not the written one.
#[test]
fn gl_fragcoord_read_after_the_depth_write_reads_the_pixels_depth() {
    let c = compile(READ_AFTER).unwrap_or_else(|e| panic!("refused: {e}"));
    assert_eq!((c.ps_depth_out_reg, c.writes_depth), (0, 1));
    let (r0, colour) = shade(&c, PIXEL, 0.875);
    assert_eq!(r0[2], 0.875, "the depth written:\n{}", listing_of(&c));
    assert_eq!(colour, [PIXEL[2], PIXEL[0], 0.0, 1.0], "gl_FragCoord.z, .x:\n{}", listing_of(&c));
}

/// The same across a loop's trips.
#[test]
fn gl_fragcoord_read_in_a_loop_that_writes_depth_reads_the_pixels_depth() {
    let c = compile(READ_IN_LOOP).unwrap_or_else(|e| panic!("refused: {e}"));
    assert_eq!((c.ps_depth_out_reg, c.writes_depth), (0, 1));
    let (r0, colour) = shade(&c, PIXEL, 0.25);
    assert_eq!(r0[2], 1.0, "the last trip's depth, 1.25 clamped:\n{}", listing_of(&c));
    assert_eq!(colour, [2.0 * PIXEL[2]; 4], "two trips of gl_FragCoord.z:\n{}", listing_of(&c));
}

/// dEQP-GLES3 `shaders.fragdepth.write.*` and `.compare.*`'s eight fragment
/// shaders (es3fFragDepthTests.cpp, as tests/es3_shaderc_repro.rs lowers
/// them) build for the GC7000UL, say they write depth, and touch r0's `.z`
/// alone.
#[test]
fn the_deqp_fragdepth_shaders_build() {
    let prelude = "#version 300 es\nuniform highp vec4 u_color;\nuniform highp float uf_half;\n\
        uniform bool ub_true;\nuniform int ui_two;\nuniform highp float uf_fourth;\nin highp vec2 v_coord;\n\
        layout(location = 0) out mediump vec4 o_color;\n";
    let bodies = [
        ("const", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = 0.5;\n}"),
        ("uniform", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = uf_half;\n}"),
        ("dynamic", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n}"),
        ("fragcoord_z", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = gl_FragCoord.z;\n}"),
        (
            "uniform_conditional_write",
            "void main (void)\n{\n    o_color = u_color;\n    if (ub_true)\n        gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n}",
        ),
        (
            "dynamic_conditional_write",
            "void main (void)\n{\n    o_color = u_color;\n    mediump float d = (v_coord.x+v_coord.y)*0.5f;\n    if (v_coord.y < 0.5)\n        gl_FragDepth = d;\n    else\n        gl_FragDepth = 1.0 - d;\n}",
        ),
        (
            "uniform_loop_write",
            "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = 0.0;\n    for (int i = 0; i < ui_two; i++)\n        gl_FragDepth += uf_fourth;\n}",
        ),
        (
            "write_in_function",
            "void myfunc (highp vec2 coord)\n{\n    gl_FragDepth = (coord.x+coord.y)*0.5;\n}\nvoid main (void)\n{\n    o_color = u_color;\n    myfunc(v_coord);\n}",
        ),
    ];
    for (name, body) in bodies {
        let c = compile(&format!("{prelude}{body}\n")).unwrap_or_else(|e| panic!("fragdepth.{name}: refused: {e}"));
        assert_eq!((c.ps_depth_out_reg, c.writes_depth), (0, 1), "fragdepth.{name}");
        let w = writes_to(&c, 0);
        assert!(
            !w.is_empty() && w.iter().all(|&(_, mask)| mask == 0b0100),
            "fragdepth.{name}: r0 writes {w:?}:\n{}",
            listing_of(&c)
        );
    }
}
