// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Integer-format samplers on cube, 2D array and 3D targets on the PowerVR
//! online back end: the shape of dEQP-GLES3 `texture.format.sized.{cube,
//! 2d_array,3d}` for the integer formats (R8I/R8UI, RG16I/RG16UI, RGB32I,
//! RGBA32UI, ...). On the AM62P at ad2b5ed about 150 of those lines failed
//! with "Failed to compile shader program at glsTextureTestUtil.cpp:411"
//! (docs/es3/am62p-gles3-notpass.tsv); the selector has taken them since, and
//! nothing pinned it.
//!
//! The shader is dEQP's (glsTextureTestUtil.cpp): a varying coordinate, an
//! `isamplerCube` / `usampler2DArray` / `isampler3D` uniform, one `texture()`
//! look-up, the integer result converted to float and scaled and biased into
//! a float colour output. The format enters a shader only through the
//! sampler's type (signed or unsigned), so the formats (8, 16 and 32 bit,
//! signed and unsigned, one, two and four components) are represented by the
//! sampler type; the lists in `TARGETS` name the formats each stands for.
//!
//! What is checked, per sampler type and per switch setting (the defaults;
//! native cube, 2D array and 3D all on; all off): the front end accepts the
//! shader, the PowerVR chain links it with no refusal, the program carries
//! at least one `smp` and, with the native targets on, its `dmn` is the
//! target's (3d for a cube, 2d for a native 2D array, 3d for a native 3D).
//! The vertex stage with `textureLod` is checked the same way. HOST-ONLY: the
//! byte model reads no integer texel formats, so there is no texture upload,
//! draw or read-back here; what is pinned is that the shaders compile, link
//! and sample.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_int_targets_deqp
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The three native-target switches set for the closure, and back at their
/// defaults after it, panic or not.
fn with_switches<T>(cube: bool, array: bool, d3: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _r = Reset;
    yakogl::shaderc::pvr_set_native_cube(i32::from(cube));
    yakogl::shaderc::pvr_set_native_array(i32::from(array));
    yakogl::shaderc::pvr_set_native_3d(i32::from(d3));
    f()
}

/// The default switches.
fn with_defaults<T>(f: impl FnOnce() -> T) -> T {
    with_switches(
        yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT != 0,
        yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT != 0,
        yakogl::shaderc::PVR_NATIVE_3D_DEFAULT != 0,
        f,
    )
}

/// One shader through the online chain: the `dmn` of every `smp` in the
/// program, in order, or the refusal.
fn compile(stage: Stage, src: &str) -> Result<Vec<u32>, String> {
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = if stage == Stage::Fragment {
            compile_fragment(&l.module, &l.reflection, a)
        } else {
            compile_vertex(&l.module, &l.reflection, a)
        }
        .map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut off = 0usize;
        let mut dmns = Vec::new();
        while off < image.len() {
            let g = decode_group(&image, off).expect("decodes");
            for (_, ph, r) in g.instrs() {
                if ops_for(r, ph).any(|o| o.name == "smp") {
                    dmns.push(r.get("dmn").unwrap_or(0));
                }
            }
            off += g.total;
        }
        Ok(dmns)
    })
    .unwrap()
}

/// dEQP's fragment shader for one sampler type: coordinate varying, one
/// `texture()` look-up, `vec4()` of the integer texel scaled and biased.
fn deqp_fragment(sampler: &str) -> String {
    format!(
        "#version 300 es\nlayout(location = 0) out highp vec4 dEQP_FragColor;\n\
         uniform highp {sampler} u_sampler;\nuniform highp vec4 u_scale;\nuniform highp vec4 u_bias;\n\
         in highp vec3 v_texCoord;\n\
         void main() {{ dEQP_FragColor = vec4(texture(u_sampler, v_texCoord)) * u_scale + u_bias; }}\n"
    )
}

/// Every sampler type the integer formats of the three targets are read
/// through, with the formats it stands for.
const TARGETS: &[(&str, &str)] = &[
    ("isamplerCube", "cube: r8i rg8i rgba8i, r16i rg16i rgba16i, r32i rg32i rgba32i"),
    ("usamplerCube", "cube: r8ui rg8ui rgba8ui, r16ui rg16ui rgba16ui, r32ui rg32ui rgba32ui, rgb10_a2ui"),
    ("isampler2DArray", "2d_array: r8i rg8i rgb8i rgba8i, r16i rg16i rgba16i, r32i rg32i rgba32i"),
    ("usampler2DArray", "2d_array: r8ui rg8ui rgba8ui, r16ui rg16ui rgba16ui, r32ui rg32ui rgb10_a2ui"),
    ("isampler3D", "3d: r8i rg8i rgba8i, r16i rg16i rgba16i, r32i rg32i rgba32i"),
    ("usampler3D", "3d: r8ui rg8ui rgba8ui, r16ui rg16ui rgba16ui, r32ui rg32ui rgb10_a2ui"),
];

/// A target's `dmn` when its native switch is on: a cube is the TPU's own
/// (3d), a native 2D array is 2d with a layer override, a native 3D is 3d.
fn native_dmn(sampler: &str) -> &'static str {
    if sampler.ends_with("2DArray") {
        "2d"
    } else {
        "3d"
    }
}

fn dmn_value(name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == "dmn")
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("dmn has no value {name}"))
}

#[test]
fn integer_samplers_compile_at_the_default_switches() {
    with_defaults(|| {
        for (sampler, formats) in TARGETS {
            let dmns = compile(Stage::Fragment, &deqp_fragment(sampler))
                .unwrap_or_else(|e| panic!("{sampler} ({formats}): {e}"));
            assert!(!dmns.is_empty(), "{sampler} ({formats}): no smp in the program");
        }
    });
}

#[test]
fn integer_samplers_compile_with_every_native_target_on() {
    with_switches(true, true, true, || {
        for (sampler, formats) in TARGETS {
            let dmns = compile(Stage::Fragment, &deqp_fragment(sampler))
                .unwrap_or_else(|e| panic!("{sampler} ({formats}): {e}"));
            assert!(!dmns.is_empty(), "{sampler} ({formats}): no smp in the program");
            let want = dmn_value(native_dmn(sampler));
            assert!(dmns.iter().all(|d| *d == want), "{sampler}: smp dmn {dmns:?}, want {want}");
        }
    });
}

#[test]
fn integer_samplers_compile_with_every_native_target_off() {
    with_switches(false, false, false, || {
        for (sampler, formats) in TARGETS {
            let dmns = compile(Stage::Fragment, &deqp_fragment(sampler))
                .unwrap_or_else(|e| panic!("{sampler} ({formats}): {e}"));
            assert!(!dmns.is_empty(), "{sampler} ({formats}): no smp in the program");
        }
    });
}

#[test]
fn integer_samplers_also_sample_in_the_vertex_stage() {
    // dEQP's texture.vertex uses the same samplers, read with textureLod.
    with_defaults(|| {
        for (sampler, formats) in TARGETS {
            let src = format!(
                "#version 300 es\nuniform highp {sampler} u_sampler;\nin highp vec4 a;\nin highp vec3 c;\n\
                 out highp vec4 v;\n\
                 void main() {{ gl_Position = a; v = vec4(textureLod(u_sampler, c, 0.0)); }}\n"
            );
            let dmns = compile(Stage::Vertex, &src).unwrap_or_else(|e| panic!("{sampler} ({formats}): {e}"));
            assert!(!dmns.is_empty(), "{sampler}: no smp in the vertex program");
        }
    });
}
