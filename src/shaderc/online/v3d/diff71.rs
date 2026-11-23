// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The 7.1 back end against the 4.2 one in the QPU interpreters: every
//! program of `tools/shaders/es100` compiled by `chain::compile_program`
//! (V3D 4.2) and by `chain71::compile_program71` (V3D 7.1), each stage run
//! in `sim::run_gen` at its generation on the same inputs, and the outputs
//! compared. The 4.2 back end is the one the board's dEQP runs vouch for,
//! so a 7.1 stage that computes something else is a 7.1 miscompile (or a
//! sim71 defect) until shown otherwise; this keeps the result of plan
//! #386's read-only differential runs (sim71-diff, sim71-reorder) in the
//! gate, so a later change of the 7.1 selection, allocation or scheduling
//! cannot move a value without notice.
//!
//! # The programs
//!
//! Each es100 source is paired with the stub of the other stage that
//! `tools/v3d/es100_ref71.sh` gives Mesa (the pairing of
//! `tests/v3d71_online_corpus.rs`, whose stub writer this module repeats:
//! a stub VS writes every varying the FS declares from a uniform vec4, a
//! stub FS adds every varying the VS declares into gl_FragColor). The
//! sources are embedded with `include_str!` ([`ES100`]), not read from
//! `CARGO_MANIFEST_DIR` at run time: the gate's lib-test rows are cached
//! on a key over `src/` and `tests/`, which a run-time read of
//! `tools/shaders` would sit outside of, so an edited source would leave
//! a stale pass in the cache; through `include_str!` the sources are
//! compiled into the test binary and cargo rebuilds (and the row reruns)
//! when one changes. [`the_embedded_list_is_the_directory`] fails when a
//! source is added to or removed from the directory without this list.
//!
//! # The inputs
//!
//! Fixed and deterministic, the same for both generations, in all 16
//! lanes:
//!
//! * uniforms: every dword of the stage's block a positive float exact in
//!   f16 (1/32 .. 63/32, [`pattern`]), int and bool uniforms small integers and 0 / ~0, the
//!   dEQP index and loop-count uniforms (`ui_zero` .. `ui_four`, `u_i`)
//!   their dEQP values ([`int_value`]);
//! * attributes: one positive value per VPM input row and lane (so W is
//!   positive and no square root or logarithm meets a negative operand);
//! * fragments: four quads in the interpreter's ASSUMED quad layout
//!   (`Env::quads`, so derivatives run), each varying linear in x and y
//!   with a constant C coefficient, W 1 in quads 0 and 1 and 2 in quads 2
//!   and 3, quads 1 and 3 back-facing;
//! * textures: 16 units, a cube where the sampler uniform is a samplerCube;
//!   7x7, 3x3, 1x1 NEAREST_MIPMAP_NEAREST where every look-up names its
//!   level, else one 7x7 level (an implicit level of detail on a chain is
//!   not modelled); every texel distinct and exact in f16 ([`texture`]).
//!
//! # The comparison
//!
//! The fragment shader's TLB writes (configuration and every lane) and its
//! final multisample flags; the vertex and coordinate shaders' VPM output
//! rows. Exact, except the integer screen x and y words (VS rows 0 and 1,
//! CS rows 4 and 5), where 7.1's word may be 4.2's plus 1: 4.2 converts the
//! scaled position with FFLOOR then FTOIZ (a floor) and 7.1 with FTOIN, a
//! round to nearest even (select71.rs rule 3, `viewport_pairs`; Mesa's two
//! generations differ the same way, v3d_nir_lower_io.c:529-543;
//! sim71-reorder's finding). A difference of any other size or sign there
//! is a mismatch.
//!
//! # The control
//!
//! Every stage runs a second time with only the 7.1 run's inputs moved
//! ([`CONTROL_BLIND`]): a stage whose outputs then still agree compares
//! nothing that the inputs reach, and is listed. The counts of what was
//! compared (defined TLB lanes, discarded lanes, TMU look-ups, VPM words,
//! screen words 1 apart) are printed and must not be zero.
//!
//! # The lists
//!
//! [`SKIPS`] names every program either back end refuses and every stage
//! an interpreter cannot run (at either generation), with the reason;
//! [`EXPECTED_MISMATCHES`] names every stage whose outputs differ, with the
//! reason. Both are exact: [`es100_at_4_2_and_7_1_agree_in_the_interpreters`]
//! fails on a skip or mismatch that is not listed and on a listed one that
//! now runs or agrees.
//!
//! # The other in-tree sets
//!
//! [`SETS`] repeats the run over `gl_test_es100`, `gltest`, `femtovg` and
//! the loose files at the top of `tools/shaders`, each an embedded list
//! with its own the-list-is-the-directory test and its own pinned counts
//! ([`Pin`]: programs compiled, stages agreeing, skips, mismatches,
//! control-blind stages). Programs are paired with the es100 stubs, except
//! femtovg, whose `.frag` variants take `femtovg/main.vert` (as femtovg
//! itself pairs them; its text is already in the tree under tools/shaders/femtovg, PROVENANCE.md
//! names no GPL source). List
//! keys start with the set's name. The loose files that are desktop GLSL
//! and the ES 3.00 fragment shaders of `gltest` with `in` varyings are
//! skipped with their reasons.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --lib diff71
//! ```
//!
//! Every es100 program is run; none is left out for time. Under the
//! crate's `[profile.test]` (opt-level 3, the profile the gate's lib rows
//! build) the whole module takes under a second; at opt-level 0 about 25 s,
//! nearly all of it the 4.2 back end compiling three programs
//! (tmparr_const.vert about 13 s, light_amount_0_16.vert 2 s,
//! varyings15.frag 1 s), with each program's six interpreter runs under
//! 0.1 s.

use super::chain::{compile_program, Prim, Target};
use super::chain71::{compile_program71, TARGET71};
use super::qpu_ffi::Gen;
use super::select::{
    Uniform, QUNIFORM_CONSTANT, QUNIFORM_FB_PNTC_Y_OFFSET, QUNIFORM_FB_PNTC_Y_SCALE, QUNIFORM_FB_WPOS_Y_OFFSET,
    QUNIFORM_FB_WPOS_Y_SCALE, QUNIFORM_TEXTURE_ARRAY_SIZE, QUNIFORM_TEXTURE_DEPTH, QUNIFORM_TEXTURE_HEIGHT,
    QUNIFORM_TEXTURE_WIDTH, QUNIFORM_TMU_CONFIG_P0, QUNIFORM_TMU_CONFIG_P1, QUNIFORM_UNIFORM,
    QUNIFORM_VIEWPORT_X_SCALE, QUNIFORM_VIEWPORT_Y_SCALE, QUNIFORM_VIEWPORT_Z_OFFSET, QUNIFORM_VIEWPORT_Z_SCALE,
};
use super::sim::{self, Env, FsInputs, Inputs, Outcome, Sampler, Texture, Varying, VsInputs, LANES};
use crate::shaderc::online::ir::{Module, Op, TexDim, TexLod};
use crate::shaderc::online::sema::Stage;
use crate::shaderc::online::texref::{Level, MinFilter};
use crate::shaderc::online::{lower, store};
use std::boxed::Box;
use std::collections::BTreeMap;
use std::format;
use std::string::{String, ToString};
use std::vec;
use std::vec::Vec;

/// `(file name, source)` of the sources of one `tools/shaders` directory
/// (`dir` ends in `/`, or is empty for the loose files at the top).
macro_rules! sources {
    ($dir:literal; $($f:literal),* $(,)?) => {
        &[$(($f, include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/shaders/", $dir, $f)))),*]
    };
}

/// The sources, in the directory's sorted order.
static ES100: &[(&str, &str)] = sources![
    "es100/";
    "absadd.frag",
    "affectand.frag",
    "anyall.vert",
    "attrib.frag",
    "attrib_hole_float.vert",
    "attrib_max_vec4.vert",
    "boolany.frag",
    "branchlane.frag",
    "branchquad.frag",
    "clipping.frag",
    "clipping.vert",
    "cond_complex.frag",
    "cond_sequence.frag",
    "contloops.frag",
    "cubebias.frag",
    "cubebias.vert",
    "cubelod.frag",
    "cubelod.vert",
    "cubemap.vert",
    "databreak.frag",
    "deadloop.frag",
    "deriv.frag",
    "deriv.vert",
    "discard_dynloop.frag",
    "discard_dynloop.vert",
    "dynidx.frag",
    "dynidx.vert",
    "dynidx_loop.frag",
    "dynvec4.frag",
    "equirect.frag",
    "f2b.frag",
    "fmods.frag",
    "fmods2.frag",
    "fragcentre.frag",
    "fragcoord.frag",
    "fragz.frag",
    "frontfacing.frag",
    "hdrcolor.frag",
    "hdrquad.vert",
    "hdrscale.frag",
    "hello.frag",
    "hello.vert",
    "idiv.frag",
    "ifelse_loop.frag",
    "ifloop.frag",
    "ifoutput.frag",
    "ifoutput2.frag",
    "immneg.frag",
    "immneg.vert",
    "int_compare.frag",
    "int_div.frag",
    "intlit.frag",
    "ioswap.vert",
    "light_amount.frag",
    "light_amount_0_16.vert",
    "light_amount_2_2.vert",
    "loop_full.frag",
    "loop_nested.dowhile.frag",
    "loop_nested.frag",
    "loop_nested.vert",
    "loop_nested.while.frag",
    "loop_uniform.dowhile.vert",
    "loop_uniform.frag",
    "loopcarry.frag",
    "matattr.frag",
    "matattr2.vert",
    "matattr3.vert",
    "matattr4.vert",
    "matdyn.frag",
    "matvary.frag",
    "matvary.vert",
    "mvstruct.vert",
    "partial_varying.frag",
    "partial_varying.vert",
    "pointcoord.frag",
    "pointcoordvary.frag",
    "psiz_if.vert",
    "rasterization.frag",
    "rasterization.vert",
    "ret_dynloop_never.frag",
    "ret_nested.frag",
    "ret_nested.vert",
    "selects.frag",
    "sharedint.frag",
    "sharedint.vert",
    "swizzled_broadcast.frag",
    "texbranch.frag",
    "texfill.vert",
    "textured.frag",
    "textured.vert",
    "tmparr_const.frag",
    "tmparr_const.vert",
    "transc.frag",
    "tri_discard.frag",
    "tri_discard.vert",
    "tri_fragcoord.frag",
    "tri_fragcoord.vert",
    "tri_if.frag",
    "tri_if.vert",
    "tri_int.frag",
    "tri_int.vert",
    "unibranch.frag",
    "uniform_idx.frag",
    "uniidx_a.frag",
    "uniidx_b.frag",
    "uniidx_c.frag",
    "uniidx_d.frag",
    "uniidx_e.vert",
    "uniidx_f.vert",
    "uniidx_g.frag",
    "uniidx_h.frag",
    "uniidx_i.frag",
    "uniidx_j.frag",
    "utri.frag",
    "utri.vert",
    "va.frag",
    "va_vec2.vert",
    "va_vec4x2.vert",
    "varyings15.frag",
    "varyings15.vert",
    "vrcp.vert",
    "vtex_struct_sampler.vert",
    "vtex_struct_sampler_array.vert",
    "vtransc.frag",
    "xlink.frag",
    "xlink.vert",
];

/// `tools/shaders/gl_test_es100`: the ES 1.00 twins of gl_test's shaders, sorted.
static GL_TEST_ES100: &[(&str, &str)] = sources![
    "gl_test_es100/";
    "armselect.frag",
    "color_uv.frag",
    "color_uv.vert",
    "compact.frag",
    "interp_color.frag",
    "mvp.vert",
    "mvp_color.vert",
    "passthrough.vert",
    "points.vert",
    "texture.frag",
    "texture2.frag",
    "uniform_color.frag",
    "uv.vert",
    "zorder.vert",
];

/// `tools/shaders/gltest`: the gl_test and board-probe shaders (some are ES 3.00 and are refused), sorted.
static GLTEST: &[(&str, &str)] = sources![
    "gltest/";
    "blendonce_fs.frag",
    "dep1_fs.frag",
    "dep2_fs.frag",
    "dep4_fs.frag",
    "deqp_position.vert",
    "es3trunc_fs.frag",
    "es3trunc_vary_fs.frag",
    "es3trunc_vs.vert",
    "es3trunc_vsc.vert",
    "f1_edge_fs.frag",
    "f1_fs.frag",
    "f1_mid_fs.frag",
    "f3_fs.frag",
    "f3_lit_fs.frag",
    "f3_var_fs.frag",
    "f3_var_vs.vert",
    "fma_fs_ctrl.frag",
    "fma_fs_fused.frag",
    "fma_fs_varying.frag",
    "fma_vs_ctrl.vert",
    "fma_vs_fused.vert",
    "fsat_fs_ctrl.frag",
    "fsat_fs_folded.frag",
    "fsat_vs_ctrl.vert",
    "fsat_vs_folded.vert",
    "fshp_vs.vert",
    "intrt.frag",
    "isamp.frag",
    "kc_fadd_fs.frag",
    "kc_fmul_fs.frag",
    "kc_maxmul_fs.frag",
    "kc_maxneg_fs.frag",
    "kc_mulneg_fs.frag",
    "mulone_fs_half.frag",
    "mulone_fs_mods.frag",
    "mulone_fs_one.frag",
    "position_only.vert",
    "position_only2.vert",
    "predif_fs_coord.frag",
    "predif_fs_uni.frag",
    "predif_vs_uni.vert",
    "predlive_fs_coord.frag",
    "predlive_fs_uni.frag",
    "psizvpm_plain_vs.vert",
    "psizvpm_word_vs.vert",
    "texcube_fs.frag",
    "texcube_vs.vert",
    "texcubemip_vs.vert",
    "texcubenorm_fs.frag",
    "tsf_fs.frag",
    "tsf_vs.vert",
    "uniform_color.frag",
    "unipad_fs.frag",
    "unipad_vs.vert",
    "usamp.frag",
    "varyfma_fs.frag",
    "varyfma_vs.vert",
    "vsunits_fs.frag",
    "vtl_fs.frag",
    "vtl_vs.vert",
    "vtl_vs_const.vert",
    "vtl_vs_grid.vert",
    "vtxgrid_vs.vert",
];

/// `tools/shaders/femtovg`: femtovg 0.26.0's shader variants (each .frag pairs with main.vert), sorted.
static FEMTOVG: &[(&str, &str)] = sources![
    "femtovg/";
    "fill_color.frag",
    "fill_color.glyph.frag",
    "fill_color.glyph.nodiscard.frag",
    "fill_color.nodiscard.frag",
    "fill_color.nodiscard.noscissor.frag",
    "fill_gradient.frag",
    "fill_gradient.glyph.frag",
    "fill_gradient.glyph.nodiscard.frag",
    "fill_gradient.nodiscard.frag",
    "fill_gradient.nodiscard.noscissor.frag",
    "fill_gradient_conic.frag",
    "fill_gradient_conic.glyph.frag",
    "fill_gradient_conic.glyph.nodiscard.frag",
    "fill_gradient_conic.nodiscard.frag",
    "fill_gradient_conic.nodiscard.noscissor.frag",
    "fill_image.frag",
    "fill_image.glyph.frag",
    "fill_image.glyph.nodiscard.frag",
    "fill_image.nodiscard.frag",
    "fill_image.nodiscard.noscissor.frag",
    "fill_image_gradient.frag",
    "fill_image_gradient.glyph.frag",
    "fill_image_gradient.glyph.nodiscard.frag",
    "fill_image_gradient.nodiscard.frag",
    "fill_image_gradient.nodiscard.noscissor.frag",
    "fill_image_gradient_conic.frag",
    "fill_image_gradient_conic.glyph.frag",
    "fill_image_gradient_conic.glyph.nodiscard.frag",
    "fill_image_gradient_conic.nodiscard.frag",
    "fill_image_gradient_conic.nodiscard.noscissor.frag",
    "filter_image.frag",
    "filter_image.nodiscard.frag",
    "main.vert",
    "stencil.frag",
    "stencil.nodiscard.frag",
    "texture_copy_unclipped.frag",
];

/// the loose `.vert` and `.frag` files at the top of `tools/shaders`, sorted.
static LOOSE: &[(&str, &str)] = sources![
    "";
    "armselect.frag",
    "color_uv.frag",
    "color_uv.vert",
    "compact.frag",
    "fvgtest_cover.frag",
    "fvgtest_discard.frag",
    "fvgtest_discard2.frag",
    "fvgtest_ftcoord.frag",
    "fvgtest_green.frag",
    "fvgtest_inner.frag",
    "interp_color.frag",
    "mvp.vert",
    "mvp_color.vert",
    "passthrough.vert",
    "points.vert",
    "texture.frag",
    "texture2.frag",
    "uniform_color.frag",
    "uv.vert",
    "zorder.vert",
];

/// Programs either back end refuses (key: the source) and stages an
/// interpreter cannot run at either generation (key: `<source> <stage>`),
/// with the reason. Exact: see the module's header.
const ES3_SKIP: &str = "{}: GLSL ES 3.00 source; this differential is of the ES 1.00 programs";

const SKIPS: &[(&str, &str)] = &[
    (
        "gltest/es3trunc_fs.frag",
        "gltest/es3trunc_fs.frag: GLSL ES 3.00 source; this differential is of the ES 1.00 programs",
    ),
    (
        "gltest/es3trunc_vary_fs.frag",
        "gltest/es3trunc_vary_fs.frag: GLSL ES 3.00 source; this differential is of the ES 1.00 programs",
    ),
    (
        "gltest/es3trunc_vs.vert",
        "gltest/es3trunc_vs.vert: GLSL ES 3.00 source; this differential is of the ES 1.00 programs",
    ),
    (
        "gltest/es3trunc_vsc.vert",
        "gltest/es3trunc_vsc.vert: GLSL ES 3.00 source; this differential is of the ES 1.00 programs",
    ),
    ("gltest/intrt.frag", "gltest/intrt.frag: GLSL ES 3.00 source; this differential is of the ES 1.00 programs"),
    ("gltest/isamp.frag", "gltest/isamp.frag: GLSL ES 3.00 source; this differential is of the ES 1.00 programs"),
    ("gltest/usamp.frag", "gltest/usamp.frag: GLSL ES 3.00 source; this differential is of the ES 1.00 programs"),
    ("loose/armselect.frag", "front end loose/armselect.frag: desktop GLSL source; this is an OpenGL ES compiler"),
    ("loose/color_uv.frag", "front end loose/color_uv.frag: desktop GLSL source; this is an OpenGL ES compiler"),
    ("loose/color_uv.vert", "front end loose/color_uv.vert: desktop GLSL source; this is an OpenGL ES compiler"),
    ("loose/compact.frag", "front end loose/compact.frag: desktop GLSL source; this is an OpenGL ES compiler"),
    (
        "loose/interp_color.frag",
        "front end loose/interp_color.frag: desktop GLSL source; this is an OpenGL ES compiler",
    ),
    ("loose/mvp.vert", "front end loose/mvp.vert: desktop GLSL source; this is an OpenGL ES compiler"),
    ("loose/mvp_color.vert", "front end loose/mvp_color.vert: desktop GLSL source; this is an OpenGL ES compiler"),
    ("loose/passthrough.vert", "front end loose/passthrough.vert: desktop GLSL source; this is an OpenGL ES compiler"),
    ("loose/points.vert", "front end loose/points.vert: desktop GLSL source; this is an OpenGL ES compiler"),
    ("loose/texture.frag", "front end loose/texture.frag: desktop GLSL source; this is an OpenGL ES compiler"),
    ("loose/texture2.frag", "front end loose/texture2.frag: desktop GLSL source; this is an OpenGL ES compiler"),
    (
        "loose/uniform_color.frag",
        "front end loose/uniform_color.frag: desktop GLSL source; this is an OpenGL ES compiler",
    ),
    ("loose/uv.vert", "front end loose/uv.vert: desktop GLSL source; this is an OpenGL ES compiler"),
    ("loose/zorder.vert", "front end loose/zorder.vert: desktop GLSL source; this is an OpenGL ES compiler"),
];

/// Stages whose 4.2 and 7.1 outputs differ (key: `<source> <stage>`),
/// with the reason. Exact: see the module's header.
const EXPECTED_MISMATCHES: &[(&str, &str)] = &[];

/// The control: every stage is run a second time with the 7.1 run's
/// inputs moved (every float uniform dword, every attribute word and every
/// varying's C coefficient; ints and bools, which index and count, stay)
/// and the 4.2 run's not, so the comparison must report a difference.
/// Listed here are the agreeing stages for which it does not, with the
/// reason: their outputs do not depend on what moved. Exact like the
/// others, so the set of stages whose agreement means little stays known.
const CONTROL_BLIND: &[(&str, &str)] = &[
    ("es100/affectand.frag fs", "reads only the bool uniform u_false, which the control keeps"),
    (
        "es100/boolany.frag fs",
        "the bool uniforms stay, and any() and all() of u_v > 0.5 come out the same for both blocks",
    ),
    ("es100/cond_complex.frag fs", "in0 = 3 v_color.r is >= 1 in every lane on both inputs: the same arm"),
    ("es100/cond_sequence.frag fs", "in0 = 3 v_color.r is >= 1 in every lane on both inputs: the same arm"),
    ("es100/cubebias.frag fs", "the control's move of v_dir stays inside the same texel of the same cube face"),
    (
        "es100/deriv.frag fs",
        "derivatives of varyings linear in x and y, and of gl_FragCoord: constants, whatever the offset",
    ),
    ("es100/hello.frag fs", "a constant colour"),
    ("es100/hello.vert fs", "the stub FS of a VS with no varyings writes a constant vec4(0.0)"),
    ("es100/intlit.frag fs", "only int(u_in.x) == 2 reads an input, and u_in.x is below 2 in both blocks"),
    ("es100/tmparr_const.frag fs", "reads only the int uniforms ui_zero .. ui_three, which the control keeps"),
    (
        "es100/varyings15.frag fs",
        "compares the varyings with fixed values neither input matches: the same `bad` in both runs",
    ),
    (
        "femtovg/fill_color.glyph.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_color.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_gradient.glyph.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_gradient.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_gradient_conic.glyph.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_gradient_conic.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_image.glyph.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_image.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_image_gradient.glyph.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_image_gradient.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "femtovg/fill_image_gradient_conic.frag fs",
        "the control does not run: 7.1 does not run: the control's coordinate falls on a texel edge (instruction 61 mul, coordinate 0.8571136), which sim71 refuses as ambiguous",
    ),
    (
        "femtovg/fill_image_gradient_conic.glyph.frag fs",
        "the control does not run: 7.1 does not run: the control's coordinate falls on a texel edge (instruction 61 mul, coordinate 0.8571136), which sim71 refuses as ambiguous",
    ),
    (
        "femtovg/fill_image_gradient_conic.glyph.nodiscard.frag fs",
        "the control does not run: 7.1 does not run: the control's coordinate falls on a texel edge (instruction 56 mul, coordinate 0.8571136), which sim71 refuses as ambiguous",
    ),
    (
        "femtovg/fill_image_gradient_conic.nodiscard.frag fs",
        "the control does not run: 7.1 does not run: the control's coordinate falls on a texel edge (instruction 61 mul, coordinate 0.8571136), which sim71 refuses as ambiguous",
    ),
    (
        "femtovg/fill_image_gradient_conic.nodiscard.noscissor.frag fs",
        "the control does not run: 7.1 does not run: the control's coordinate falls on a texel edge (instruction 61 mul, coordinate 0.8571136), which sim71 refuses as ambiguous",
    ),
    (
        "femtovg/stencil.nodiscard.frag fs",
        "the NO_DISCARD variant: its output does not move with the control's inputs (the uniform array frag[] selects the paint, and the control's pattern shift leaves the branch taken and the output the same)",
    ),
    (
        "gl_test_es100/mvp.vert fs",
        "the stub FS of a VS with no varyings writes a constant vec4(0.0)",
    ),
    (
        "gltest/deqp_position.vert fs",
        "the stub FS of a VS with no varyings writes a constant vec4(0.0)",
    ),
    (
        "gltest/f1_edge_fs.frag fs",
        "constant-folded arithmetic and a look-up at a constant coordinate: no input reaches the output",
    ),
    (
        "gltest/f1_fs.frag fs",
        "constant-folded arithmetic and a look-up at a constant coordinate: no input reaches the output",
    ),
    (
        "gltest/f1_mid_fs.frag fs",
        "constant-folded arithmetic and a look-up at a constant coordinate: no input reaches the output",
    ),
    (
        "gltest/f3_fs.frag fs",
        "constants only: gl_FragColor is a folded constant",
    ),
    (
        "gltest/f3_lit_fs.frag fs",
        "constants only: gl_FragColor is a folded constant",
    ),
    (
        "gltest/fshp_vs.vert fs",
        "the stub FS of a VS with no varyings writes a constant vec4(0.0)",
    ),
    (
        "gltest/position_only.vert fs",
        "the stub FS of a VS with no varyings writes a constant vec4(0.0)",
    ),
    (
        "gltest/position_only2.vert fs",
        "the stub FS of a VS with no varyings writes a constant vec4(0.0)",
    ),
    (
        "gltest/texcube_fs.frag fs",
        "the control's move of v_dir stays inside the same texel of the same cube face",
    ),
    (
        "gltest/texcubenorm_fs.frag fs",
        "the control's move of v_dir stays inside the same texel of the same cube face",
    ),
    (
        "gltest/unipad_vs.vert fs",
        "the stub FS of a VS with no varyings writes a constant vec4(0.0)",
    ),
    (
        "loose/fvgtest_green.frag fs",
        "a constant colour",
    ),
];

const TARGET42: Target = Target { vpm_size: 65536 };

/// The viewport and window transforms the uniform stream carries (a 64x64
/// target in 1/64 pixel, y down; depth range 0..1; a window's y flips).
const X_SCALE: f32 = 2048.0;
const Y_SCALE: f32 = -2048.0;
const Z_SCALE: f32 = 0.5;
const Z_OFFSET: f32 = 0.5;
const WPOS_SCALE: f32 = -1.0;
const WPOS_OFFSET: f32 = 64.0;
const PNTC_SCALE: f32 = -1.0;
const PNTC_OFFSET: f32 = 1.0;

/// Varyings supplied past the FS's input slots: the implicit point and
/// line varyings (gl_PointCoord's two), which a fragment shader reads with
/// LDVARY beyond the slots it declares; an LDVARY past what is supplied
/// stops the interpreter.
const IMPLICIT_VARYINGS: usize = 2;

/// Texture units bound (every binding the front end hands out).
const TEXTURE_UNITS: u32 = 16;

// ---------------------------------------------------------------------------
// es100_ref71.sh's stubs (tests/v3d71_online_corpus.rs's, line for line).
// ---------------------------------------------------------------------------

/// The text without `//` and `/* */` comments, each a space.
fn strip_comments(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            out.push(' ');
        } else if b[i..].starts_with(b"/*") {
            i += 2;
            while i < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(b.len());
            out.push(' ');
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

const TYPES: [&str; 7] = ["float", "vec2", "vec3", "vec4", "mat2", "mat3", "mat4"];

/// `(type, name, array size or 0)` of the source's varying declarations.
fn varyings(text: &str) -> Vec<(String, String, usize)> {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let t = strip_comments(text);
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = t[from..].find("varying").map(|k| k + from) {
        from = k + 7;
        if (k > 0 && word(b[k - 1])) || b.get(k + 7).is_some_and(|&c| word(c)) {
            continue;
        }
        let Some(end) = t[k + 7..].find(';').map(|e| e + k + 7) else { break };
        let mut toks = t[k + 7..end].split_whitespace().peekable();
        if toks.peek().is_some_and(|p| ["highp", "mediump", "lowp"].contains(p)) {
            toks.next();
        }
        let Some(ty) = toks.next().filter(|ty| TYPES.contains(ty)) else { continue };
        let decls: String = toks.collect::<Vec<_>>().join(" ");
        for d in decls.split(',') {
            let d = d.trim();
            let (name, n) = match d.split_once('[') {
                Some((n, rest)) => (n.trim(), rest.trim_end_matches(']').trim().parse().expect("a varying array size")),
                None => (d, 0),
            };
            assert!(!name.is_empty() && name.bytes().all(word), "cannot read varying `{d}`");
            out.push((ty.to_string(), name.to_string(), n));
        }
        from = end;
    }
    out
}

/// An expression of type `ty` from the uniform vec4 `src`.
fn value(ty: &str, src: &str) -> String {
    match ty {
        "float" => format!("{src}.x"),
        "vec4" => format!("vec4({src})"),
        t if t.starts_with("vec") => format!("{src}.{}", &"xyzw"[..t[3..].parse::<usize>().unwrap()]),
        t => {
            let n: usize = t[3..].parse().unwrap();
            format!("{t}({})", vec![format!("{src}.x"); n * n].join(", "))
        }
    }
}

fn stub_vs(list: &[(String, String, usize)]) -> String {
    let mut s: Vec<String> =
        ["#version 100", "precision highp float;", "attribute vec4 a_pos;", "uniform vec4 u_stub;"]
            .map(String::from)
            .to_vec();
    let mut body = vec!["    gl_Position = a_pos;".to_string()];
    for (ty, name, arr) in list {
        s.push(format!("varying {ty} {name}{};", if *arr > 0 { format!("[{arr}]") } else { String::new() }));
        for i in 0..(*arr).max(1) {
            let e = if *arr > 0 { format!("{name}[{i}]") } else { name.clone() };
            body.push(format!("    {e} = {};", value(ty, "u_stub")));
        }
    }
    s.extend(["void main()".to_string(), "{".to_string()]);
    s.extend(body);
    s.extend(["}".to_string(), String::new()]);
    s.join("\n")
}

fn stub_fs(list: &[(String, String, usize)]) -> String {
    let mut s: Vec<String> = vec!["#version 100".into(), "precision highp float;".into()];
    let mut body = vec!["    vec4 c = vec4(0.0);".to_string()];
    for (ty, name, arr) in list {
        s.push(format!("varying {ty} {name}{};", if *arr > 0 { format!("[{arr}]") } else { String::new() }));
        for i in 0..(*arr).max(1) {
            let e = if *arr > 0 { format!("{name}[{i}]") } else { name.clone() };
            if let Some(n) = ty.strip_prefix("mat") {
                let n: usize = n.parse().unwrap();
                let sum: Vec<String> = (0..n).map(|k| format!("{e}[{k}].x")).collect();
                body.push(format!("    c.x += {};", sum.join(" + ")));
            } else if ty == "float" {
                body.push(format!("    c += vec4({e});"));
            } else {
                let pad =
                    if ty == "vec4" { String::new() } else { ", 0.0".repeat(4 - ty[3..].parse::<usize>().unwrap()) };
                body.push(format!("    c += vec4({e}{pad});"));
            }
        }
    }
    s.extend(["void main()".to_string(), "{".to_string()]);
    s.extend(body);
    s.extend(["    gl_FragColor = c;".to_string(), "}".to_string(), String::new()]);
    s.join("\n")
}

// ---------------------------------------------------------------------------
// The inputs.
// ---------------------------------------------------------------------------

/// A positive value exact in f16, 1/32 .. 63/32, from `k` (32 values,
/// distinct for 32 consecutive `k`, on both sides of 1/2 and 1): an odd
/// number of 32nds, so never a multiple of 1/2 (a NEAREST_MIPMAP_NEAREST
/// level boundary, which the interpreter refuses as a level of detail) nor
/// a texel edge of the 7-, 3- or 1-texel levels as a coordinate.
fn pattern(k: usize) -> f32 {
    (1 + 2 * ((k * 7 + 3) % 32)) as f32 / 32.0
}

/// The value of an int uniform: the dEQP index and loop-count uniforms
/// take their dEQP values (es2fShaderIndexingTests.cpp and
/// es2fShaderLoopTests.cpp: `ui_zero` .. `ui_four` = 0 .. 4), and matdyn's
/// `u_i` 1, its only in-range index (tests/v3d_sim_mesa.rs
/// UNIFORM_OVERRIDES says why); any other int 1 + k % 3.
fn int_value(name: &str, k: usize) -> u32 {
    match name {
        "ui_zero" => 0,
        "ui_one" | "u_i" => 1,
        "ui_two" => 2,
        "ui_three" => 3,
        "ui_four" => 4,
        _ => 1 + (k % 3) as u32,
    }
}

/// The stage's uniform block: [`pattern`] floats (of `k + 1` for the
/// control), then the int and bool uniforms' dwords overwritten (the same
/// in the control: they index and count loops).
fn block(m: &Module<'_>, control: bool) -> Vec<u32> {
    let k0 = usize::from(control);
    let mut b: Vec<u32> = (0..m.uniform_dwords as usize).map(|k| pattern(k + k0).to_bits()).collect();
    for d in m.uniforms.iter().filter(|d| d.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK) {
        // GL_INT, GL_INT_VEC2..4, GL_BOOL, GL_BOOL_VEC2..4
        let (rows, boolean) = match d.gl_type {
            0x1404 => (1, false),
            0x8b53..=0x8b55 => ((d.gl_type - 0x8b53 + 2) as usize, false),
            0x8b56..=0x8b59 => ((d.gl_type - 0x8b56 + 1) as usize, true),
            _ => continue,
        };
        let len = usize::from(d.array_len.max(1));
        let stride = usize::from(d.dwords) / len;
        for e in 0..len {
            for r in 0..rows {
                let k = usize::from(d.dword_off) + e * stride + r;
                if let Some(x) = b.get_mut(k) {
                    // a bool true is ~0 (any non-zero dword reads true)
                    *x = match (boolean, k % 2) {
                        (true, 0) => u32::MAX,
                        (true, _) => 0,
                        (false, _) => int_value(d.name, k),
                    };
                }
            }
        }
    }
    b
}

/// `GL_SAMPLER_CUBE`.
const GL_SAMPLER_CUBE: u16 = 0x8b60;

/// The texture of unit `u`: 7x7, 3x3, 1x1 and NEAREST_MIPMAP_NEAREST when
/// `mip`, else 7x7 and NEAREST; texel (i, j) of image f of level k
/// ((8u + i + 1)/128 + j/1024, (8u + j + 1)/128, (64u + i + 7j)/1024 + f/8,
/// 1 - (64u + i + 7j)/2048 - k/16): tests/v3d_sim_mesa.rs's mipmapped
/// harness texture, every channel naming the unit, exact in f16, odd at
/// every level (0.5 is a texel centre).
fn texture(u: u32, cube: bool, mip: bool) -> Texture {
    let level = |k: u32| {
        let n = (7u32 >> k).max(1);
        let face = |f: u32| -> Vec<[f32; 4]> {
            (0..n * n)
                .map(|x| {
                    let (i, j) = ((x % n) as f32, (x / n) as f32);
                    let (u, f, k) = (u as f32, f as f32, k as f32);
                    let t = i + 7.0 * j;
                    [
                        (8.0 * u + i + 1.0) / 128.0 + j / 1024.0,
                        (8.0 * u + j + 1.0) / 128.0,
                        (64.0 * u + t) / 1024.0 + f / 8.0,
                        1.0 - (64.0 * u + t) / 2048.0 - k / 16.0,
                    ]
                })
                .collect()
        };
        Level { width: n, height: n, faces: (0..if cube { 6 } else { 1 }).map(face).collect() }
    };
    let (min, levels) = if mip { (MinFilter::NearestMipmapNearest, 3) } else { (MinFilter::Nearest, 1) };
    Texture { cube, min, levels: (0..levels).map(level).collect() }
}

/// The 16 textures: a cube on every unit a samplerCube uniform of either
/// stage names or a cube look-up reads; mipmapped on every unit that is
/// read, and only with an explicit level (the interpreter does not model
/// an implicit level of detail on more than one level), so the level a
/// look-up asks for shows in the value.
fn textures(ms: [&Module<'_>; 2]) -> Vec<Texture> {
    let mut cube = [false; TEXTURE_UNITS as usize];
    // (read, every read names its level)
    let mut mip = [(false, true); TEXTURE_UNITS as usize];
    for m in ms {
        for d in m.uniforms.iter().filter(|d| d.dword_off == crate::shaderc::SHC_UNIFORM_NO_BLOCK) {
            for k in 0..d.array_len.max(1) {
                if let Some(c) = cube.get_mut(usize::from(d.binding + k)) {
                    *c |= d.gl_type == GL_SAMPLER_CUBE;
                }
            }
        }
        for op in m.instrs {
            if let Op::Tex { dim, sampler, lod, .. } = *op {
                let u = usize::from(sampler);
                if let (Some(c), Some(m)) = (cube.get_mut(u), mip.get_mut(u)) {
                    *c |= dim == TexDim::Cube;
                    *m = (true, m.1 && matches!(lod, TexLod::Lod(_) | TexLod::Fetch(_)));
                }
            }
        }
    }
    (0..TEXTURE_UNITS as usize).map(|u| texture(u as u32, cube[u], mip[u] == (true, true))).collect()
}

/// A stage's uniform stream resolved to values, or the contents it cannot
/// resolve.
fn stream(us: &[(u32, u32)], block: &[u32], texs: &[Texture]) -> Result<Vec<u32>, String> {
    us.iter()
        .map(|&(c, d)| {
            let unit = |d: u32| texs.get(d as usize).map(|t| t.size()).ok_or(format!("texture unit {d}"));
            Ok(match c {
                QUNIFORM_CONSTANT => d,
                QUNIFORM_UNIFORM => *block.get(d as usize).ok_or(format!("uniform dword {d} past the block"))?,
                QUNIFORM_VIEWPORT_X_SCALE => X_SCALE.to_bits(),
                QUNIFORM_VIEWPORT_Y_SCALE => Y_SCALE.to_bits(),
                QUNIFORM_VIEWPORT_Z_OFFSET => Z_OFFSET.to_bits(),
                QUNIFORM_VIEWPORT_Z_SCALE => Z_SCALE.to_bits(),
                // the binding in bits 24..31, the unit the interpreter reads
                QUNIFORM_TMU_CONFIG_P0 | QUNIFORM_TMU_CONFIG_P1 if d >> 24 < TEXTURE_UNITS => d,
                QUNIFORM_TEXTURE_WIDTH => unit(d)?.0,
                QUNIFORM_TEXTURE_HEIGHT => unit(d)?.1,
                QUNIFORM_TEXTURE_DEPTH | QUNIFORM_TEXTURE_ARRAY_SIZE => 1,
                QUNIFORM_FB_WPOS_Y_SCALE => WPOS_SCALE.to_bits(),
                QUNIFORM_FB_WPOS_Y_OFFSET => WPOS_OFFSET.to_bits(),
                QUNIFORM_FB_PNTC_Y_SCALE => PNTC_SCALE.to_bits(),
                QUNIFORM_FB_PNTC_Y_OFFSET => PNTC_OFFSET.to_bits(),
                c => return Err(format!("uniform contents {c} (data {d:#x}) not resolved here")),
            })
        })
        .collect()
}

/// Lane `l`'s pixel: quad l / 4 at (2 (q & 1), 2 (q >> 1)) from (3, 5),
/// lane 4q + i at column i & 1, row i >> 1 (`Env::quads`'s layout); two
/// pixels right and down for the control.
fn pixel(l: usize, control: bool) -> (f32, f32) {
    let (q, i, o) = (l / 4, l % 4, 2 * usize::from(control));
    ((3 + o + 2 * (q & 1) + (i & 1)) as f32 + 0.5, (5 + o + 2 * (q >> 1) + (i >> 1)) as f32 + 0.5)
}

/// The fragment inputs: varying `i` is `(1/4 + i/32 + x/64 + y/128) / W`
/// before the W multiply, with C 1/8 (a flat input's value is C). The
/// control ([`CONTROL_BLIND`]) moves every one: the pixels (and with them
/// the varyings), C to 3/16, Z by 1/128, and the facing of every quad.
fn fs_inputs(n: usize, control: bool) -> FsInputs {
    let c = if control { 0.1875f32 } else { 0.125 };
    let dz = if control { 1.0 / 128.0 } else { 0.0 };
    let w = |l: usize| if l < 8 { 1.0f32 } else { 2.0 };
    let varyings = (0..n)
        .map(|i| Varying {
            vary: core::array::from_fn(|l| {
                let (x, y) = pixel(l, control);
                ((0.25 + i as f32 / 32.0 + x / 64.0 + y / 128.0) / w(l)).to_bits()
            }),
            c: [c.to_bits(); LANES],
        })
        .collect();
    FsInputs {
        w: core::array::from_fn(|l| w(l).to_bits()),
        w_centroid: core::array::from_fn(|l| w(l).to_bits()),
        z: core::array::from_fn(|l| (0.25 + l as f32 / 64.0 + dz).to_bits()),
        x: core::array::from_fn(|l| pixel(l, control).0.to_bits()),
        y: core::array::from_fn(|l| pixel(l, control).1.to_bits()),
        back_facing: core::array::from_fn(|l| ((l / 4) % 2 == 1) != control),
        varyings,
    }
}

/// The VPM input rows of a stage with these attribute sizes: row r, lane
/// l is [`pattern`]`(5r + 3l)`, positive (`5r + 3l + 1` for the control).
fn vs_inputs(sizes: &[u8; 16], control: bool) -> VsInputs {
    let rows: usize = sizes.iter().map(|&s| usize::from(s)).sum();
    let k = |r: usize, l: usize| 5 * r + 3 * l + usize::from(control);
    VsInputs { vpm: (0..rows).map(|r| core::array::from_fn(|l| pattern(k(r, l)).to_bits())).collect() }
}

// ---------------------------------------------------------------------------
// Compiling and running.
// ---------------------------------------------------------------------------

/// One stage as the interpreter needs it.
struct Code {
    words: Vec<u64>,
    uniforms: Vec<(u32, u32)>,
    threads: u32,
}

/// One generation's program.
struct Built {
    fs: Code,
    vs: Code,
    cs: Code,
    input_slots: Vec<(u8, u8)>,
    vs_sizes: [u8; 16],
    cs_sizes: [u8; 16],
}

/// Both generations' programs and the inputs that do not depend on one.
struct Pair {
    b42: Built,
    b71: Built,
    /// The FS's and the VS's (and CS's) uniform blocks: `[0]` the inputs,
    /// `[1]` the control's.
    fs_block: [Vec<u32>; 2],
    vs_block: [Vec<u32>; 2],
    textures: Vec<Texture>,
}

fn code(words: &[u64], us: &[Uniform], threads: u32) -> Code {
    Code { words: words.to_vec(), uniforms: us.iter().map(|u| (u.contents, u.data)).collect(), threads }
}

/// Compile `vs` + `fs` at both generations; `Err` names the refusal.
fn compile(names: [&str; 2], vs: &str, fs: &str) -> Result<Pair, String> {
    // A GLSL ES 3.00 source is refused here, not by the front end: whether
    // the front end takes it depends on the build (report-es2), and the
    // lists below must be the same in every configuration the gate runs.
    for (name, src) in [(names[0], vs), (names[1], fs)] {
        if src.trim_start().starts_with("#version 300 es") {
            return Err(ES3_SKIP.replace("{}", name));
        }
    }
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, name, src) in [(Stage::Vertex, names[0], vs), (Stage::Fragment, names[1], fs)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => m.push(l.module),
                Err(e) => return Err(format!("front end {name}: {}", e.msg)),
            }
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        let p = compile_program(v, f, Prim::Triangles, TARGET42, a).map_err(|e| format!("4.2 refuses: {e:?}"))?;
        let b42 = Built {
            fs: code(p.fs.code.words, p.fs.code.uniforms, p.fs.threads),
            vs: code(p.vs.code.words, p.vs.code.uniforms, p.vs.threads),
            cs: code(p.cs.code.words, p.cs.code.uniforms, p.cs.threads),
            input_slots: p.fs.input_slots.to_vec(),
            vs_sizes: p.vs.vattr_sizes,
            cs_sizes: p.cs.vattr_sizes,
        };
        let p = compile_program71(v, f, Prim::Triangles, TARGET71, a).map_err(|e| format!("7.1 refuses: {e:?}"))?;
        let b71 = Built {
            fs: code(p.fs.code.words, p.fs.code.uniforms, p.fs.threads),
            vs: code(p.vs.code.words, p.vs.code.uniforms, p.vs.threads),
            cs: code(p.cs.code.words, p.cs.code.uniforms, p.cs.threads),
            input_slots: p.fs.input_slots.to_vec(),
            vs_sizes: p.vs.vattr_sizes,
            cs_sizes: p.cs.vattr_sizes,
        };
        Ok(Pair {
            b42,
            b71,
            fs_block: [block(f, false), block(f, true)],
            vs_block: [block(v, false), block(v, true)],
            textures: textures([v, f]),
        })
    })
    .expect("scratch")
}

/// What one stage's comparison came to.
enum Verdict {
    Agree,
    /// An interpreter could not run it (or its stream could not be
    /// resolved): the reason.
    Skip(String),
    Mismatch(String),
}

/// Run one stage at one generation.
fn run(gen: Gen, c: &Code, block: &[u32], texs: &[Texture], inputs: &Inputs) -> Result<Outcome, String> {
    let s = stream(&c.uniforms, block, texs)?;
    let env = Env {
        textures: texs.to_vec(),
        samplers: texs.iter().map(Sampler::of).collect(),
        threads: c.threads,
        quads: true,
        ..Env::default()
    };
    sim::run_gen(gen, &c.words, &s, inputs, &env)
}

/// The FS's TLB writes and flags, as text: equal text is equal output.
fn fs_output(o: &Outcome) -> Vec<String> {
    let mut v: Vec<String> =
        o.tlb.iter().map(|t| format!("tlb config {:x?} values {:x?}", t.config, t.value)).collect();
    v.push(format!("msf {:x?}", o.msf));
    v
}

/// What the agreeing stages compared, so that an agreement is not vacuous.
#[derive(Default, Debug)]
struct Stats {
    /// Defined TLB lanes, and lanes the FS discarded.
    tlb_lanes: usize,
    discarded: usize,
    /// TMU look-ups (at 4.2).
    tmu_lookups: usize,
    /// Defined VPM output words, and screen x/y words 1 apart.
    vpm_words: usize,
    screen_off_by_one: usize,
}

/// Compare the FS's TLB writes and flags.
fn fs_compare(a: &Outcome, b: &Outcome, st: &mut Stats) -> Result<(), String> {
    let (x, y) = (fs_output(a), fs_output(b));
    if x != y {
        return Err(format!("4.2 {x:?}\n      7.1 {y:?}"));
    }
    st.tlb_lanes += a.tlb.iter().map(|t| t.value.iter().filter(|v| v.is_some()).count()).sum::<usize>();
    st.discarded += a.msf.iter().filter(|&&m| m == 0).count();
    st.tmu_lookups += a.tmu_lookups;
    Ok(())
}

/// Compare the VPM output rows; `screen` are the rows of the integer
/// screen x and y, where 7.1's word may be 4.2's plus 1 (and only that).
fn vpm_compare(a: &Outcome, b: &Outcome, screen: [usize; 2], st: &mut Stats) -> Result<(), String> {
    if a.vpm_out.len() != b.vpm_out.len() {
        return Err(format!("{} VPM rows at 4.2, {} at 7.1", a.vpm_out.len(), b.vpm_out.len()));
    }
    let mut off = 0;
    for (r, (x, y)) in a.vpm_out.iter().zip(&b.vpm_out).enumerate() {
        let same = match (x, y) {
            (Some(x), Some(y)) if screen.contains(&r) => x.iter().zip(y).all(|(&p, &q)| {
                // 7.1's round to nearest even less 4.2's floor
                let d = (q as i32).wrapping_sub(p as i32);
                off += usize::from(d == 1);
                d == 0 || d == 1
            }),
            (x, y) => x == y,
        };
        if !same {
            return Err(format!("VPM row {r}: 4.2 {x:x?}, 7.1 {y:x?}"));
        }
    }
    if !a.tlb.is_empty() || !b.tlb.is_empty() {
        return Err("a vertex-pipeline stage wrote the TLB".into());
    }
    st.vpm_words += a.vpm_out.iter().flatten().count() * LANES;
    st.screen_off_by_one += off;
    Ok(())
}

/// Judge the two runs of one stage.
fn judge(
    r42: Result<Outcome, String>,
    r71: Result<Outcome, String>,
    cmp: impl FnOnce(&Outcome, &Outcome) -> Result<(), String>,
) -> Verdict {
    match (r42, r71) {
        (Ok(a), Ok(b)) => match cmp(&a, &b) {
            Ok(()) => Verdict::Agree,
            Err(e) => Verdict::Mismatch(e),
        },
        (Err(a), Ok(_)) => Verdict::Skip(format!("4.2 does not run: {a}")),
        (Ok(_), Err(b)) => Verdict::Skip(format!("7.1 does not run: {b}")),
        (Err(a), Err(b)) => Verdict::Skip(format!("neither runs: 4.2 {a}; 7.1 {b}")),
    }
}

/// The three stages' verdicts. With `control`, the 7.1 run takes the
/// control's inputs ([`CONTROL_BLIND`]); the 4.2 run always takes the inputs.
fn stages(p: &Pair, control: bool, st: &mut Stats) -> [(&'static str, Verdict); 3] {
    let k = usize::from(control);
    let fs = if p.b42.input_slots != p.b71.input_slots {
        Verdict::Mismatch(format!("input slots: 4.2 {:?}, 7.1 {:?}", p.b42.input_slots, p.b71.input_slots))
    } else {
        let n = p.b42.input_slots.len() + IMPLICIT_VARYINGS;
        let i42 = Inputs::Fragment(Box::new(fs_inputs(n, false)));
        let i71 = Inputs::Fragment(Box::new(fs_inputs(n, control)));
        judge(
            run(Gen::V42, &p.b42.fs, &p.fs_block[0], &p.textures, &i42),
            run(Gen::V71, &p.b71.fs, &p.fs_block[k], &p.textures, &i71),
            |a, b| fs_compare(a, b, st),
        )
    };
    let mut vertex = |what: &str, c42: &Code, c71: &Code, s42: &[u8; 16], s71: &[u8; 16], screen: [usize; 2]| {
        if s42 != s71 {
            return Verdict::Mismatch(format!("{what} attribute sizes: 4.2 {s42:?}, 7.1 {s71:?}"));
        }
        judge(
            run(Gen::V42, c42, &p.vs_block[0], &p.textures, &Inputs::Vertex(vs_inputs(s42, false))),
            run(Gen::V71, c71, &p.vs_block[k], &p.textures, &Inputs::Vertex(vs_inputs(s71, control))),
            |a, b| vpm_compare(a, b, screen, st),
        )
    };
    let vs = vertex("vs", &p.b42.vs, &p.b71.vs, &p.b42.vs_sizes, &p.b71.vs_sizes, [0, 1]);
    let cs = vertex("cs", &p.b42.cs, &p.b71.cs, &p.b42.cs_sizes, &p.b71.cs_sizes, [4, 5]);
    [("fs", fs), ("vs", vs), ("cs", cs)]
}

/// The differences between what ran and a list: one line each.
fn exact(what: &str, got: &BTreeMap<String, String>, listed: &[(&str, &str)]) -> Vec<String> {
    let want: BTreeMap<&str, &str> = listed.iter().copied().collect();
    assert_eq!(want.len(), listed.len(), "{what}: a key is listed twice");
    let mut bad = Vec::new();
    for (k, m) in got {
        if !want.contains_key(k.as_str()) {
            bad.push(format!("NEW {what}, not listed: {k}: {m}"));
        }
    }
    for k in want.keys().filter(|k| !got.contains_key(**k)) {
        bad.push(format!("listed {what} that no longer is one (remove it): {k}"));
    }
    bad
}

/// One in-tree set: its sources and the counts pinned for it.
struct Set {
    /// The key prefix of its list entries (`<name>/<file> <stage>`).
    name: &'static str,
    /// Its directory under `tools/shaders` (empty: the loose files at the
    /// top).
    dir: &'static str,
    sources: &'static [(&'static str, &'static str)],
    /// Every `.frag` pairs with `femtovg/main.vert` (femtovg's own pairing,
    /// `main.vert` with each variant) instead of a stub vertex shader.
    paired_with_main_vert: bool,
    pin: Pin,
}

/// What a set's run came to: programs both back ends compile, stages that
/// agree, entries skipped (programs and stages), stages that differ,
/// agreeing stages blind to the control.
#[derive(Debug, PartialEq)]
struct Pin {
    programs: usize,
    agreed: usize,
    skipped: usize,
    differ: usize,
    blind: usize,
}

const fn pin(programs: usize, agreed: usize, skipped: usize, differ: usize, blind: usize) -> Pin {
    Pin { programs, agreed, skipped, differ, blind }
}

/// `femtovg/main.vert`, the vertex shader of every femtovg variant.
fn femtovg_main_vert() -> &'static str {
    FEMTOVG.iter().find(|f| f.0 == "main.vert").expect("femtovg/main.vert is listed").1
}

static SETS: &[Set] = &[
    Set { name: "es100", dir: "es100", sources: ES100, paired_with_main_vert: false, pin: pin(126, 378, 0, 0, 11) },
    Set {
        name: "gl_test_es100",
        dir: "gl_test_es100",
        sources: GL_TEST_ES100,
        paired_with_main_vert: false,
        pin: pin(14, 42, 0, 0, 1),
    },
    Set { name: "gltest", dir: "gltest", sources: GLTEST, paired_with_main_vert: false, pin: pin(56, 168, 7, 0, 12) },
    Set { name: "femtovg", dir: "femtovg", sources: FEMTOVG, paired_with_main_vert: true, pin: pin(35, 105, 0, 0, 16) },
    Set { name: "loose", dir: "", sources: LOOSE, paired_with_main_vert: false, pin: pin(6, 18, 14, 0, 1) },
];

#[test]
fn es100_at_4_2_and_7_1_agree_in_the_interpreters() {
    measure(&SETS[0]);
}

#[test]
fn gl_test_es100_at_4_2_and_7_1_agree_in_the_interpreters() {
    measure(&SETS[1]);
}

#[test]
fn gltest_at_4_2_and_7_1_agree_in_the_interpreters() {
    measure(&SETS[2]);
}

#[test]
fn femtovg_at_4_2_and_7_1_agree_in_the_interpreters() {
    measure(&SETS[3]);
}

#[test]
fn loose_shaders_at_4_2_and_7_1_agree_in_the_interpreters() {
    measure(&SETS[4]);
}

/// Compile, run and compare every program of a set; the lists are checked
/// and the counts compared with the set's pin.
fn measure(set: &Set) {
    let mut skips = BTreeMap::new();
    let mut mismatches = BTreeMap::new();
    let mut agreed = 0usize;
    let mut programs = 0usize;
    let mut st = Stats::default();
    // the control: stages that agree on the inputs and still agree when
    // only the 7.1 run's inputs move
    let mut blind = BTreeMap::new();
    for (file, src) in set.sources {
        if set.paired_with_main_vert && *file == "main.vert" {
            continue;
        }
        let key = format!("{}/{file}", set.name);
        let vary = varyings(src);
        let (vs, fs, names) = if set.paired_with_main_vert {
            (femtovg_main_vert().to_string(), src.to_string(), ["femtovg/main.vert", key.as_str()])
        } else if file.ends_with(".vert") {
            (src.to_string(), stub_fs(&vary), [key.as_str(), "<stub fs>"])
        } else {
            (stub_vs(&vary), src.to_string(), ["<stub vs>", key.as_str()])
        };
        let p = match compile(names, &vs, &fs) {
            Ok(p) => p,
            Err(e) => {
                skips.insert(key, e);
                continue;
            }
        };
        programs += 1;
        let control = stages(&p, true, &mut Stats::default());
        for ((stage, v), (_, c)) in stages(&p, false, &mut st).into_iter().zip(control) {
            let k = format!("{key} {stage}");
            match v {
                Verdict::Agree => {
                    agreed += 1;
                    match c {
                        Verdict::Agree => {
                            blind.insert(k, "agrees on the control's inputs".to_string());
                        }
                        Verdict::Skip(e) => {
                            blind.insert(k, format!("the control does not run: {e}"));
                        }
                        Verdict::Mismatch(_) => {}
                    }
                }
                Verdict::Skip(e) => {
                    skips.insert(k, e);
                }
                Verdict::Mismatch(e) => {
                    mismatches.insert(k, e);
                }
            }
        }
    }
    let got = pin(programs, agreed, skips.len(), mismatches.len(), blind.len());
    std::println!(
        "diff71 {}: {} sources, {programs} compiled, {agreed} stages agree, {} skipped, {} differ",
        set.name,
        set.sources.len(),
        skips.len(),
        mismatches.len()
    );
    for (k, m) in &skips {
        std::println!("  skip {k}: {m}");
    }
    for (k, m) in &mismatches {
        std::println!("  DIFFER {k}: {m}");
    }
    std::println!("diff71 {}: compared {st:?}; {} agreeing stages blind to the control", set.name, blind.len());
    for (k, m) in &blind {
        std::println!("  blind {k}: {m}");
    }
    let mine = |l: &[(&'static str, &'static str)]| -> Vec<(&'static str, &'static str)> {
        l.iter().copied().filter(|(k, _)| k.split('/').next() == Some(set.name)).collect()
    };
    let mut bad = exact("skip", &skips, &mine(SKIPS));
    bad.extend(exact("mismatch", &mismatches, &mine(EXPECTED_MISMATCHES)));
    bad.extend(exact("control-blind stage", &blind, &mine(CONTROL_BLIND)));
    assert!(bad.is_empty(), "diff71's lists are out of date:\n{}", bad.join("\n"));
    assert_eq!(got, set.pin, "{}: the pinned counts moved", set.name);
    assert!(agreed > 0, "{}: nothing was compared", set.name);
    // the es100 set has a program of every kind; the others only need to
    // compare something
    if set.name == "es100" {
        assert!(
            st.tlb_lanes > 0 && st.discarded > 0 && st.tmu_lookups > 0 && st.vpm_words > 0 && st.screen_off_by_one > 0,
            "a part of the comparison compared nothing: {st:?}"
        );
    }
}

/// The lists are exact: [`exact`] names an entry that is not listed and a
/// listed one that no longer is one.
#[test]
fn a_list_that_is_not_exact_is_named() {
    let got: BTreeMap<String, String> = [("a fs".to_string(), "why".to_string())].into_iter().collect();
    assert!(exact("skip", &got, &[("a fs", "why")]).is_empty());
    assert_eq!(exact("skip", &got, &[]), ["NEW skip, not listed: a fs: why"]);
    assert_eq!(
        exact("skip", &BTreeMap::new(), &[("a fs", "why")]),
        ["listed skip that no longer is one (remove it): a fs"]
    );
}

/// Each set's embedded list is its directory's `.vert` and `.frag` files,
/// sorted (one test per set).
fn list_is_the_directory(set: &Set) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders").join(set.dir);
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("the set's directory is readable")
        .map(|e| e.expect("a directory entry"))
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".vert") || f.ends_with(".frag"))
        .collect();
    files.sort();
    let embedded: Vec<&str> = set.sources.iter().map(|e| e.0).collect();
    assert_eq!(embedded, files, "add or remove the source in diff71.rs's list of {}", set.name);
}

#[test]
fn the_embedded_list_is_the_directory() {
    list_is_the_directory(&SETS[0]);
}

#[test]
fn the_gl_test_es100_list_is_the_directory() {
    list_is_the_directory(&SETS[1]);
}

#[test]
fn the_gltest_list_is_the_directory() {
    list_is_the_directory(&SETS[2]);
}

#[test]
fn the_femtovg_list_is_the_directory() {
    list_is_the_directory(&SETS[3]);
}

#[test]
fn the_loose_list_is_the_directory() {
    list_is_the_directory(&SETS[4]);
}
