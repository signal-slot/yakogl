// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The STM32MP257F's Mesa reference table against the i.MX8MP's.
//!
//! `tests/reference/vivante_blobs_st.rs` is the i.MX8MP table's rows compiled
//! by the same etna_shc for the ST ids (model 0x8000, revision 0x6205,
//! product 0x80003, eco 0, customer 0x15; `tools/vivante/gen_shader_table.py
//! --st`). The one ISA difference between the two hwdb rows is the ST row's
//! `SH_NO_ONECONST_LIMIT` (an instruction may read two different uniforms;
//! docs/notes/pre-study-stm32mp257f-dk.md, section 3), so the rows and the
//! instruction words that differ are the size of that relaxation. The
//! i.MX8MP row of every differing program is its one-constant form, which
//! tests/constant_registers.rs holds the whole table to, so the stricter
//! output our compiler emits for the i.MX8MP stays valid on the ST part.
//! This test pins those counts; when the generator moves them, take the new
//! numbers from the failing assertion.
//!
//! The ST table is not compiled into any build: this file includes it with
//! `#[path]`, and its `use` lines reach the types through the crate. The
//! file is a `tests/gc_*` test so that tools/check.sh's scope map sends it
//! to the Vivante rows (an unmapped `tests/` path selects every row).

#![cfg(all(feature = "gpu-vivante", not(feature = "no-shader-table")))]

#[path = "reference/vivante_blobs_st.rs"]
mod st;

use yakogl::shaderc::mesa_reference::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::ShcBlob;

/// The rows of the i.MX8MP table (`SHC_BLOB_TABLE` of vivante_blobs.rs), which
/// the ST table repeats one for one.
const ROWS: usize = 234;

/// The rows whose instruction words differ between the two tables, and the
/// words that differ (positions over the common length plus the words one
/// program has beyond the other's: `words_that_differ` in the generator).
/// The ST program is never longer; `SHORTER_ON_ST` of them are shorter, and
/// the two tables hold `INSTRUCTIONS` instructions in all.
const DIFFERING_ROWS: usize = 91;
const DIFFERING_WORDS: usize = 5853;
const SHORTER_ON_ST: usize = 28;
const INSTRUCTIONS: (usize, usize) = (5540, 5415);

/// The differing rows by name, in table order.
const DIFFERING: &[&str] = &[
    "es300/cube.frag",
    "femtovg/fill_color.frag",
    "femtovg/fill_color.glyph.frag",
    "femtovg/fill_color.glyph.nodiscard.frag",
    "femtovg/fill_color.nodiscard.frag",
    "femtovg/fill_gradient.frag",
    "femtovg/fill_gradient.glyph.frag",
    "femtovg/fill_gradient.glyph.nodiscard.frag",
    "femtovg/fill_gradient.nodiscard.frag",
    "femtovg/fill_gradient_conic.frag",
    "femtovg/fill_gradient_conic.glyph.frag",
    "femtovg/fill_gradient_conic.glyph.nodiscard.frag",
    "femtovg/fill_gradient_conic.nodiscard.frag",
    "femtovg/fill_image.frag",
    "femtovg/fill_image.glyph.frag",
    "femtovg/fill_image.glyph.nodiscard.frag",
    "femtovg/fill_image.nodiscard.frag",
    "femtovg/fill_image_gradient.frag",
    "femtovg/fill_image_gradient.glyph.frag",
    "femtovg/fill_image_gradient.glyph.nodiscard.frag",
    "femtovg/fill_image_gradient.nodiscard.frag",
    "femtovg/fill_image_gradient_conic.frag",
    "femtovg/fill_image_gradient_conic.glyph.frag",
    "femtovg/fill_image_gradient_conic.glyph.nodiscard.frag",
    "femtovg/fill_image_gradient_conic.nodiscard.frag",
    "femtovg/filter_image.frag",
    "femtovg/filter_image.nodiscard.frag",
    "femtovg/stencil.frag",
    "fvgtest_cover.frag",
    "fvgtest_discard.frag",
    "femtovg/fill_color.nodiscard.noscissor.frag",
    "femtovg/fill_gradient.nodiscard.noscissor.frag",
    "femtovg/fill_gradient_conic.nodiscard.noscissor.frag",
    "femtovg/fill_image.nodiscard.noscissor.frag",
    "femtovg/fill_image_gradient.nodiscard.noscissor.frag",
    "femtovg/fill_image_gradient_conic.nodiscard.noscissor.frag",
    "armselect.frag",
    "es100/boolany.frag",
    "es100/branchlane.frag",
    "es100/contloops.frag",
    "es100/databreak.frag",
    "es100/deadloop.frag",
    "es100/deriv.frag",
    "es100/dynidx.frag",
    "es100/dynvec4.frag",
    "es100/f2b.frag",
    "es100/idiv.frag",
    "es100/ifoutput.frag",
    "es100/ifoutput2.frag",
    "es100/immneg.frag",
    "es100/immneg.vert",
    "es100/int_compare.frag",
    "es100/loop_full.frag",
    "es100/loopcarry.frag",
    "es100/matdyn.frag",
    "es100/mvstruct.vert",
    "es100/swizzled_broadcast.frag",
    "es100/texbranch.frag",
    "es100/tmparr_const.frag",
    "es100/tmparr_const.vert",
    "es100/transc.frag",
    "es100/tri_int.frag",
    "es100/unibranch.frag",
    "es100/uniform_idx.frag",
    "es100/uniidx_d.frag",
    "es100/uniidx_e.vert",
    "es100/uniidx_g.frag",
    "es100/uniidx_h.frag",
    "es100/uniidx_i.frag",
    "es100/uniidx_j.frag",
    "es100/utri.frag",
    "es100/varyings15.frag",
    "es100/vtransc.frag",
    "es300/flatlast.vert",
    "es300/idxtex.frag",
    "es300/idxtex.vert",
    "gl_test_es100/armselect.frag",
    "es100/absadd.frag",
    "es100/equirect.frag",
    "es100/fmods.frag",
    "es100/ifelse_loop.frag",
    "es100/selects.frag",
    "es100/sharedint.frag",
    "es100/sharedint.vert",
    "es100/xlink.vert",
    "es300/ifelse_uint.frag",
    "es300/texfetch.frag",
    "es300/texoffset.frag",
    "es300/tmpidx.frag",
    "es300/uniidx.frag",
    "es300/uops.frag",
];

/// The constant register file moves with the words: a relaxation that lets
/// one instruction read two uniforms changes which constants are where. The
/// differing rows whose constant file is nevertheless the i.MX8MP's.
const SAME_CONSTANT_FILE: &[&str] = &["es300/cube.frag", "es100/transc.frag", "es100/vtransc.frag"];

/// The rows that need fewer temporaries on the ST part (3 against 2), each a
/// program the relaxation shortens.
const FEWER_TEMPS: &[&str] = &["armselect.frag", "es100/utri.frag", "gl_test_es100/armselect.frag"];

fn words_that_differ(a: &[u32], b: &[u32]) -> usize {
    let n = a.len().min(b.len());
    (0..n).filter(|&i| a[i] != b[i]).count() + a.len().abs_diff(b.len())
}

fn same_identity(a: &ShcBlob, b: &ShcBlob) -> bool {
    a.sha256 == b.sha256 && a.name == b.name && a.stage == b.stage && a.blend_key == b.blend_key
}

/// The rows of the two tables side by side, with the i.MX8MP row's name.
fn pairs() -> impl Iterator<Item = (&'static str, &'static ShcBlob, &'static ShcBlob)> {
    SHC_BLOB_TABLE.iter().zip(st::SHC_BLOB_TABLE.iter()).map(|(mx, s)| (mx.source_name(), mx, s))
}

#[test]
fn the_st_table_holds_the_rows_of_the_imx8mp_table() {
    assert_eq!(SHC_BLOB_TABLE.len(), ROWS, "the i.MX8MP table moved; regenerate the ST table (--st)");
    assert_eq!(st::SHC_BLOB_TABLE.len(), ROWS, "the ST table does not hold the i.MX8MP table's rows");
    for (i, (mx, s)) in SHC_BLOB_TABLE.iter().zip(st::SHC_BLOB_TABLE.iter()).enumerate() {
        assert!(same_identity(mx, s), "row {i}: {} in the i.MX8MP table, {} in the ST table", mx.name, s.name);
    }
    assert_eq!(st::SHC_SOURCE_TABLE.len(), SHC_SOURCE_TABLE.len());
    for (mx, s) in SHC_SOURCE_TABLE.iter().zip(st::SHC_SOURCE_TABLE.iter()) {
        assert_eq!((mx.name, mx.text), (s.name, s.text), "the ST table stores another source text");
    }
}

#[test]
fn rows_and_words_that_differ_from_the_imx8mp_table() {
    let (mut rows, mut words, mut shorter, mut longer) = (Vec::new(), 0usize, 0usize, 0usize);
    let mut instructions = (0usize, 0usize);
    for (name, mx, s) in pairs() {
        let (a, b) = (mx.shader.usc_words(), s.shader.usc_words());
        instructions.0 += a.len() / 4;
        instructions.1 += b.len() / 4;
        let d = words_that_differ(a, b);
        if d > 0 {
            rows.push(name);
            words += d;
            shorter += (b.len() < a.len()) as usize;
            longer += (b.len() > a.len()) as usize;
        }
    }
    assert_eq!(
        (rows.len(), words),
        (DIFFERING_ROWS, DIFFERING_WORDS),
        "rows and instruction words that differ between the two tables; the rows: {rows:?}"
    );
    assert_eq!(rows, DIFFERING, "the differing rows by name");
    assert_eq!((shorter, longer), (SHORTER_ON_ST, 0), "rows shorter and longer on the ST part");
    assert_eq!(instructions, INSTRUCTIONS, "instructions in the i.MX8MP table and in the ST table");
}

#[test]
fn the_constant_file_and_the_temporaries_move_only_with_the_words() {
    let (mut same_file, mut fewer_temps) = (Vec::new(), Vec::new());
    for (name, mx, s) in pairs() {
        let (a, b) = (&mx.shader, &s.shader);
        let code_differs = a.usc_words() != b.usc_words();
        let file_differs = a.viv.const_contents() != b.viv.const_contents() || a.viv.const_data() != b.viv.const_data();
        assert!(code_differs || !file_differs, "{name}: the constant file differs where the words do not");
        if code_differs && !file_differs {
            same_file.push(name);
        }
        if a.temps != b.temps {
            assert!(code_differs && b.temps + 1 == a.temps, "{name}: temporaries {} against {}", a.temps, b.temps);
            fewer_temps.push(name);
        }
    }
    assert_eq!(same_file, SAME_CONSTANT_FILE, "differing rows that keep the i.MX8MP constant file");
    assert_eq!(fewer_temps, FEWER_TEMPS, "rows with fewer temporaries on the ST part");
}

/// Everything else is the same row for row: the register wiring, the uniform
/// reflection, the attribute list and the flags do not depend on the ids, so
/// a table user needs no second ABI.
#[test]
fn the_wiring_of_every_row_is_the_imx8mp_rows() {
    for (name, mx, s) in pairs() {
        let (a, b) = (&mx.shader, &s.shader);
        assert_eq!(a.uses_discard, b.uses_discard, "{name}: uses_discard");
        assert_eq!(a.reads_front_facing, b.reads_front_facing, "{name}: reads_front_facing");
        assert_eq!(a.uniform_dwords, b.uniform_dwords, "{name}: uniform block size");
        assert_eq!(a.num_attribs, b.num_attribs, "{name}: attribute count");
        for k in 0..a.num_attribs as usize {
            let (x, y) = (&a.attribs[k], &b.attribs[k]);
            assert_eq!(
                (x.location, x.components, x.vi_reg, x.format, x.name, x.gl_type),
                (y.location, y.components, y.vi_reg, y.format, y.name, y.gl_type),
                "{name}: attribute {k}"
            );
        }
        assert_eq!(a.num_uniforms, b.num_uniforms, "{name}: uniform count");
        for (x, y) in a.uniform_table().iter().zip(b.uniform_table().iter()) {
            assert_eq!(
                (x.name, x.dword_off, x.dwords, x.gl_type, x.array_len, x.binding, x.inner),
                (y.name, y.dword_off, y.dwords, y.gl_type, y.array_len, y.binding, y.inner),
                "{name}: a uniform's reflection"
            );
        }
        let (va, vb) = (&a.viv, &b.viv);
        assert_eq!(va.num_in, vb.num_in, "{name}: input count");
        assert_eq!(va.num_out, vb.num_out, "{name}: output count");
        for k in 0..va.num_in as usize {
            let (x, y) = (&va.in_[k], &vb.in_[k]);
            assert_eq!(
                (x.reg, x.slot, x.components, x.interp, x.name),
                (y.reg, y.slot, y.components, y.interp, y.name),
                "{name}: input {k}"
            );
        }
        for k in 0..va.num_out as usize {
            let (x, y) = (&va.out[k], &vb.out[k]);
            assert_eq!(
                (x.reg, x.slot, x.components, x.name),
                (y.reg, y.slot, y.components, y.name),
                "{name}: output {k}"
            );
        }
        assert_eq!(
            (va.vs_pos_out_reg, va.vs_pointsize_out_reg, va.vs_id_in_reg, va.ps_color_out_reg, va.ps_depth_out_reg),
            (vb.vs_pos_out_reg, vb.vs_pointsize_out_reg, vb.vs_id_in_reg, vb.ps_color_out_reg, vb.ps_depth_out_reg),
            "{name}: the special registers"
        );
        assert_eq!(va.vs_load_balancing, vb.vs_load_balancing, "{name}: VS_LOAD_BALANCING");
        assert_eq!(va.input_count_unk8, vb.input_count_unk8, "{name}: input_count_unk8");
    }
}

/// With the decoder: the ST rows read two different constant registers in one
/// instruction, which is the relaxation itself, and the i.MX8MP rows of the
/// same programs never do (tests/constant_registers.rs holds the whole
/// table to that), so every differing program has its one-constant form.
#[cfg(feature = "shaderc-online")]
mod decoded {
    use super::{pairs, DIFFERING_ROWS};

    /// Rows of the ST table with an instruction reading two different
    /// constant registers, and such instructions in all. The other
    /// differing rows read one constant register per instruction on both
    /// parts and differ by a literal that left the constant file for an
    /// immediate operand beside the uniform read (es100/uniidx_d.frag, the
    /// mode-25 words of tools/vivante/README.md; es100/immneg.vert).
    const TWO_CONSTANT_ROWS: usize = 51;
    const TWO_CONSTANT_INSTRUCTIONS: usize = 284;
    const DIFFERING_WITHOUT: usize = DIFFERING_ROWS - TWO_CONSTANT_ROWS;

    /// Instructions of `words` that read more than one distinct constant
    /// register (the count of tests/constant_registers.rs).
    fn multi(words: &[u32]) -> usize {
        let mut n = 0usize;
        yakogl::shaderc::online::vivante::decode::for_each_instr(words, |_, ins| {
            let mut regs: Vec<(u8, u16)> = Vec::new();
            for k in 0..3 {
                if let Some(s) = ins.src(k) {
                    let key = (s.group, s.reg);
                    if (key.0 == 2 || key.0 == 3) && !regs.contains(&key) {
                        regs.push(key);
                    }
                }
            }
            n += (regs.len() > 1) as usize;
            Ok(())
        })
        .expect("decodes");
        n
    }

    #[test]
    fn the_st_rows_read_two_constants_where_the_imx8mp_rows_read_one() {
        let (mut rows, mut instrs, mut differing_without) = (0usize, 0usize, Vec::new());
        for (name, mx, s) in pairs() {
            assert_eq!(
                multi(mx.shader.usc_words()),
                0,
                "{name}: the i.MX8MP row reads two constants in one instruction"
            );
            let m = multi(s.shader.usc_words());
            let differs = mx.shader.usc_words() != s.shader.usc_words();
            assert!(differs || m == 0, "{name}: the same words read two constants on one table only");
            if m > 0 {
                rows += 1;
                instrs += m;
            } else if differs {
                differing_without.push(name);
            }
        }
        assert_eq!(
            (rows, instrs),
            (TWO_CONSTANT_ROWS, TWO_CONSTANT_INSTRUCTIONS),
            "ST rows with a two-constant instruction, and such instructions; differing rows without one: {differing_without:?}"
        );
        assert_eq!(differing_without.len(), DIFFERING_WITHOUT, "differing rows without a two-constant instruction");
    }
}
