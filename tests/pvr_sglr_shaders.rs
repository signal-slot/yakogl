// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The dEQP sglr shaders of texture.specification, texture.units and
//! fbo.color / fbo.depth compile and link on the PowerVR online compiler.
//!
//! The AM62P record of ad2b5ed had 493 cases fail with "Compile failed at
//! sglrGLContext.cpp:863" (texture.specification 323, texture.units 103,
//! fbo.color 57, fbo.depth 10). At the tip none of those shaders fails on the
//! host any more; this test pins that, through the library's own
//! glCompileShader / glLinkProgram, so it cannot come back.
//!
//! The shader texts are the C++ string templates of VK-GL-CTS
//! 93bca01861b0e3ef3c387027a9791e6d065f900c (Apache-2.0, Copyright 2014 The
//! Android Open Source Project), rebuilt here by the same concatenation:
//! - modules/gles3/functional/es3fFboTestUtil.cpp: `FlatColorShader`,
//!   `GradientShader`, `genTexFragmentShader` and `genTexture2DShaderDecl`
//!   (`Texture2DShader`), `TextureCubeShader`, `Texture2DArrayShader`,
//!   `Texture3DShader`, `DepthGradientShader` (the `gl_FragDepth` one);
//! - modules/gles3/functional/es3fTextureUnitTests.cpp:
//!   `generateMultiTexFragmentShader` and the `MultiTexShader` vertex shader.
//!
//! texture.specification builds the 2D / cube / 2D-array / 3D shaders with
//! the sampler type of the texture format and a `vec4` output, and its
//! `GradientShader` cases use `vec4`. fbo.color builds `FlatColorShader`,
//! `GradientShader` and `Texture2DShader` with the output type of the
//! attachment's format (`vec4`, `ivec4`, `uvec4`) and any sampler type;
//! fbo.depth builds `DepthGradientShader`. So every shader here is crossed
//! over the float / int / uint sampler classes and the three output types.
//! texture.units uses 2, 4, 8 and the maximum (16) units of 2D, cube,
//! 2D-array and 3D samplers, alone and mixed.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_sglr_shaders
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeSet;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ------------------------------------------------------- the stand-in -- */

unsafe extern "C" fn s_init() -> i32 {
    0
}
unsafe extern "C" fn s_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn s_draw(_d: *const GlesDraw) -> i32 {
    0
}
unsafe extern "C" fn s_ok() -> i32 {
    0
}
unsafe extern "C" fn s_kick_id() -> u32 {
    1
}
unsafe extern "C" fn s_kick_pending(_id: u32) -> bool {
    false
}

/// The PowerVR's flags (src/pvr/backend.rs `yakogl_backend_pvr`); nothing is
/// drawn here, only compiled and linked.
static STAND_IN: YakoglBackend = YakoglBackend {
    name: c"sglr shaders stand-in".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(s_init),
    shutdown: None,
    draw: Some(s_draw),
    flush: Some(s_ok),
    finish: Some(s_ok),
    finish_previous: None,
    kick_id: Some(s_kick_id),
    kick_pending: Some(s_kick_pending),
    heap_base: Some(s_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu };
    0
}
unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(o_gpu_alloc),
    gpu_free: Some(o_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// One test at a time: the back end, the OSAL and the GL core are statics.
static SERIAL: Mutex<()> = Mutex::new(());

struct Ctx(*mut yakogl::gl::context::GlContext);

impl Drop for Ctx {
    fn drop(&mut self) {
        // SAFETY: the context `start` made.
        unsafe { yakogl::gl::context::gl_context_destroy(self.0) };
    }
}

fn start() -> (std::sync::MutexGuard<'static, ()>, Ctx) {
    let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; both tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &STAND_IN;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let ctx = yakogl::gl::context::gl_context_create(256, 256);
        assert!(!ctx.is_null(), "context pool exhausted");
        ctx
    };
    (g, Ctx(ctx))
}

/* -------------------------------------------- compile and link, by GL -- */

fn shader_log(s: GLuint) -> String {
    let mut buf = [0 as c_char; 2048];
    let mut n: GLsizei = 0;
    // SAFETY: a buffer of the stated length.
    unsafe { glGetShaderInfoLog(s, buf.len() as GLsizei, &mut n, buf.as_mut_ptr()) };
    buf[..(n.max(0) as usize).min(buf.len())].iter().map(|&c| c as u8 as char).collect()
}

fn compile(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            return Err(format!("compile failed: {}\n{text}", shader_log(s)));
        }
        Ok(s)
    }
}

/// Compile both stages and link them; the program is deleted again.
fn build(vs: &str, fs: &str) -> Result<(), String> {
    let v = compile(GL_VERTEX_SHADER, vs)?;
    let f = compile(GL_FRAGMENT_SHADER, fs)?;
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let r = if ok == 1 { Ok(()) } else { Err(format!("link failed\n{vs}\n{fs}")) };
        glDeleteProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        r
    }
}

/// Every distinct pair builds; returns how many distinct pairs there were.
fn build_all(pairs: Vec<(String, String)>) -> usize {
    let _s = start();
    let distinct: BTreeSet<(String, String)> = pairs.into_iter().collect();
    let mut failed = Vec::new();
    for (vs, fs) in &distinct {
        if let Err(e) = build(vs, fs) {
            failed.push(e);
        }
    }
    assert!(failed.is_empty(), "{} of {} pairs failed, first:\n{}", failed.len(), distinct.len(), failed[0]);
    distinct.len()
}

/* ------------------------------------------- the dEQP shader templates -- */

/// The output types of `getFragmentOutputType`: float, int and uint formats.
const OUT_TYPES: [&str; 3] = ["vec4", "ivec4", "uvec4"];
/// The sampler classes of `glu::getSampler*Type`: float, int and uint formats.
const CLASSES: [&str; 3] = ["", "i", "u"];

/// The vertex shader of `FlatColorShader` (es3fFboTestUtil.cpp:99-104).
const VS_POSITION: &str = "#version 300 es\n\
in highp vec4 a_position;\n\
void main (void)\n\
{\n\
    gl_Position = a_position;\n\
}\n";

/// The vertex shader of `GradientShader` and `DepthGradientShader`
/// (es3fFboTestUtil.cpp:178-186, :829-837).
const VS_GRADIENT: &str = "#version 300 es\n\
in highp vec4 a_position;\n\
in highp vec4 a_coord;\n\
out highp vec4 v_coord;\n\
void main (void)\n\
{\n\
    gl_Position = a_position;\n\
    v_coord = a_coord;\n\
}\n";

/// The vertex shader of `Texture2DShader` (:310-318), `Texture2DArrayShader`
/// (:596-604) and `Texture3DShader` (:712-720) differ only in `void main(void)`
/// against `void main (void)`; both spellings are built.
const VS_TEX_2D: &str = "#version 300 es\n\
in highp vec4 a_position;\n\
in highp vec2 a_coord;\n\
out highp vec2 v_coord;\n\
void main(void)\n\
{\n\
    gl_Position = a_position;\n\
    v_coord = a_coord;\n\
}\n";

const VS_TEX_LAYERED: &str = "#version 300 es\n\
in highp vec4 a_position;\n\
in highp vec2 a_coord;\n\
out highp vec2 v_coord;\n\
void main (void)\n\
{\n\
    gl_Position = a_position;\n\
    v_coord = a_coord;\n\
}\n";

/// The vertex shader of `TextureCubeShader` (:464-473).
const VS_TEX_CUBE: &str = "#version 300 es\n\
in highp vec4 a_position;\n\
in mediump vec2 a_coord;\n\
uniform mat3 u_coordMat;\n\
out mediump vec3 v_coord;\n\
void main (void)\n\
{\n\
    gl_Position = a_position;\n\
    v_coord = u_coordMat * vec3(a_coord, 1.0);\n\
}\n";

/// The vertex shader of texture.units (es3fTextureUnitTests.cpp:207-216).
const VS_UNITS: &str = "#version 300 es\n\
in highp vec4 a_position;\n\
in highp vec2 a_coord;\n\
out highp vec2 v_coord;\n\
\n\
void main (void)\n\
{\n\
    gl_Position = a_position;\n\
    v_coord = a_coord;\n\
}\n";

/// es3fFboTestUtil.cpp `FlatColorShader` fragment shader (:105-117).
fn flat_color(out: &str) -> (String, String) {
    (
        VS_POSITION.into(),
        format!(
            "#version 300 es\n\
uniform highp vec4 u_color;\n\
layout(location = 0) out highp {out} o_color;\n\
void main (void)\n\
{{\n\
    o_color = {out}(u_color);\n\
}}\n"
        ),
    )
}

/// es3fFboTestUtil.cpp `GradientShader` fragment shader (:187-206).
fn gradient(out: &str) -> (String, String) {
    (
        VS_GRADIENT.into(),
        format!(
            "#version 300 es\n\
in highp vec4 v_coord;\n\
uniform highp vec4 u_gradientMin;\n\
uniform highp vec4 u_gradientMax;\n\
layout(location = 0) out highp {out} o_color;\n\
void main (void)\n\
{{\n\
    highp float x = v_coord.x;\n\
    highp float y = v_coord.y;\n\
    highp float f0 = (x + y) * 0.5;\n\
    highp float f1 = 0.5 + (x - y) * 0.5;\n\
    highp vec4 fv = vec4(f0, f1, 1.0f-f0, 1.0f-f1);\n\
    o_color = {out}(u_gradientMin + (u_gradientMax-u_gradientMin)*fv);\n\
}}\n"
        ),
    )
}

/// es3fFboTestUtil.cpp `DepthGradientShader` fragment shader (:838-860), the
/// `gl_FragDepth` one.
fn depth_gradient(out: &str) -> (String, String) {
    (
        VS_GRADIENT.into(),
        format!(
            "#version 300 es\n\
in highp vec4 v_coord;\n\
uniform highp float u_minGradient;\n\
uniform highp float u_maxGradient;\n\
uniform highp vec4 u_color;\n\
layout(location = 0) out highp {out} o_color;\n\
void main (void)\n\
{{\n\
    highp float x = v_coord.x;\n\
    highp float y = v_coord.y;\n\
    highp float f0 = (x + y) * 0.5;\n\
    gl_FragDepth = u_minGradient + (u_maxGradient-u_minGradient)*f0;\n\
    o_color = {out}(u_color);\n\
}}\n"
        ),
    )
}

/// es3fFboTestUtil.cpp `genTexFragmentShader` (:260-298): one or more
/// samplers, summed, then the output scale and bias.
fn tex_2d(samplers: &[&str], out: &str) -> (String, String) {
    let mut fs = format!("#version 300 es\nlayout(location = 0) out highp {out} o_color0;\nin highp vec2 v_coord;\n");
    for (i, s) in samplers.iter().enumerate() {
        fs += &format!(
            "uniform highp {s} u_sampler{i};\nuniform highp vec4 u_texScale{i};\nuniform highp vec4 u_texBias{i};\n"
        );
    }
    fs += "uniform highp vec4 u_outScale0;\nuniform highp vec4 u_outBias0;\n\nvoid main (void)\n{\n    highp vec4 out0 = vec4(0.0);\n";
    for i in 0..samplers.len() {
        fs += &format!("\tout0 += vec4(texture(u_sampler{i}, v_coord)) * u_texScale{i} + u_texBias{i};\n");
    }
    fs += &format!("    o_color0 = {out}(out0 * u_outScale0 + u_outBias0);\n}}\n");
    (VS_TEX_2D.into(), fs)
}

/// es3fFboTestUtil.cpp `TextureCubeShader` fragment shader (:474-494).
fn tex_cube(sampler: &str, out: &str) -> (String, String) {
    (
        VS_TEX_CUBE.into(),
        format!(
            "#version 300 es\n\
uniform highp {sampler} u_sampler0;\n\
uniform highp vec4 u_scale;\n\
uniform highp vec4 u_bias;\n\
in mediump vec3 v_coord;\n\
layout(location = 0) out highp {out} o_color;\n\
void main (void)\n\
{{\n\
    o_color = {out}(vec4(texture(u_sampler0, v_coord)) * u_scale + u_bias);\n\
}}\n"
        ),
    )
}

/// es3fFboTestUtil.cpp `Texture2DArrayShader` (:605-627) and `Texture3DShader`
/// (:721-743) fragment shaders: the layer is an `int`, the depth a `float`.
fn tex_layered(sampler: &str, out: &str, layer_ty: &str, layer: &str) -> (String, String) {
    (
        VS_TEX_LAYERED.into(),
        format!(
            "#version 300 es\n\
uniform highp {sampler} u_sampler0;\n\
uniform highp vec4 u_scale;\n\
uniform highp vec4 u_bias;\n\
uniform highp {layer_ty} u_{layer};\n\
in highp vec2 v_coord;\n\
layout(location = 0) out highp {out} o_color;\n\
void main (void)\n\
{{\n\
    o_color = {out}(vec4(texture(u_sampler0, vec3(v_coord, u_{layer}))) * u_scale + u_bias);\n\
}}\n"
        ),
    )
}

/// es3fTextureUnitTests.cpp `generateMultiTexFragmentShader` (:132-181).
/// `units`: (is 2D, sampler type name) per unit.
fn multi_tex(units: &[(bool, String)]) -> (String, String) {
    let n = units.len();
    let (mut samplers, mut mats, mut scales, mut biases, mut lookups) =
        (String::new(), String::new(), String::new(), String::new(), String::new());
    let mul = format!("(1.0/{n}.0)");
    for (i, (is_2d, ty)) in units.iter().enumerate() {
        samplers += &format!("uniform highp {ty} u_sampler{i};\n");
        mats += &format!("uniform highp mat4 u_trans{i};\n");
        scales += &format!("uniform highp vec4 u_texScale{i};\n");
        biases += &format!("uniform highp vec4 u_texBias{i};\n");
        let coord = format!("u_trans{i}*vec4(v_coord, 1.0, 1.0)");
        let coord = if *is_2d { format!("vec2({coord})") } else { format!("vec3({coord})") };
        lookups += &format!("\tcolor += {mul}*(vec4(texture(u_sampler{i}, {coord}))*u_texScale{i} + u_texBias{i});\n");
    }
    (
        VS_UNITS.into(),
        format!(
            "#version 300 es\n\
layout(location = 0) out mediump vec4 o_color;\n\
{samplers}{mats}{scales}{biases}\
in highp vec2 v_coord;\n\
\n\
void main (void)\n\
{{\n\
    mediump vec4 color = vec4(0.0);\n\
{lookups}\
    o_color = color;\n\
}}\n"
        ),
    )
}

/* ------------------------------------------------------------- tests -- */

/* One test, one thread: a suite of parallel tests on fresh threads failed its
 * first compile at random (empty log; alone or with --test-threads=1 it
 * passes), so the groups run in turn inside one test. */

/// fbo.color and texture.specification (gradient): every output type.
fn flat_color_and_gradient_shaders_build() {
    let mut p = Vec::new();
    for o in OUT_TYPES {
        p.push(flat_color(o));
        p.push(gradient(o));
    }
    assert_eq!(build_all(p), 6);
}

/// fbo.depth: the `gl_FragDepth` gradient shader.
fn the_gl_frag_depth_gradient_shader_builds() {
    assert_eq!(build_all(OUT_TYPES.iter().map(|o| depth_gradient(o)).collect()), 3);
}

/// texture.specification and fbo.color: `Texture2DShader` with one sampler
/// of any class and any output type, and the two-sampler one of
/// `multiTexShader`.
fn texture_2d_shaders_build() {
    let mut p = Vec::new();
    for c in CLASSES {
        let s = format!("{c}sampler2D");
        for o in OUT_TYPES {
            p.push(tex_2d(&[&s], o));
            for c2 in CLASSES {
                let s2 = format!("{c2}sampler2D");
                p.push(tex_2d(&[&s, &s2], o));
            }
        }
    }
    assert_eq!(build_all(p), 36);
}

/// texture.specification and fbo.color: the cube, 2D-array and 3D shaders.
fn texture_cube_array_and_3d_shaders_build() {
    let mut p = Vec::new();
    for c in CLASSES {
        for o in OUT_TYPES {
            p.push(tex_cube(&format!("{c}samplerCube"), o));
            p.push(tex_layered(&format!("{c}sampler2DArray"), o, "int", "layer"));
            p.push(tex_layered(&format!("{c}sampler3D"), o, "float", "depth"));
        }
    }
    assert_eq!(build_all(p), 27);
}

/// texture.units: 2, 4, 8 and 16 units, alone per target and mixed, with
/// each sampler class.
fn texture_units_shaders_build() {
    // (is 2D, sampler type name) of a target.
    let kinds = [(true, "sampler2D"), (false, "samplerCube"), (false, "sampler2DArray"), (false, "sampler3D")];
    let unit = |k: usize, c: usize| (kinds[k].0, format!("{}{}", CLASSES[c], kinds[k].1));
    let mut p = Vec::new();
    for n in [2usize, 4, 8, 16] {
        for c in 0..CLASSES.len() {
            for k in 0..kinds.len() {
                p.push(multi_tex(&(0..n).map(|_| unit(k, c)).collect::<Vec<_>>()));
            }
            // mixed: the targets in turn, the classes shifted.
            p.push(multi_tex(&(0..n).map(|i| unit(i % kinds.len(), (c + i) % CLASSES.len())).collect::<Vec<_>>()));
            p.push(multi_tex(&(0..n).map(|i| unit((i + c) % kinds.len(), 0)).collect::<Vec<_>>()));
        }
    }
    build_all(p);
}

#[test]
fn the_deqp_sglr_shaders_compile_and_link() {
    flat_color_and_gradient_shaders_build();
    the_gl_frag_depth_gradient_shader_builds();
    texture_2d_shaders_build();
    texture_cube_array_and_3d_shaders_build();
    texture_units_shaders_build();
}
