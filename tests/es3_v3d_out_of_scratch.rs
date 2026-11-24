// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014 The Android Open Source Project
// SPDX-FileCopyrightText: 2017 Alastair F. Donaldson and Hugues Evrard, Imperial College London
//! The V3D compiler's scratch-arena refusal (plan #380, issue #1261; images
//! S16 and UA1 of tools/cts/pi4-gles3-tree.txt). On the Pi 4 at b1b5b845
//! dEQP-GLES3 `functional.shaders.metamorphic.synthetic.variant_3` (S16)
//! failed at glLinkProgram with
//!
//! ```text
//! online shaderc: the code generator refused this program (vertex + fragment): Program(Select(OutOfScratch))
//! ```
//!
//! The V3D compiles the linked program, not a stage (`link_sources`,
//! src/shaderc/online/backend.rs): both stages compile, the link fails with
//! -ENOTSUP and that text. It is the compiler's instruction selection
//! running out of its scratch arena, not a defect of the generated code (the
//! host tables list it as RC13, code generator). The program is small:
//! `recipient.frag`, the same program without the three early exits
//! (`if (injectionSwitch.x > injectionSwitch.y) { break; / return; }`),
//! links (the control). The fix belongs to plan #365
//! (src/shaderc/online/v3d/**); `variant_3_is_refused_with_select_out_of_scratch`
//! flips to `Outcome::Linked` then.
//!
//! UA1 `uniform_api.random.83` (same refusal on the board) is generated from
//! a seed by dEQP's uniform_api random generator (the shaders below are its
//! logged output). Its vertex shader is 13.8 KB (50 active uniforms: struct
//! arrays of matrices and samplers); linking it needs about 7.5 MB
//! of scratch. `random_83_is_refused_with_select_out_of_scratch` pins the
//! refusal and flips to `Outcome::Linked` with variant_3 when #365 fixes
//! the arena; `random_25_links` (a 2.6 KB fragment-side case of the same
//! group) is the control.
//!
//! Both refusals are at the scratch the compiler gets by default
//! (`store::SCRATCH_BYTES`, 4 MiB, the Kconfig default). Since f2fb64c9 the
//! Pi 4's dEQP images lend it 8 MiB (`driver/samples/zephyr/deqp/boards/rpi_4b.conf`),
//! and at 8 MiB both programs link (`*_links_in_the_pi4_images_8_mib`), so
//! S16 and UA1 are images of the Pi 4 rerun (plan #380). Each build lends
//! its region under [`COMPILING`], so the two sizes never meet.
//!
//! The variant_3 shader text is dEQP-GLES3's: VK-GL-CTS 93bca01
//! `external/graphicsfuzz/data/gles3/graphicsfuzz/` (`vertexShader.glsl`,
//! `synthetic/recipient.frag`, `synthetic/variant_3.frag`), Apache-2.0,
//! Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College
//! London, 2017 (GraphicsFuzz), distributed by The Khronos Group Inc.
//! (Apache-2.0, as tools/mali/testdata/deqp_*.license).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_out_of_scratch
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/// The refusal glLinkProgram reports, verbatim.
const OUT_OF_SCRATCH: &str =
    "online shaderc: the code generator refused this program (vertex + fragment): Program(Select(OutOfScratch))";

/// `vertexShader.glsl`, verbatim.
const VERTEX: &str = r####"#version 100

// Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College London, 2017

attribute vec2 coord2d;

varying vec2 surfacePosition;

void main(void)
{
    gl_Position = vec4(coord2d, 0.0, 1.0);
    surfacePosition = coord2d;
}
"####;

/// `synthetic/recipient.frag`, verbatim.
const RECIPIENT: &str = r####"#version 100

// Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College London, 2017

#ifdef GL_ES
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
precision highp int;
#else
precision mediump float;
precision mediump int;
#endif
#endif

uniform vec2 injectionSwitch;

void main()
{
    int r;
    int g;
    int b;
    int a;
    r = 100 * int(injectionSwitch.y);
    g = int(injectionSwitch.x) * int(injectionSwitch.y);
    b = 2 * int(injectionSwitch.x);
    a = g - int(injectionSwitch.x);
    for(
        int i = 0;
        i < 10;
        i ++
    )
        {
            r --;
            g ++;
            b ++;
            a ++;
            for(
                int j = 1;
                j < 10;
                j ++
            )
                {
                    a ++;
                    b ++;
                    g ++;
                    r --;
                }
        }
    float fr;
    float fg;
    float fb;
    float fa;
    fr = float(r / 100);
    fg = float(g / 100);
    fb = float(b / 100);
    fa = float(a / 100);
    gl_FragColor = vec4(r, g, b, a);
}
"####;

/// `synthetic/variant_3.frag`, verbatim.
const VARIANT_3: &str = r####"#version 100

// Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College London, 2017
// Defect found using GLFuzz - https://www.graphicsfuzz.com/
//
// Causes black image to be rendered on:
// ASUS ZenPad 3S 10
// Model number: P027
// GL_VERSION: OpenGL ES 3.1 build 1.5@3830101
// GL_VENDOR: Imagination Technologies
// GL_RENDERER: PowerVR Rogue GX6250
// Android version: 6.0

#ifdef GL_ES
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
precision highp int;
#else
precision mediump float;
precision mediump int;
#endif
#endif

uniform vec2 injectionSwitch;

void main()
{
    int r;
    int g;
    int b;
    int a;
    r = 100 * int(injectionSwitch.y);
    g = int(injectionSwitch.x) * int(injectionSwitch.y);
    b = 2 * int(injectionSwitch.x);
    a = g - int(injectionSwitch.x);
    for(
        int i = 0;
        i < 10;
        i ++
    )
        {
            r --;
            g ++;
            b ++;
            a ++;
            for(
                int j = 1;
                j < 10;
                j ++
            )
                {
                    if(injectionSwitch.x > injectionSwitch.y)
                        {
                            break;
                        }
                    a ++;
                    if(injectionSwitch.x > injectionSwitch.y)
                        {
                            break;
                        }
                    b ++;
                    g ++;
                    r --;
                    if(injectionSwitch.x > injectionSwitch.y)
                        {
                            return;
                        }
                }
        }
    float fr;
    float fg;
    float fb;
    float fa;
    fr = float(r / 100);
    fg = float(g / 100);
    fb = float(b / 100);
    fa = float(a / 100);
    gl_FragColor = vec4(r, g, b, a);
}
"####;

/// dEQP-GLES3's `uniform_api.random.N` shaders are generated by
/// `UniformCollection::random(N)` and `UniformCase` in VK-GL-CTS 93bca01
/// `modules/gles3/functional/es3fUniformApiTests.cpp` (Apache-2.0, Copyright
/// 2014 The Android Open Source Project). The text below is what that case
/// hands glShaderSource, logged by the module's null-context build
/// (`--deqp-log-shader-sources=enable`), HTML entities decoded.
/// `uniform_api.random.83`, vertex stage, as the case logs it (13,794 bytes).
const RANDOM_83_VERTEX: &str = r####"#version 300 es
in highp vec4 a_position;
out mediump float v_vtxOut;

struct structType2
{
	mediump vec2 m0;
	mediump mat4x2 m1;
	mediump mat4x3 m2;
	mediump float m3[3];
};
struct structType1
{
	structType2 m0;
	mediump mat3 m1;
};
struct structType4
{
	mediump mat3 m0[3];
	mediump sampler2D m1;
	mediump mat4 m2[5];
};
struct structType3
{
	mediump uint m0;
	structType4 m1;
	mediump samplerCube m2;
};
struct structType6
{
	mediump mat4x2 m0;
	bvec3 m1;
};
struct structType5
{
	mediump ivec3 m0;
	structType6 m1;
};
struct structType0
{
	structType1 m0[1];
	structType3 m1[5];
	mediump mat3x2 m2[1];
	structType5 m3[2];
};
struct structType9
{
	mediump int m0;
	mediump sampler2D m1;
	bool m2[1];
	mediump mat4x3 m3[5];
};
struct structType8
{
	mediump sampler2D m0[1];
	structType9 m1;
	mediump float m2;
	mediump sampler2D m3;
};
struct structType7
{
	mediump mat2x3 m0[4];
	structType8 m1;
	mediump mat2 m2;
	mediump uvec3 m3[5];
	mediump ivec2 m4;
};
uniform structType0 u_var0;
uniform mediump mat2x3 u_var1[3];
uniform structType7 u_var2;
uniform mediump vec2 u_var3;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat2x3   (mediump mat2x3 a, mediump mat2x3 b){ return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1]); }
mediump float compare_mat3x2   (mediump mat3x2 a, mediump mat3x2 b){ return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1])*compare_vec2(a[2], b[2]); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_mat4x2   (mediump mat4x2 a, mediump mat4x2 b){ return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1])*compare_vec2(a[2], b[2])*compare_vec2(a[3], b[3]); }
mediump float compare_mat4x3   (mediump mat4x3 a, mediump mat4x3 b){ return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2])*compare_vec3(a[3], b[3]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec2    (mediump ivec2 a, mediump ivec2 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_uint     (mediump uint a, mediump uint b)    { return a == b ? 1.0 : 0.0; }
mediump float compare_uvec3    (mediump uvec3 a, mediump uvec3 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec3    (bvec3 a, bvec3 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_vec2(u_var0.m0[0].m0.m0, vec2(-3.17, -7.50));
	v_vtxOut *= compare_mat4x2(u_var0.m0[0].m0.m1, mat4x2(2.28, 3.39, -0.00, 6.54, -0.13, 7.17, -1.29, 4.74));
	v_vtxOut *= compare_mat4x3(u_var0.m0[0].m0.m2, mat4x3(-9.02, 0.15, -7.65, 9.51, 1.33, -5.36, -8.64, -2.99, 4.24, 0.04, -2.19, -2.23));
	v_vtxOut *= compare_float(u_var0.m0[0].m0.m3[0], 4.00);
	v_vtxOut *= compare_float(u_var0.m0[0].m0.m3[1], -5.85);
	v_vtxOut *= compare_float(u_var0.m0[0].m0.m3[2], -0.63);
	v_vtxOut *= compare_mat3(u_var0.m0[0].m1, mat3(-6.49, 8.06, -7.91, -5.52, -5.05, -1.29, 0.42, -9.55, -1.66));
	v_vtxOut *= compare_uint(u_var0.m1[0].m0, 4u);
	v_vtxOut *= compare_mat3(u_var0.m1[0].m1.m0[0], mat3(9.18, 2.03, -5.39, 1.92, 2.92, 4.43, 0.32, -7.22, 4.10));
	v_vtxOut *= compare_mat3(u_var0.m1[0].m1.m0[1], mat3(1.49, -0.12, 1.51, -3.52, 1.66, -0.99, -4.80, 3.12, -4.86));
	v_vtxOut *= compare_mat3(u_var0.m1[0].m1.m0[2], mat3(-2.03, -6.61, -2.53, 0.69, 1.47, -1.76, 5.30, 4.80, 7.47));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[0].m1.m1, vec2(0.0)), vec4(0.21, 0.36, 0.99, 0.06));
	v_vtxOut *= compare_mat4(u_var0.m1[0].m1.m2[0], mat4(-3.26, 8.66, -0.77, -0.74, 6.06, 0.44, -9.06, 6.00, 1.23, 3.35, -6.08, -5.95, -3.17, 4.65, -5.91, -3.96));
	v_vtxOut *= compare_mat4(u_var0.m1[0].m1.m2[1], mat4(-9.48, 6.24, -3.80, -4.10, 0.79, 5.48, -5.82, -7.03, 6.11, 3.15, 4.41, 1.11, 9.95, 9.29, -9.84, 6.23));
	v_vtxOut *= compare_mat4(u_var0.m1[0].m1.m2[2], mat4(4.74, 0.27, -1.03, 6.82, -8.47, 5.59, 0.32, 6.80, -0.99, -7.36, -3.26, -4.54, 4.57, -9.54, -9.55, -6.01));
	v_vtxOut *= compare_mat4(u_var0.m1[0].m1.m2[3], mat4(-6.69, 8.42, -1.38, 8.98, 5.77, 2.46, -5.08, 9.51, -9.85, -3.96, -7.18, 2.23, 0.70, -7.67, 4.07, -7.61));
	v_vtxOut *= compare_mat4(u_var0.m1[0].m1.m2[4], mat4(-0.43, 2.18, -4.32, 6.45, 2.50, 8.76, -4.14, 4.94, -1.82, 9.76, 6.27, 9.45, 8.55, 1.33, 6.16, -1.25));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[0].m2, vec3(0.0)), vec4(0.95, 0.04, 0.35, 0.74));
	v_vtxOut *= compare_uint(u_var0.m1[1].m0, 5u);
	v_vtxOut *= compare_mat3(u_var0.m1[1].m1.m0[0], mat3(9.79, 0.85, 2.78, -0.69, -3.63, 6.34, -9.67, -6.02, 8.29));
	v_vtxOut *= compare_mat3(u_var0.m1[1].m1.m0[1], mat3(0.56, 1.76, -4.22, -1.97, -6.63, 8.34, -1.13, 4.36, 4.58));
	v_vtxOut *= compare_mat3(u_var0.m1[1].m1.m0[2], mat3(-7.16, 9.54, -1.49, 3.87, -3.40, 1.86, 9.90, -6.81, 1.03));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[1].m1.m1, vec2(0.0)), vec4(0.34, 0.52, 0.31, 0.20));
	v_vtxOut *= compare_mat4(u_var0.m1[1].m1.m2[0], mat4(7.02, -3.32, 9.74, -2.66, 0.08, -6.08, 1.19, -5.73, 3.84, -2.28, -9.14, -0.75, 3.47, -0.59, -9.89, 7.92));
	v_vtxOut *= compare_mat4(u_var0.m1[1].m1.m2[1], mat4(-2.69, 9.50, -6.06, 5.91, 9.07, -6.66, 5.30, 1.67, -2.65, 9.66, 0.24, -8.41, -9.39, 8.27, -8.12, -3.47));
	v_vtxOut *= compare_mat4(u_var0.m1[1].m1.m2[2], mat4(-8.99, 9.62, 8.98, -7.49, 7.39, 0.09, -3.73, 9.06, 6.34, -6.13, -8.40, -9.55, 4.21, -6.69, -5.19, 8.36));
	v_vtxOut *= compare_mat4(u_var0.m1[1].m1.m2[3], mat4(-6.40, -2.35, -6.54, 1.47, 7.91, -7.46, -3.32, -0.22, 0.13, 6.65, 2.92, 6.62, 1.45, 7.07, 0.26, -6.15));
	v_vtxOut *= compare_mat4(u_var0.m1[1].m1.m2[4], mat4(8.87, 4.59, -9.92, 5.59, 9.21, -6.27, -8.13, -5.41, -7.13, -4.82, -3.26, -1.85, 8.98, -8.54, -1.08, 8.18));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[1].m2, vec3(0.0)), vec4(0.08, 0.05, 0.63, 0.53));
	v_vtxOut *= compare_uint(u_var0.m1[2].m0, 1u);
	v_vtxOut *= compare_mat3(u_var0.m1[2].m1.m0[0], mat3(-9.48, 7.37, -6.36, 1.54, -3.01, 1.49, 7.32, 7.10, 5.12));
	v_vtxOut *= compare_mat3(u_var0.m1[2].m1.m0[1], mat3(-6.81, 2.94, 8.59, -3.45, -8.50, -1.24, -9.83, 3.19, -4.14));
	v_vtxOut *= compare_mat3(u_var0.m1[2].m1.m0[2], mat3(-9.03, -4.07, 9.58, -0.68, 9.97, -2.95, -3.55, -7.85, -9.56));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[2].m1.m1, vec2(0.0)), vec4(0.84, 0.50, 0.38, 0.68));
	v_vtxOut *= compare_mat4(u_var0.m1[2].m1.m2[0], mat4(-1.41, 7.57, -0.69, -0.04, 5.47, -6.54, 5.99, 6.41, 7.70, -9.75, -0.33, 6.57, -0.14, 3.74, -8.03, 4.19));
	v_vtxOut *= compare_mat4(u_var0.m1[2].m1.m2[1], mat4(-2.34, -2.77, -3.62, -0.93, -7.11, 0.81, 2.39, 6.78, -6.61, -0.73, -2.53, -4.66, -4.34, 3.35, 9.27, 1.37));
	v_vtxOut *= compare_mat4(u_var0.m1[2].m1.m2[2], mat4(-9.23, 5.41, 3.54, 1.86, -7.26, -6.62, -7.48, 7.64, 7.55, -4.48, 7.87, -8.88, 0.94, 0.76, -5.39, 7.72));
	v_vtxOut *= compare_mat4(u_var0.m1[2].m1.m2[3], mat4(7.79, -2.12, -5.82, -2.48, -9.08, -7.57, 3.22, -2.47, -0.74, -8.11, 8.70, 9.36, -8.48, -9.17, 2.63, -9.79));
	v_vtxOut *= compare_mat4(u_var0.m1[2].m1.m2[4], mat4(-5.27, -2.26, 8.56, 4.78, 1.96, 4.15, -5.42, 8.67, -6.59, -1.87, 0.39, -7.60, -0.69, -8.00, -1.19, 6.90));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[2].m2, vec3(0.0)), vec4(0.28, 0.31, 0.24, 0.71));
	v_vtxOut *= compare_uint(u_var0.m1[3].m0, 8u);
	v_vtxOut *= compare_mat3(u_var0.m1[3].m1.m0[0], mat3(2.22, -5.32, -1.86, 8.39, 8.13, 5.43, -0.76, 8.57, 0.92));
	v_vtxOut *= compare_mat3(u_var0.m1[3].m1.m0[1], mat3(6.77, -2.60, -9.92, -3.80, -6.79, 1.71, 9.31, -7.65, -1.98));
	v_vtxOut *= compare_mat3(u_var0.m1[3].m1.m0[2], mat3(-5.93, 1.65, 7.79, 9.58, -3.37, -6.29, 9.75, -8.36, 8.14));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[3].m1.m1, vec2(0.0)), vec4(0.90, 0.95, 0.83, 0.61));
	v_vtxOut *= compare_mat4(u_var0.m1[3].m1.m2[0], mat4(2.17, 9.29, -7.41, -2.55, -1.31, -9.94, 5.83, -6.95, -4.79, 9.75, 7.70, 8.36, -0.17, 1.98, 7.69, 1.61));
	v_vtxOut *= compare_mat4(u_var0.m1[3].m1.m2[1], mat4(-9.88, 9.09, 5.01, 0.82, 9.89, 1.77, -5.50, -5.16, 9.48, -4.22, -1.67, 4.23, 6.65, 2.10, -4.24, 4.57));
	v_vtxOut *= compare_mat4(u_var0.m1[3].m1.m2[2], mat4(-2.47, -6.53, 9.29, -6.83, -2.90, -7.18, 7.62, 2.25, -2.40, 1.51, 9.59, -8.37, 7.52, 4.87, -0.91, -1.63));
	v_vtxOut *= compare_mat4(u_var0.m1[3].m1.m2[3], mat4(-9.55, -1.69, 6.09, -5.78, -1.66, 0.42, -7.22, 1.20, -1.56, 4.80, 5.53, -2.65, 9.75, 6.87, -6.55, 8.48));
	v_vtxOut *= compare_mat4(u_var0.m1[3].m1.m2[4], mat4(4.59, 0.74, -6.77, -0.54, 0.30, 6.30, -5.61, -2.68, 3.62, 2.56, -3.45, -5.89, -9.40, 5.69, -3.76, -5.43));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[3].m2, vec3(0.0)), vec4(0.82, 0.80, 0.56, 0.19));
	v_vtxOut *= compare_uint(u_var0.m1[4].m0, 10u);
	v_vtxOut *= compare_mat3(u_var0.m1[4].m1.m0[0], mat3(-4.83, -2.57, -0.27, -7.73, 7.35, -8.94, -0.66, -4.27, -3.47));
	v_vtxOut *= compare_mat3(u_var0.m1[4].m1.m0[1], mat3(0.10, 2.95, 6.86, -2.08, -0.95, -0.52, -1.44, 5.22, 0.80));
	v_vtxOut *= compare_mat3(u_var0.m1[4].m1.m0[2], mat3(4.48, -0.15, 6.36, 9.30, 8.46, 7.60, 0.86, -1.74, 2.25));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[4].m1.m1, vec2(0.0)), vec4(0.78, 0.94, 0.99, 0.54));
	v_vtxOut *= compare_mat4(u_var0.m1[4].m1.m2[0], mat4(-4.88, -2.30, 3.55, -4.12, -2.20, -9.58, -7.10, -2.72, 7.11, -3.44, 9.65, -9.37, -5.46, 0.23, 6.73, -3.77));
	v_vtxOut *= compare_mat4(u_var0.m1[4].m1.m2[1], mat4(-1.14, 1.38, 1.53, 2.03, -2.79, -3.33, -4.71, -7.56, -5.99, -6.91, 5.73, -9.62, -3.74, 8.92, 1.06, 5.30));
	v_vtxOut *= compare_mat4(u_var0.m1[4].m1.m2[2], mat4(-7.39, 1.33, 9.26, 1.27, -8.37, 3.33, 1.85, -2.14, 0.25, -3.48, -1.39, -7.57, 8.59, 1.16, -0.16, 8.90));
	v_vtxOut *= compare_mat4(u_var0.m1[4].m1.m2[3], mat4(5.31, -6.85, -9.59, 8.60, -5.48, 3.53, 8.20, 8.57, -4.52, 5.49, -5.24, 0.77, 7.88, 4.53, 4.97, -7.34));
	v_vtxOut *= compare_mat4(u_var0.m1[4].m1.m2[4], mat4(2.77, -2.42, -2.11, 3.45, 3.03, -0.75, -1.17, 3.19, -8.77, 9.46, -9.47, 9.91, 3.79, -4.14, -8.94, -1.71));
	v_vtxOut *= compare_vec4(texture(u_var0.m1[4].m2, vec3(0.0)), vec4(0.35, 0.04, 0.47, 0.72));
	v_vtxOut *= compare_mat3x2(u_var0.m2[0], mat3x2(-2.57, -6.93, -3.95, -3.25, 3.14, -9.95));
	v_vtxOut *= compare_ivec3(u_var0.m3[0].m0, ivec3(10, 3, 1));
	v_vtxOut *= compare_mat4x2(u_var0.m3[0].m1.m0, mat4x2(-0.78, 7.74, -2.08, 7.86, -8.91, -4.66, 9.73, 2.31));
	v_vtxOut *= compare_bvec3(u_var0.m3[0].m1.m1, bvec3(false, true, true));
	v_vtxOut *= compare_ivec3(u_var0.m3[1].m0, ivec3(2, -5, -5));
	v_vtxOut *= compare_mat4x2(u_var0.m3[1].m1.m0, mat4x2(-6.19, -2.03, -4.74, 7.16, 5.41, 7.62, 3.79, -0.72));
	v_vtxOut *= compare_bvec3(u_var0.m3[1].m1.m1, bvec3(true, true, true));
	v_vtxOut *= compare_mat2x3(u_var1[0], mat2x3(8.85, 1.67, 1.14, -5.91, 7.45, 9.12));
	v_vtxOut *= compare_mat2x3(u_var1[1], mat2x3(-6.45, -2.04, -7.40, -7.58, -0.11, 4.85));
	v_vtxOut *= compare_mat2x3(u_var1[2], mat2x3(0.69, 6.91, -0.03, 7.10, -5.33, -6.99));
	v_vtxOut *= compare_mat2x3(u_var2.m0[0], mat2x3(7.25, 8.50, -9.09, 4.02, -5.65, 4.52));
	v_vtxOut *= compare_mat2x3(u_var2.m0[1], mat2x3(3.20, 3.11, 9.51, -4.89, -3.34, -3.92));
	v_vtxOut *= compare_mat2x3(u_var2.m0[2], mat2x3(-1.74, 6.18, 6.22, 8.91, 8.29, 3.78));
	v_vtxOut *= compare_mat2x3(u_var2.m0[3], mat2x3(2.64, -0.41, -5.22, 9.19, 5.22, -1.26));
	v_vtxOut *= compare_vec4(texture(u_var2.m1.m0[0], vec2(0.0)), vec4(0.59, 0.21, 0.57, 0.24));
	v_vtxOut *= compare_int(u_var2.m1.m1.m0, 9);
	v_vtxOut *= compare_vec4(texture(u_var2.m1.m1.m1, vec2(0.0)), vec4(0.18, 0.00, 0.75, 0.58));
	v_vtxOut *= compare_bool(u_var2.m1.m1.m2[0], true);
	v_vtxOut *= compare_mat4x3(u_var2.m1.m1.m3[0], mat4x3(1.72, 6.19, 1.25, -4.45, -0.24, 1.08, 1.13, 0.53, 4.34, -2.78, 4.50, 5.52));
	v_vtxOut *= compare_mat4x3(u_var2.m1.m1.m3[1], mat4x3(-2.23, 8.65, -4.74, 7.53, 8.79, 0.57, 5.63, 9.28, 3.52, 5.11, -6.88, 2.05));
	v_vtxOut *= compare_mat4x3(u_var2.m1.m1.m3[2], mat4x3(-2.85, -5.16, 6.05, 8.28, 5.75, -1.91, 7.47, 3.95, -4.55, 5.50, -3.12, -8.75));
	v_vtxOut *= compare_mat4x3(u_var2.m1.m1.m3[3], mat4x3(-3.31, -3.31, -7.40, -5.02, -1.82, -8.78, 4.39, 1.12, 9.38, 5.07, -4.79, 5.57));
	v_vtxOut *= compare_mat4x3(u_var2.m1.m1.m3[4], mat4x3(8.23, -1.26, -5.88, 8.17, -8.80, -8.67, 7.90, 8.68, 3.56, -2.08, -5.63, 2.44));
	v_vtxOut *= compare_float(u_var2.m1.m2, 8.60);
	v_vtxOut *= compare_vec4(texture(u_var2.m1.m3, vec2(0.0)), vec4(0.29, 0.49, 0.79, 0.47));
	v_vtxOut *= compare_mat2(u_var2.m2, mat2(-6.96, 8.84, 2.66, 1.98));
	v_vtxOut *= compare_uvec3(u_var2.m3[0], uvec3(5u, 4u, 1u));
	v_vtxOut *= compare_uvec3(u_var2.m3[1], uvec3(10u, 1u, 3u));
	v_vtxOut *= compare_uvec3(u_var2.m3[2], uvec3(9u, 9u, 3u));
	v_vtxOut *= compare_uvec3(u_var2.m3[3], uvec3(2u, 8u, 3u));
	v_vtxOut *= compare_uvec3(u_var2.m3[4], uvec3(9u, 3u, 5u));
	v_vtxOut *= compare_ivec2(u_var2.m4, ivec2(-5, 1));
	v_vtxOut *= compare_vec2(u_var3, vec2(-9.54, 6.88));
}
"####;

/// `uniform_api.random.83`, fragment stage.
const RANDOM_83_FRAGMENT: &str = r####"#version 300 es
in mediump float v_vtxOut;


layout(location = 0) out mediump vec4 dEQP_FragColor;

void main (void)
{
    mediump float result = v_vtxOut;
    dEQP_FragColor = vec4(result, result, result, 1.0);
}
"####;

/// `uniform_api.random.25`, vertex stage.
const RANDOM_25_VERTEX: &str = r####"#version 300 es
in highp vec4 a_position;
out mediump float v_vtxOut;

struct structType2
{
	mediump mat2x3 m0;
	mediump uvec2 m1;
	mediump mat4x3 m2;
	mediump float m3;
};
struct structType1
{
	bool m0;
	bool m1;
	structType2 m2;
};
struct structType0
{
	structType1 m0[4];
};
uniform structType0 u_var0;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_mat2x3   (mediump mat2x3 a, mediump mat2x3 b){ return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1]); }
mediump float compare_mat4x3   (mediump mat4x3 a, mediump mat4x3 b){ return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2])*compare_vec3(a[3], b[3]); }
mediump float compare_uvec2    (mediump uvec2 a, mediump uvec2 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_bool(u_var0.m0[0].m0, false);
	// UNUSED: u_var0.m0[0].m1
	v_vtxOut *= compare_mat2x3(u_var0.m0[0].m2.m0, mat2x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[0].m2.m1
	v_vtxOut *= compare_mat4x3(u_var0.m0[0].m2.m2, mat4x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[0].m2.m3
	v_vtxOut *= compare_bool(u_var0.m0[1].m0, false);
	// UNUSED: u_var0.m0[1].m1
	v_vtxOut *= compare_mat2x3(u_var0.m0[1].m2.m0, mat2x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[1].m2.m1
	v_vtxOut *= compare_mat4x3(u_var0.m0[1].m2.m2, mat4x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[1].m2.m3
	v_vtxOut *= compare_bool(u_var0.m0[2].m0, false);
	// UNUSED: u_var0.m0[2].m1
	v_vtxOut *= compare_mat2x3(u_var0.m0[2].m2.m0, mat2x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[2].m2.m1
	v_vtxOut *= compare_mat4x3(u_var0.m0[2].m2.m2, mat4x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[2].m2.m3
	v_vtxOut *= compare_bool(u_var0.m0[3].m0, false);
	// UNUSED: u_var0.m0[3].m1
	v_vtxOut *= compare_mat2x3(u_var0.m0[3].m2.m0, mat2x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[3].m2.m1
	v_vtxOut *= compare_mat4x3(u_var0.m0[3].m2.m2, mat4x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[3].m2.m3
}
"####;

/// `uniform_api.random.25`, fragment stage (the control).
const RANDOM_25_FRAGMENT: &str = r####"#version 300 es
in mediump float v_vtxOut;

struct structType2
{
	mediump mat2x3 m0;
	mediump uvec2 m1;
	mediump mat4x3 m2;
	mediump float m3;
};
struct structType1
{
	bool m0;
	bool m1;
	structType2 m2;
};
struct structType0
{
	structType1 m0[4];
};
uniform structType0 u_var0;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_mat2x3   (mediump mat2x3 a, mediump mat2x3 b){ return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1]); }
mediump float compare_mat4x3   (mediump mat4x3 a, mediump mat4x3 b){ return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2])*compare_vec3(a[3], b[3]); }
mediump float compare_uvec2    (mediump uvec2 a, mediump uvec2 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }

layout(location = 0) out mediump vec4 dEQP_FragColor;

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bool(u_var0.m0[0].m0, false);
	// UNUSED: u_var0.m0[0].m1
	result *= compare_mat2x3(u_var0.m0[0].m2.m0, mat2x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[0].m2.m1
	result *= compare_mat4x3(u_var0.m0[0].m2.m2, mat4x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[0].m2.m3
	result *= compare_bool(u_var0.m0[1].m0, false);
	// UNUSED: u_var0.m0[1].m1
	result *= compare_mat2x3(u_var0.m0[1].m2.m0, mat2x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[1].m2.m1
	result *= compare_mat4x3(u_var0.m0[1].m2.m2, mat4x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[1].m2.m3
	result *= compare_bool(u_var0.m0[2].m0, false);
	// UNUSED: u_var0.m0[2].m1
	result *= compare_mat2x3(u_var0.m0[2].m2.m0, mat2x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[2].m2.m1
	result *= compare_mat4x3(u_var0.m0[2].m2.m2, mat4x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[2].m2.m3
	result *= compare_bool(u_var0.m0[3].m0, false);
	// UNUSED: u_var0.m0[3].m1
	result *= compare_mat2x3(u_var0.m0[3].m2.m0, mat2x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[3].m2.m1
	result *= compare_mat4x3(u_var0.m0[3].m2.m2, mat4x3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m0[3].m2.m3
    dEQP_FragColor = vec4(result, result, result, 1.0);
}
"####;

#[derive(Debug, PartialEq)]
enum Outcome {
    Linked,
    CompileFail(String),
    LinkFail(String),
}

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

/// The scratch the Pi 4's dEQP images lend the compiler, read from their board
/// file so this follows it.
fn pi4_image_scratch() -> usize {
    let conf = include_str!("../driver/samples/zephyr/deqp/boards/rpi_4b.conf");
    let line = conf
        .lines()
        .find_map(|l| l.strip_prefix("CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES="))
        .expect("rpi_4b.conf sets the compiler scratch");
    line.trim().parse().expect("a byte count")
}

/// Lend the compiler a zeroed region of `bytes` (one leaked region a size).
/// Called under [`COMPILING`], so no compile runs in a region another lent.
fn lend(bytes: usize) {
    static REGIONS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());
    let mut regions = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    let at = match regions.iter().find(|r| r.0 == bytes) {
        Some(r) => r.1,
        None => {
            let at = Box::leak(vec![0u8; bytes].into_boxed_slice()).as_mut_ptr() as usize;
            regions.push((bytes, at));
            at
        }
    };
    // SAFETY: a leaked region of `bytes` that only the compiler, serialized
    // by COMPILING, ever touches.
    let rc = unsafe { yakogl::shaderc::online::store::shc_online_lend_scratch(at as *mut u8, bytes) };
    assert_eq!(rc, 0, "lend {bytes} bytes of scratch");
}

/// glCompileShader on both stages, then glLinkProgram, at the default scratch.
fn build(vs: &str, fs: &str) -> Outcome {
    build_in(yakogl::shaderc::online::store::SCRATCH_BYTES, vs, fs)
}

/// [`build`] with `scratch` bytes lent to the compiler.
fn build_in(scratch: usize, vs: &str, fs: &str) -> Outcome {
    let _one = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    lend(scratch);
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = match compile(SHC_STAGE_VERTEX, vs) {
        Ok(o) => o,
        Err(m) => return Outcome::CompileFail(format!("vertex: {m}")),
    };
    let mut fso = match compile(SHC_STAGE_FRAGMENT, fs) {
        Ok(o) => o,
        Err(m) => return Outcome::CompileFail(format!("fragment: {m}")),
    };
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the V3D back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Outcome::Linked
    } else {
        Outcome::LinkFail(text_of(&log))
    }
}

/// S16 variant_3 is refused at link with the scratch-arena text.
/// #365 flips this to `Outcome::Linked`.
#[test]
fn variant_3_is_refused_with_select_out_of_scratch() {
    assert_eq!(build(VERTEX, VARIANT_3), Outcome::LinkFail(OUT_OF_SCRATCH.to_string()));
}

/// Control: the recipient of the same family (no early exits) links.
#[test]
fn the_recipient_of_variant_3_links() {
    assert_eq!(build(VERTEX, RECIPIENT), Outcome::Linked);
}

/// UA1 `uniform_api.random.83` is refused at link with the same scratch-arena
/// text as variant_3. #365 flips this to `Outcome::Linked` when the arena is
/// fixed.
#[test]
fn random_83_is_refused_with_select_out_of_scratch() {
    assert_eq!(build(RANDOM_83_VERTEX, RANDOM_83_FRAGMENT), Outcome::LinkFail(OUT_OF_SCRATCH.to_string()));
}

/// Control: a smaller case of the same group links.
#[test]
fn random_25_links() {
    assert_eq!(build(RANDOM_25_VERTEX, RANDOM_25_FRAGMENT), Outcome::Linked);
}

/// The Pi 4's dEQP images lend 8 MiB (f2fb64c9), twice the default.
#[test]
fn the_pi4_images_lend_8_mib() {
    assert_eq!(pi4_image_scratch(), 8 << 20);
    assert_eq!(yakogl::shaderc::online::store::SCRATCH_BYTES, 4 << 20);
}

/// S16 variant_3 links with the scratch of the Pi 4's images.
#[test]
fn variant_3_links_in_the_pi4_images_8_mib() {
    assert_eq!(build_in(pi4_image_scratch(), VERTEX, VARIANT_3), Outcome::Linked);
}

/// UA1 `uniform_api.random.83` links with the scratch of the Pi 4's images.
#[test]
fn random_83_links_in_the_pi4_images_8_mib() {
    assert_eq!(build_in(pi4_image_scratch(), RANDOM_83_VERTEX, RANDOM_83_FRAGMENT), Outcome::Linked);
}
