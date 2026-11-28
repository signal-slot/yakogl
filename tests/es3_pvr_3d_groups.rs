// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The dEQP-GLES3 `texture.filtering.3d` fragment programs (AM62P tree rows
//! 22-38) on the PowerVR online compiler with native 3D textures on
//! (`pvr_native_3d`, docs/notes/pvr-3d-native.md, docs/notes/pvr-3d-mipmap-hang.md).
//!
//! host-deqp-r22b measured: with native 3D on, every fragment program of those
//! rows compiles to exactly one `smp` with `dmn` 3d and 48 instruction groups;
//! on the grid of slices (switch off) it is 161 groups and two 2D `smp`. The
//! group count is what #1285's watchdog cost scales with, so this pins one 3D
//! `smp` and at most 48 groups (inv-1285-cost's request). The default is
//! not changed here; the switch is set explicitly.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_3d_groups
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment;
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

/// The most groups one program of rows 22-38 may take with native 3D on.
const MAX_GROUPS: usize = 48;

/// The compiler keeps its output in one arena and the switch is process-wide:
/// one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// dEQP's 3D filtering fragment shader (gles3 `TextureFilteringCase3D`,
/// `texture(u_sampler, v_texCoord) * u_colorScale + u_colorBias`), with the
/// look-up form of the case: plain `texture` for the nearest/linear and
/// mipmap min filters alike (the filter is sampler state, not code).
fn deqp_fragment(lookup: &str) -> String {
    format!(
        "#version 300 es\nlayout(location = 0) out mediump vec4 dEQP_FragColor;\n\
         uniform highp sampler3D u_sampler;\nuniform mediump vec4 u_colorScale;\n\
         uniform mediump vec4 u_colorBias;\nin highp vec3 v_texCoord;\n\
         void main() {{ dEQP_FragColor = {lookup} * u_colorScale + u_colorBias; }}\n"
    )
}

/// (instruction groups, `smp` count, `dmn` of each `smp`) of the program.
fn shape(src: &str, native_3d: bool) -> (usize, Vec<u32>) {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Default;
    impl Drop for Default {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _d = Default;
    yakogl::shaderc::pvr_set_native_3d(i32::from(native_3d));
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).expect("front end");
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("chain");
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let (mut off, mut groups, mut dmns) = (0usize, 0usize, Vec::new());
        while off < image.len() {
            let g = decode_group(&image, off).expect("decodes");
            for (_, ph, r) in g.instrs() {
                if ops_for(r, ph).any(|o| o.name == "smp") {
                    dmns.push(r.get("dmn").unwrap_or(0));
                }
            }
            groups += 1;
            off += g.total;
        }
        (groups, dmns)
    })
    .unwrap()
}

fn dmn_3d() -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == "dmn")
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == "3d"))
        .map(|(v, _)| *v)
        .expect("dmn has 3d")
}

/// The representative programs: nearest and linear (and the NEAREST/LINEAR
/// mipmap min filters, which differ only in sampler state) use the one look-up.
const CASES: &[(&str, &str)] = &[
    ("nearest / linear", "texture(u_sampler, v_texCoord)"),
    ("mipmap min filters", "texture(u_sampler, v_texCoord)"),
    ("wrap-mode cases, projected form", "textureProj(u_sampler, vec4(v_texCoord, 1.0))"),
];

#[test]
fn native_3d_programs_are_one_3d_smp_in_at_most_48_groups() {
    let want = dmn_3d();
    for (name, lookup) in CASES {
        let (groups, dmns) = shape(&deqp_fragment(lookup), true);
        assert_eq!(dmns, vec![want], "{name}: one smp with dmn 3d");
        assert!(groups <= MAX_GROUPS, "{name}: {groups} groups, limit {MAX_GROUPS}");
    }
}

#[test]
fn off_it_is_the_grid_of_slices_two_2d_smp() {
    let want = dmn_3d();
    let (groups, dmns) = shape(&deqp_fragment("texture(u_sampler, v_texCoord)"), false);
    assert_eq!(dmns.len(), 2, "two 2D samples on the grid");
    assert!(dmns.iter().all(|d| *d != want));
    assert!(groups > MAX_GROUPS, "the grid is far past the limit ({groups} groups)");
}
