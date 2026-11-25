// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mesa 26.2.1 references `tools/mali/testdata/vs_tex2d_cube*` (plan #406,
//! item vs2tex-ref, issue #11308): a vertex shader that reads a sampler2D
//! and a samplerCube, the shape of
//! `dEQP-GLES2.functional.uniform_api.value.assigned.{by_pointer,by_value}.render.basic_struct.sampler2D_samplerCube_vertex`,
//! which `src/mali/backend.rs` refuses at the draw (a vertex shader that
//! reads more than one texture).
//!
//! * `vs_tex2d_cube_deqp.vert` is dEQP's vertex shader, rebuilt from
//!   VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`
//!   `modules/gles2/functional/es2fUniformApiTests.cpp`
//!   (`generateVertexSource`; the fill colours are
//!   `de::Random(deStringHash("sampler2D_samplerCube_vertex"))`), and
//!   `vs_tex2d_cube.frag` its fragment shader. Mesa does not draw that pair
//!   through `tools/mali/gl_ref.py`: a sampler struct member is not set, both
//!   samplers stay at unit 0 and the draw is `GL_INVALID_OPERATION`.
//! * `vs_tex2d_cube.vert` is the same text with the two members as plain
//!   uniforms (`u_var_m0`, `u_var_m1`), which gl_ref.py gives a texture
//!   each at units 0 and 1, the values dEQP sets. Mesa's NIR and
//!   disassembly of the two vertex shaders are the same but for the
//!   declarations. The reference is its draw:
//!   `tools/mali/drm_shim_ref.sh run <shim> tools/mali/testdata/vs_tex2d_cube.vert tools/mali/testdata/vs_tex2d_cube.frag tools/mali/testdata/vs_tex2d_cube --uniform-values`
//!   (`.trace`, `.stdout`, `.stderr`), then
//!   `tools/mali/pan_ref_diff.py extract tools/mali/testdata/vs_tex2d_cube.trace --draw`
//!   (`.expect.json`), which is what
//!   `tools/mali/deqp_ref.sh ... --instrument shim` writes.
//!
//! What is pinned: the files' shape (the two look-ups, the handle words,
//! the vertex stage's sampler and texture descriptors and the resource
//! table they sit in), and what the compiler does with each text at the tip:
//! the plain-uniform text is refused, dEQP's own compiles with other words
//! than Mesa's and the same two handle words. A change of either moves its
//! pin in the same commit.
//!
//! gl_ref.py's textures are not dEQP's: a 2x2 RGBA8 texture and a 16x16
//! RGBA8 cube of five levels under `GL_NEAREST_MIPMAP_NEAREST`, where dEQP
//! uploads a 32x32 RGBA8 texture and a 32x32 RGBA8 cube of one level, all
//! `GL_NEAREST` and `GL_CLAMP_TO_EDGE` (`UniformCase::setupTexture`). The
//! indices, dimensions and wraps are dEQP's; the sizes, level counts and the
//! cube sampler's mipmap mode are gl_ref.py's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_vs_tex2d_cube
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

use yakogl::shaderc::online::mali::fau::FauWord;

const DEQP_VS: &str = include_str!("../tools/mali/testdata/vs_tex2d_cube_deqp.vert");
const VS: &str = include_str!("../tools/mali/testdata/vs_tex2d_cube.vert");
const FS: &str = include_str!("../tools/mali/testdata/vs_tex2d_cube.frag");
const EXPECT: &str = include_str!("../tools/mali/testdata/vs_tex2d_cube.expect.json");
const TRACE: &str = include_str!("../tools/mali/testdata/vs_tex2d_cube.trace");
const STDOUT: &str = include_str!("../tools/mali/testdata/vs_tex2d_cube.stdout");

/// Mesa's handle word of the sampler2D (`u5.w1`): texture 0 of table 4,
/// sampler 0 of table 3.
const HANDLE_2D: u32 = 0x2000_1800;
/// Mesa's handle word of the samplerCube (`u6.w1`): texture 1 of table 4,
/// sampler 1 of table 3.
const HANDLE_CUBE: u32 = 0x2001_1801;

/// The refusal of the plain-uniform text at the tip (`MaliError::message`).
const REFUSAL: &str = "a vertex shader texture look-up other than 2D or cube look-ups at a level";

/// `text` from the first line containing `from` up to the first later line
/// containing `to`.
fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    let a = text.find(from).unwrap_or_else(|| panic!("no {from:?}"));
    let b = text[a..].find(to).unwrap_or_else(|| panic!("no {to:?} after {from:?}")) + a;
    &text[a..b]
}

/// Entry `n` of a resource table's text.
fn entry(table: &str, n: usize) -> &str {
    let from = format!("Entry {n} @");
    let a = table.find(&from).unwrap_or_else(|| panic!("no entry {n}"));
    let b = table[a..].find(&format!("Entry {} @", n + 1)).map_or(table.len(), |b| b + a);
    &table[a..b]
}

/// The values of every `key: value` line of `text`, in order.
fn values<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    text.lines().filter_map(|l| l.trim().strip_prefix(key)?.strip_prefix(": ")).collect()
}

/// The two vertex shaders are one text but for the declaration: dEQP's
/// struct members are the plain uniforms of the drawn one.
#[test]
fn drawn_text_is_deqps_with_plain_uniforms() {
    let block =
        "struct structType\n{\n\tmediump sampler2D m0;\n\tmediump samplerCube m1;\n};\nuniform structType u_var;\n";
    assert!(DEQP_VS.contains(block));
    let plain = "uniform mediump sampler2D u_var_m0;\nuniform mediump samplerCube u_var_m1;\n";
    assert_eq!(DEQP_VS.replace(block, plain).replace("u_var.m", "u_var_m"), VS);
    assert!(VS.contains("texture2D(u_var_m0, vec2(0.0)), vec4(0.15, 0.52, 0.26, 0.35)"));
    assert!(VS.contains("textureCube(u_var_m1, vec3(0.0)), vec4(0.88, 0.09, 0.30, 0.61)"));
    assert!(!FS.contains("uniform"));
}

/// The draw gave the sampler2D unit 0 and the samplerCube unit 1, without a
/// GL error.
#[test]
fn stdout_units() {
    assert!(STDOUT.contains("gl_ref: sampler2D u_var_m0 at unit 0: a 2x2 RGBA8 texture\n"));
    assert!(STDOUT.contains("gl_ref: samplerCube u_var_m1 at unit 1: a 16x16 RGBA8 cube map of 5 levels\n"));
    assert!(STDOUT.contains("glGetError 0x0\n"));
}

/// The varying program holds the two look-ups, each at level zero through
/// its handle word; the position program has none; both see one FAU table.
#[test]
fn expect_words() {
    let pos = between(EXPECT, "\"vs.position\"", "\"vs.varying\"");
    let var = between(EXPECT, "\"vs.varying\"", "\"fs.fragment\"");
    assert!(!pos.contains("TEX_"));
    assert_eq!(var.matches("TEX_").count(), 2);
    for text in [
        "\"MOV.i32 r0, u6.w1\"",
        "\"TEX_SINGLE.slot0.reserved.16.cube.zero.wait0 @r0:r1, @r2:r3, [r0^:r1^]\"",
        "\"MOV.i32 r4, u5.w1\"",
        "\"TEX_SINGLE.slot1.reserved.16.2d.zero.wait1 @r2:r3, @r2:r3, [r4^:r5^]\"",
    ] {
        assert!(var.contains(text), "{text}");
    }
    /* words 11 to 14: u5.w1 the 2D handle, u6.w0 the cube face mask, u6.w1
     * the cube handle, u7.w0 compare_float's 0.05 as two fp16 */
    let fau = format!("\"{HANDLE_2D:08x}\",\"1fffffff\",\"{HANDLE_CUBE:08x}\",\"2a662a66\",\"00000000\"]");
    for region in [pos, var] {
        let packed: String = region.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(packed.contains(&fau));
        assert_eq!(packed.split("\"fau\":[").nth(1).unwrap().split(']').next().unwrap().matches(',').count(), 15);
    }
}

/// The draw's resource tables: the position and the varying program share
/// one (`varying_srt0`), whose entry 3 holds the two samplers and entry 4
/// the two textures, the 2D one first; the fragment table has its own
/// sampler and no texture.
#[test]
fn trace_descriptors() {
    assert_eq!(TRACE.matches("RUN_IDVS").count(), 1);
    assert!(TRACE.contains("RUN_IDVS.varying_srt0.varying_fau0.varying_tsd0.frag_srt1.frag_tsd0"));
    let pos = between(TRACE, "Position resources resource table", "Varying resources resource table");
    let var = between(TRACE, "Varying resources resource table", "Fragment resources resource table");
    let frag = between(TRACE, "Fragment resources resource table", "Position FAU");
    assert_eq!(pos.split_once('\n').unwrap().1, var.split_once('\n').unwrap().1);

    let samplers = entry(pos, 3);
    assert_eq!(values(samplers, "Size"), ["0x40"]);
    assert_eq!(values(samplers, "Type"), ["Sampler", "Sampler"]);
    assert_eq!(values(samplers, "Wrap Mode S"), ["Clamp to Edge", "Clamp to Edge"]);
    assert_eq!(values(samplers, "Wrap Mode T"), ["Clamp to Edge", "Clamp to Edge"]);
    assert_eq!(values(samplers, "Wrap Mode R"), ["Repeat", "Clamp to Edge"]);
    assert_eq!(values(samplers, "Minify nearest"), ["true", "true"]);
    assert_eq!(values(samplers, "Magnify nearest"), ["true", "true"]);
    assert_eq!(values(samplers, "Mipmap Mode"), ["None", "Nearest"]);
    assert_eq!(values(samplers, "Seamless Cube Map"), ["true", "true"]);
    assert_eq!(values(samplers, "Minimum LOD"), ["0.000000", "0.000000"]);
    assert_eq!(values(samplers, "Maximum LOD"), ["31.996094", "31.996094"]);

    let textures = entry(pos, 4);
    assert_eq!(values(textures, "Size")[0], "0x40");
    assert_eq!(values(textures, "Type").iter().filter(|t| **t == "Texture").count(), 2);
    assert_eq!(values(textures, "Dimension"), ["2D", "Cube"]);
    assert_eq!(values(textures, "Format (v7)"), ["RGBA8 UNORM RGBA", "RGBA8 UNORM RGBA"]);
    assert_eq!(values(textures, "Levels"), ["1", "5"]);
    assert_eq!(values(textures, "Maximum LOD"), ["0.000000", "4.000000"]);
    assert_eq!(values(textures, "Array size"), ["1", "1"]);
    let two_d = between(textures, "Dimension: 2D", "Dimension: Cube");
    assert_eq!((values(two_d, "Width")[0], values(two_d, "Height")[0]), ("2", "2"));
    let cube = &textures[textures.find("Dimension: Cube").unwrap()..];
    assert_eq!((values(cube, "Width")[0], values(cube, "Height")[0]), ("16", "16"));
    /* five levels of six faces */
    assert_eq!(values(cube, "Type").iter().filter(|t| **t == "Plane").count(), 30);

    for n in 5..8 {
        assert_eq!(values(entry(pos, n), "Contains descriptors"), ["false"]);
    }
    assert_eq!(values(entry(frag, 3), "Type"), ["Sampler"]);
    assert_eq!(values(entry(frag, 4), "Contains descriptors"), ["false"]);
}

/// The drawn pair, byte for byte.
#[test]
#[ignore = "the pair is refused; pinned by compiler_pinned"]
fn vs_tex2d_cube() {
    mali_ref::assert_matches("vs_tex2d_cube");
}

/// At the tip the plain-uniform text is refused (its fp16 texels go to a
/// compare), and dEQP's own text compiles with other words than Mesa's
/// (the struct's members read as fp32) around the same two handle words.
#[test]
fn compiler_pinned() {
    let mut changed = Vec::new();
    changed.extend(mali_ref::refusal_change("vs_tex2d_cube", REFUSAL));
    let r = mali_ref::Reference { vs: DEQP_VS.to_string(), ..mali_ref::Reference::load("vs_tex2d_cube") };
    match mali_ref::compile_pair(&r.vs, &r.fs) {
        Ok(pair) => {
            let names: Vec<&str> = pair.vs.uniforms.iter().map(|u| u.0.as_str()).collect();
            if names != ["u_var.m0", "u_var.m1"] {
                changed.push(format!("dEQP's text: uniform rows {names:?}"));
            }
            for (what, word) in [("sampler2D", HANDLE_2D), ("samplerCube", HANDLE_CUBE)] {
                if pair.vs.fau.iter().filter(|w| **w == FauWord::Constant(word)).count() != 1 {
                    changed.push(format!("dEQP's text: the {what} handle {word:08x} is not one FAU word"));
                }
            }
            if mali_ref::differences(&r, &pair).is_empty() {
                changed.push("dEQP's text: now byte-equal to Mesa's; pin it with a byte-compare".into());
            }
        }
        Err(e) => changed.push(format!("dEQP's text: no longer compiles: {e}")),
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
