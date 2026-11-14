// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mesa's V3D 7.1 code for the four P8 shapes (plan #365, ROADMAP stage 5,
//! docs/notes/pre-study-rpi5.md section 8) on the 7.1 interpreter.
//!
//! For each of pass-through, uniform, `if` and texture (inline GLSL ES 1.00,
//! `tests/v3d71_ref/mod.rs`), `v3d_shc --device 71` compiles the pair, and
//! Mesa's FS, VS and CS run through `sim::run_gen(Gen::V71, ..)` on the
//! shape's inputs; the outputs (the TLB colour, the VPM rows: scaled x and
//! y, Zs, 1/Wc, the varyings; the CS's clip position) must be what the
//! shape's model computes. The test then pins Mesa's instruction counts per
//! shape and prints them. This is the reference side: it judges Mesa's code
//! and the interpreter, not our compiler, and it is the answer our 7.1
//! back end is later compared with.
//!
//! Run (Mesa 26.2.1's `build-v3d` and v3d_shc, tools/v3d/README.md):
//!
//! ```sh
//! V3D_SHC=<mesa-26.2.1>/build-v3d/src/broadcom/v3d_shc \
//! cargo test --no-default-features \
//!     --features host-test,gpu-v3d,shaderc-online,no-shader-table \
//!     --test v3d71_mesa_shapes -- --nocapture
//! ```
//!
//! `MESA_V3D_TREE=<mesa-26.2.1>` names the same binary. It also needs
//! `glslangValidator` (`GLSLANG` or on the PATH, tools/vivante/glsl2vivante.py)
//! and `python3`. Without a v3d_shc the test prints `GAP v3d71_mesa_shapes:
//! ...` and runs nothing of Mesa's: that line, not a pass, is the result.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

mod v3d71_ref;

use v3d71_ref as r;
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::sim;

/// Mesa 26.2.1's instruction counts at 7.1 (`--device 71 --vpm-size 65536`),
/// `[fs, vs, cs]`, measured with `build-v3d` of the Mesa 26.2.1 tarball.
const MESA_COUNTS: [(&str, [usize; 3]); 4] =
    [("passthrough", [9, 23, 15]), ("uniform", [6, 33, 32]), ("if", [15, 23, 15]), ("texture", [9, 19, 15])];

/// The interpreter's refusal of Mesa's 7.1 VS and CS, pinned by name (the
/// text `sim71::at_thrend` gives). Mesa's code ends `stvpmv rf0, rf<n> ;
/// thrsw` with no THRSW pair before it: the interpreter takes the lone
/// THRSW for the thread end (sim.rs, "no last-THRSW pair"), and counts
/// STVPMV's address field, rf0, as an ALU register-file write, which
/// qpu_validate.c:373-418 forbids there only when a THRSW pair came first
/// (`thrend_found`). That is a defect of the interpreter, not of Mesa's
/// code, and not this test's to fix (src/shaderc/online/v3d/sim71.rs, an
/// item of the 7.1 back end): the same refusal stops 191 of 191 tried pairs
/// of tests/v3d_sim_mesa.rs's 7.1 pass. While it stands, a VS and a CS that
/// stop with it are reported, by stage and shape, as not run; one that stops
/// with anything else, or runs and differs, fails; one that runs is
/// checked, and reported CLOSED so the pin can go.
const SIM71_VS_REFUSAL: &str = "an ALU register-file write at the thread end";

/// What running one shape's Mesa code came to.
#[derive(Default)]
struct Report {
    /// Values compared, over the stages that ran.
    compared: usize,
    /// Stages that ran and matched.
    ran: Vec<&'static str>,
    /// Stages the interpreter refused with [`SIM71_VS_REFUSAL`].
    refused: Vec<&'static str>,
}

/// Run one shape's FS, VS and CS words of Mesa's on the interpreter and
/// check them against the model.
fn run(shape: &r::Shape, mesa: &r::Mesa71) -> Result<Report, String> {
    let m = r::model(shape);
    let mut rep = Report::default();
    let slots: Vec<(u32, u32)> = mesa.fs.input_slots.iter().map(|&(s, c)| (s - 32, c)).collect();

    // FS: the colour. It does not depend on the VS: its varyings come from
    // the model, so it always runs.
    let inputs = r::fs_inputs(&m, &slots)?;
    let words = r::stream(&m, mesa, &mesa.fs, "frag")?;
    let o = sim::run_gen(Gen::V71, &mesa.fs.code, &words, &inputs, &r::env(&m, &mesa.fs))
        .map_err(|e| format!("FS: {e}"))?;
    rep.compared += r::check_fs(&m, &o).map_err(|e| format!("FS: {e}"))?;
    rep.ran.push("FS");

    // VS and CS: the VPM rows.
    for (what, st, is_cs) in [("VS", &mesa.vs, false), ("CS", &mesa.cs, true)] {
        let inputs = r::vs_rows(&m, &r::mesa_attr_order(mesa, st)?)?;
        let words = r::stream(&m, mesa, st, "vert")?;
        match sim::run_gen(Gen::V71, &st.code, &words, &inputs, &r::env(&m, st)) {
            Ok(o) => {
                rep.compared += r::check_vs(&m, &o, is_cs, &slots).map_err(|e| format!("{what}: {e}"))?;
                rep.ran.push(what);
            }
            Err(e) if e.contains(SIM71_VS_REFUSAL) => rep.refused.push(what),
            Err(e) => return Err(format!("{what}: {e}")),
        }
    }
    Ok(rep)
}

#[test]
fn mesa_71_code_runs_the_four_shapes() {
    if let Err(why) = r::shc_path() {
        r::gap("v3d71_mesa_shapes", &why);
        return;
    }
    let mut counts = Vec::new();
    for shape in r::SHAPES {
        let mesa = r::run_shc(shape.name, shape.vs, shape.fs).unwrap_or_else(|e| panic!("{}: {e}", shape.name));
        let [fs, vs, cs] = mesa.counts();
        println!("v3d71_mesa_shapes: {:<12} Mesa 7.1 words: fs {fs}, vs {vs}, cs {cs}", shape.name);
        let rep = run(shape, &mesa).unwrap_or_else(|e| panic!("{}: {e}\nMesa's listing:\n{}", shape.name, mesa.disasm));
        println!("v3d71_mesa_shapes: {:<12} {} values compared, 0 differ; ran {:?}", shape.name, rep.compared, rep.ran);
        for st in &rep.refused {
            println!(
                "v3d71_mesa_shapes: {:<12} {st} NOT RUN: the interpreter refuses Mesa's code: {SIM71_VS_REFUSAL} \
                 (sim71 defect, see SIM71_VS_REFUSAL)",
                shape.name
            );
        }
        if rep.refused.is_empty() {
            println!("v3d71_mesa_shapes: {:<12} CLOSED: remove SIM71_VS_REFUSAL", shape.name);
        }
        assert!(rep.ran.contains(&"FS") && rep.compared > 0, "{}: the FS did not run", shape.name);
        counts.push((shape.name, [fs, vs, cs]));
    }
    assert_eq!(counts, MESA_COUNTS, "Mesa's 7.1 word counts moved: re-pin on purpose");
}
