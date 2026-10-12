// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 1.00 twins of gl_test's GLSL 450 shaders are the same programs.
//!
//! `tools/shaders/gl_test_es100/` holds a GLSL ES 1.00 version of each of the
//! thirteen `#version 450` sources gl_test draws with, because those thirteen
//! use `layout(location=)`, a Vulkan push-constant block, a descriptor-set
//! sampler and a user fragment output -- none of which an ES 2.0 application
//! can write, and so none of which an ES 2.0 compiler can be asked to compile.
//! `gl_test_shaders_es100.h` substitutes them behind one build switch.
//!
//! A twin is only useful if it is the SAME PROGRAM. If a rewrite dropped an
//! attribute, renamed a uniform or reordered a declaration, gl_test would still
//! run -- and would quietly be testing something else, with every case that
//! failed afterwards read as a compiler defect. So the property is checked
//! rather than intended: for each pair, the reflection the offline compiler
//! derived from the twin must equal the reflection it derived from the
//! original, field by field.
//!
//! One property of the code IS compared, though the code itself is not: a
//! twin's program is empty exactly when its original's is. Twelve rows in the
//! GC7000 table hold four zero dwords -- one `nop` -- and every one of them is
//! a PURE PASS-THROUGH whose every output is a direct copy of an input
//! (`gl_Position = a_pos; v_color = a_color;`). On this part that needs no ALU
//! instruction: the register wiring in `ShcViv` routes input registers to
//! output registers and the program is a single terminating `nop`. It is not a
//! placeholder and not a failed compile, which the board says plainly -- cases
//! 1, 21 and 34 draw with `passthrough.vert`, 10 and 40 with
//! `interp_color.frag`, and eighteen cases with `color_uv.vert`; they pass. A
//! vertex shader that wrote no position could not.
//!
//! So "the code is non-empty" would be the wrong assertion here: it would
//! outlaw twelve correct rows. The useful one is the twin property. A twin
//! whose program came out empty while its original's did not is a twin that
//! failed to compile, and that is invisible in the reflection -- which is
//! exactly the hole an exclusion of the code fields would otherwise leave.
//!
//! Three groups of fields are deliberately NOT compared, and each for a reason
//! that would make the comparison meaningless rather than for convenience:
//!
//! * `usc`/`usc_dw`/`temps` -- the compiled instructions and the register
//!   pressure. Two sources that say the same thing in different syntax compile
//!   to different code; that they do is the point of the exercise.
//! * `meta_off`, `tile_buffer_sh`, `num_tile_buffers`, `viv`, `priv_` -- where
//!   the backend put things, not what the program declares.
//! * `blend_key` -- the row's key, not a property of the source.
//!
//! Everything a GL application can observe through the API (attributes,
//! uniforms, varyings, outputs, the uniform block's size) is compared.

//! `table` is not one table: it aliases the GC7000 blobs in a `gpu-vivante`
//! build and the PowerVR blobs otherwise, so the two builds are asked
//! different questions here. The twins belong to the GC7000 table, which is
//! ours; the PowerVR table is the AM62P's and this work must not add a row to
//! it, so in that build the check is that it holds NO twin at all. Asking the
//! GC7000 question of the PowerVR table is how the first version of this file
//! failed: it read "0 of 13 twins" from a table that should never have any.

use super::mesa_reference::SHC_BLOB_TABLE;
#[cfg(feature = "gpu-vivante")]
use super::{ShcAttrib, ShcShader, ShcUniform, ShcVarying};

const TWIN_DIR: &str = "gl_test_es100/";

#[cfg(feature = "gpu-vivante")]
fn name_of(n: &[core::ffi::c_char]) -> &str {
    let b: &[u8] = unsafe { core::slice::from_raw_parts(n.as_ptr() as *const u8, n.len()) };
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..end]).unwrap()
}

#[cfg(feature = "gpu-vivante")]
fn attrib_eq(a: &ShcAttrib, b: &ShcAttrib) -> bool {
    a.location == b.location
        && a.components == b.components
        && a.vi_reg == b.vi_reg
        && a.format == b.format
        && a.gl_type == b.gl_type
        && name_of(&a.name) == name_of(&b.name)
}

#[cfg(feature = "gpu-vivante")]
fn uniform_eq(a: &ShcUniform, b: &ShcUniform) -> bool {
    a.dword_off == b.dword_off
        && a.dwords == b.dwords
        && a.gl_type == b.gl_type
        && a.array_len == b.array_len
        && a.binding == b.binding
        && a.inner == b.inner
        && name_of(&a.name) == name_of(&b.name)
}

#[cfg(feature = "gpu-vivante")]
fn varying_eq(a: &ShcVarying, b: &ShcVarying) -> bool {
    a.location == b.location
        && a.components == b.components
        && a.interp == b.interp
        && a.vtxout_off == b.vtxout_off
        && name_of(&a.name) == name_of(&b.name)
}

/// Every way the two reflections differ, as human-readable lines.
#[cfg(feature = "gpu-vivante")]
fn differences(orig: &ShcShader, twin: &ShcShader) -> Vec<String> {
    let mut out = Vec::new();
    macro_rules! scalar {
        ($f:ident) => {
            if orig.$f != twin.$f {
                out.push(format!("{}: original {:?}, twin {:?}", stringify!($f), orig.$f, twin.$f));
            }
        };
    }
    scalar!(num_attribs);
    scalar!(num_uniforms);
    scalar!(num_varyings);
    scalar!(num_outputs);
    scalar!(uses_w);
    scalar!(uses_discard);
    scalar!(reads_front_facing);
    scalar!(uniform_dwords);
    scalar!(uniform_sh_dwords);
    scalar!(vs_output_size);
    scalar!(vs_psize_off);
    scalar!(vs_vertex_id_reg1);
    scalar!(vs_instance_id_reg1);
    scalar!(uses_fbfetch);

    /* Not the code, only whether there is any: see the note at the top. */
    let empty = |sh: &ShcShader| {
        sh.usc_dw == 0 || unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }.iter().all(|&w| w == 0)
    };
    if empty(orig) != empty(twin) {
        out.push(format!(
            "one program is empty and the other is not: original {} ({} dwords), twin {} ({} dwords)",
            if empty(orig) { "empty" } else { "has code" },
            orig.usc_dw,
            if empty(twin) { "empty" } else { "has code" },
            twin.usc_dw
        ));
    }

    for i in 0..(orig.num_attribs.min(twin.num_attribs) as usize) {
        let (a, b) = (&orig.attribs[i], &twin.attribs[i]);
        if !attrib_eq(a, b) {
            out.push(format!(
                "attrib {}: original {} loc {} comp {} reg {} fmt {} type {:#x}, \
                 twin {} loc {} comp {} reg {} fmt {} type {:#x}",
                i,
                name_of(&a.name),
                a.location,
                a.components,
                a.vi_reg,
                a.format,
                a.gl_type,
                name_of(&b.name),
                b.location,
                b.components,
                b.vi_reg,
                b.format,
                b.gl_type
            ));
        }
    }
    for i in 0..(orig.num_varyings.min(twin.num_varyings) as usize) {
        let (a, b) = (&orig.varyings[i], &twin.varyings[i]);
        if !varying_eq(a, b) {
            out.push(format!(
                "varying {}: original {} loc {} comp {} interp {} off {}, \
                 twin {} loc {} comp {} interp {} off {}",
                i,
                name_of(&a.name),
                a.location,
                a.components,
                a.interp,
                a.vtxout_off,
                name_of(&b.name),
                b.location,
                b.components,
                b.interp,
                b.vtxout_off
            ));
        }
    }
    let n = orig.num_uniforms.min(twin.num_uniforms) as usize;
    if n > 0 {
        let ua = unsafe { core::slice::from_raw_parts(orig.uniforms, n) };
        let ub = unsafe { core::slice::from_raw_parts(twin.uniforms, n) };
        for i in 0..n {
            if !uniform_eq(&ua[i], &ub[i]) {
                out.push(format!(
                    "uniform {}: original {} off {} dw {} type {:#x} len {} bind {} inner {}, \
                     twin {} off {} dw {} type {:#x} len {} bind {} inner {}",
                    i,
                    name_of(&ua[i].name),
                    ua[i].dword_off,
                    ua[i].dwords,
                    ua[i].gl_type,
                    ua[i].array_len,
                    ua[i].binding,
                    ua[i].inner,
                    name_of(&ub[i].name),
                    ub[i].dword_off,
                    ub[i].dwords,
                    ub[i].gl_type,
                    ub[i].array_len,
                    ub[i].binding,
                    ub[i].inner
                ));
            }
        }
    }
    out
}

/// The fourteen pairs, found in the table rather than listed here: any source
/// under `gl_test_es100/` must have a same-basename original beside it. A twin
/// added without its original, or an original renamed out from under a twin,
/// is a failure and not a silently shorter run.
#[cfg(feature = "gpu-vivante")]
#[test]
fn every_es100_twin_reflects_exactly_like_its_glsl_450_original() {
    let rows = &SHC_BLOB_TABLE;
    let mut pairs = 0;
    let mut problems: Vec<String> = Vec::new();
    for row in rows.iter() {
        let n = row.source_name();
        let Some(base) = n.strip_prefix(TWIN_DIR) else { continue };
        let Some(orig) = rows.iter().find(|r| r.source_name() == base) else {
            problems.push(format!("{n}: no original named {base} in the table"));
            continue;
        };
        if orig.stage != row.stage {
            problems.push(format!("{n}: stage {} but original {}", row.stage, orig.stage));
            continue;
        }
        pairs += 1;
        for d in differences(&orig.shader, &row.shader) {
            problems.push(format!("{n}: {d}"));
        }
    }

    assert!(problems.is_empty(), "{} twin(s) differ from their original:\n  {}", problems.len(), problems.join("\n  "));
    assert_eq!(pairs, 14, "expected 14 ES 1.00 twins in the table, found {pairs}");
}

/// The AM62P's table is not ours and this work must not change it. The twins
/// are compiled into the GC7000 table only, so a `gl_test_es100/` row turning
/// up in the PowerVR table means a source landed in `tools/glsl2usc.sh`'s
/// inputs -- which would rebuild every blob the AM62P boots with.
#[cfg(not(feature = "gpu-vivante"))]
#[test]
fn the_powervr_table_holds_no_es100_twin() {
    let found: Vec<&str> = SHC_BLOB_TABLE.iter().map(|b| b.source_name()).filter(|n| n.starts_with(TWIN_DIR)).collect();
    assert!(found.is_empty(), "the PowerVR table gained ES 1.00 twin rows: {found:?}");
}
