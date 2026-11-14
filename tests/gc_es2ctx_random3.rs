// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP), ES 2.0 context: dEQP-GLES2 functional.uniform_api.random.3
//! links, and its vertex constant file fits the GC7000's 320 registers.
//!
//! random.3 is a vertex case (`RandomUniformCase`, es2fUniformApiTests.cpp,
//! seed 3 and the default base seed 0): 144 basic uniforms, 13 of them vertex
//! samplers, each compared with its value written as a literal
//! (`compare_vec4(u, vec4(-0.17, -2.54, -3.25, 6.23))`). [`VS`] and [`FS`] are
//! the two sources dEQP generates for it: the generator of
//! es2fUniformApiTests.cpp and `de::Random` replayed off the board, which
//! gives the same struct types, uniforms, comparisons and literal values
//! (the blanks inside the int and bool helper lines aside). The vertex
//! constant file it compiles to before 6e498505 is the 275 registers the
//! board's case had, which is what ties the two together.
//!
//! The vertex constant file is 174 registers of uniforms and the literals.
//! Before 6e498505 the literals were vec4s, 101 registers, 275 in all
//! (src/vivante/state.rs). 6e498505 (rank 4a) folds a read of a parameter
//! slot stored once with a constant, so each `compare_float(a.x, b.x)` of the
//! inlined comparisons reads a scalar literal (`-b.x` through the subtract),
//! and `try_place` (src/shaderc/online/select.rs) counted the UNUSED padding
//! of a written register as taken: every scalar literal took a register of
//! its own, 461 in all, and the link refused the program ("the constant
//! registers the two stages need (vertex 461, fragment 0 ...) exceed the 320
//! this GPU has"). With the padding lanes taken as room the scalars pack four
//! to a register again: 217 (174 + 43).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_random3
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use yakogl::vivante::regs::GC_NUM_CONSTANTS;
use yakogl::vivante::shader::constant_registers;

/// random.3's vertex shader, as dEQP generates it.
const VS: &str = r#"attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType0
{
	bvec3 m0;
	mediump samplerCube m1[1];
};
struct structType3
{
	mediump int m0;
};
struct structType2
{
	bvec3 m0;
	structType3 m1;
};
struct structType5
{
	bvec4 m0;
	mediump ivec2 m1;
	mediump vec4 m2[4];
};
struct structType4
{
	mediump vec4 m0;
	structType5 m1[3];
	mediump sampler2D m2[1];
};
struct structType1
{
	mediump sampler2D m0;
	structType2 m1[2];
	structType4 m2[5];
};
struct structType8
{
	bool m0;
	mediump mat3 m1[2];
};
struct structType7
{
	mediump ivec2 m0[5];
	mediump samplerCube m1[3];
	structType8 m2;
	bvec2 m3[3];
};
struct structType6
{
	structType7 m0[2];
	bvec3 m1;
	mediump mat2 m2[4];
};
uniform bvec2 u_var0;
uniform structType0 u_var1;
uniform bvec2 u_var2[3];
uniform structType1 u_var3;
uniform structType6 u_var4;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_int (mediump int a, mediump int b) { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec2 (mediump ivec2 a, mediump ivec2 b) { return a == b ? 1.0 : 0.0; }
mediump float compare_bool (bool a, bool b) { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2 (bvec2 a, bvec2 b) { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec3 (bvec3 a, bvec3 b) { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4 (bvec4 a, bvec4 b) { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_bvec2(u_var0, bvec2(true, true));
	v_vtxOut *= compare_bvec3(u_var1.m0, bvec3(true, true, false));
	v_vtxOut *= compare_vec4(textureCube(u_var1.m1[0], vec3(0.0)), vec4(0.97, 0.27, 0.21, 0.27));
	v_vtxOut *= compare_bvec2(u_var2[0], bvec2(false, true));
	v_vtxOut *= compare_bvec2(u_var2[1], bvec2(true, true));
	v_vtxOut *= compare_bvec2(u_var2[2], bvec2(false, true));
	v_vtxOut *= compare_vec4(texture2D(u_var3.m0, vec2(0.0)), vec4(0.19, 0.80, 0.41, 0.72));
	v_vtxOut *= compare_bvec3(u_var3.m1[0].m0, bvec3(true, true, false));
	v_vtxOut *= compare_int(u_var3.m1[0].m1.m0, -2);
	v_vtxOut *= compare_bvec3(u_var3.m1[1].m0, bvec3(false, false, true));
	v_vtxOut *= compare_int(u_var3.m1[1].m1.m0, 3);
	v_vtxOut *= compare_vec4(u_var3.m2[0].m0, vec4(-0.17, -2.54, -3.25, 6.23));
	v_vtxOut *= compare_bvec4(u_var3.m2[0].m1[0].m0, bvec4(false, true, true, true));
	v_vtxOut *= compare_ivec2(u_var3.m2[0].m1[0].m1, ivec2(1, 9));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[0].m2[0], vec4(9.82, 1.59, -6.52, 3.19));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[0].m2[1], vec4(8.21, -2.43, -6.40, -0.62));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[0].m2[2], vec4(-2.63, 6.16, -0.17, 4.35));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[0].m2[3], vec4(6.54, -7.20, -0.65, 9.82));
	v_vtxOut *= compare_bvec4(u_var3.m2[0].m1[1].m0, bvec4(true, false, true, false));
	v_vtxOut *= compare_ivec2(u_var3.m2[0].m1[1].m1, ivec2(10, 9));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[1].m2[0], vec4(-5.54, 9.61, 1.78, 4.18));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[1].m2[1], vec4(-0.28, -9.79, 7.31, 1.43));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[1].m2[2], vec4(-5.22, 8.57, -7.42, 0.43));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[1].m2[3], vec4(2.47, -7.02, -5.74, -1.87));
	v_vtxOut *= compare_bvec4(u_var3.m2[0].m1[2].m0, bvec4(true, true, false, true));
	v_vtxOut *= compare_ivec2(u_var3.m2[0].m1[2].m1, ivec2(7, 5));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[2].m2[0], vec4(7.62, -6.41, 1.38, 5.25));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[2].m2[1], vec4(4.69, -9.64, 6.50, -7.20));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[2].m2[2], vec4(1.14, 2.83, 0.30, 7.90));
	v_vtxOut *= compare_vec4(u_var3.m2[0].m1[2].m2[3], vec4(2.33, -8.43, -7.80, 7.75));
	v_vtxOut *= compare_vec4(texture2D(u_var3.m2[0].m2[0], vec2(0.0)), vec4(0.03, 0.10, 0.23, 0.47));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m0, vec4(5.98, 8.79, -4.53, 7.01));
	v_vtxOut *= compare_bvec4(u_var3.m2[1].m1[0].m0, bvec4(false, false, false, true));
	v_vtxOut *= compare_ivec2(u_var3.m2[1].m1[0].m1, ivec2(1, -8));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[0].m2[0], vec4(-7.71, 1.42, -4.36, -3.74));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[0].m2[1], vec4(9.84, -7.14, -6.23, 6.47));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[0].m2[2], vec4(4.25, -6.97, -6.82, -1.80));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[0].m2[3], vec4(-4.25, 7.51, 9.67, -9.23));
	v_vtxOut *= compare_bvec4(u_var3.m2[1].m1[1].m0, bvec4(true, true, true, true));
	v_vtxOut *= compare_ivec2(u_var3.m2[1].m1[1].m1, ivec2(6, 3));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[1].m2[0], vec4(4.46, -7.11, 2.04, 8.97));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[1].m2[1], vec4(5.60, 0.48, 7.00, -8.67));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[1].m2[2], vec4(-6.03, -8.98, -9.56, 9.16));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[1].m2[3], vec4(-7.00, 3.87, 1.86, -6.66));
	v_vtxOut *= compare_bvec4(u_var3.m2[1].m1[2].m0, bvec4(false, false, false, false));
	v_vtxOut *= compare_ivec2(u_var3.m2[1].m1[2].m1, ivec2(0, -10));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[2].m2[0], vec4(5.96, 0.36, 2.30, -3.09));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[2].m2[1], vec4(9.08, 0.49, 2.64, 9.57));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[2].m2[2], vec4(-8.85, -0.33, -1.09, -2.17));
	v_vtxOut *= compare_vec4(u_var3.m2[1].m1[2].m2[3], vec4(-7.32, 0.84, 1.08, -5.54));
	v_vtxOut *= compare_vec4(texture2D(u_var3.m2[1].m2[0], vec2(0.0)), vec4(0.53, 0.08, 0.96, 0.41));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m0, vec4(4.38, -1.69, -7.43, -6.72));
	v_vtxOut *= compare_bvec4(u_var3.m2[2].m1[0].m0, bvec4(true, false, true, false));
	v_vtxOut *= compare_ivec2(u_var3.m2[2].m1[0].m1, ivec2(-4, 1));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[0].m2[0], vec4(-4.27, 4.86, 9.08, -2.78));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[0].m2[1], vec4(6.14, 6.46, 2.90, 0.85));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[0].m2[2], vec4(0.07, -9.17, -5.96, -1.71));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[0].m2[3], vec4(-6.59, 0.97, 6.73, -4.67));
	v_vtxOut *= compare_bvec4(u_var3.m2[2].m1[1].m0, bvec4(true, true, true, false));
	v_vtxOut *= compare_ivec2(u_var3.m2[2].m1[1].m1, ivec2(-8, 2));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[1].m2[0], vec4(-8.73, -4.35, 3.96, 5.73));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[1].m2[1], vec4(-4.60, 8.70, 9.73, 7.54));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[1].m2[2], vec4(0.80, 3.43, -5.55, 0.52));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[1].m2[3], vec4(6.75, 2.15, -8.15, -6.60));
	v_vtxOut *= compare_bvec4(u_var3.m2[2].m1[2].m0, bvec4(false, false, true, false));
	v_vtxOut *= compare_ivec2(u_var3.m2[2].m1[2].m1, ivec2(1, -7));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[2].m2[0], vec4(-9.62, 2.42, -8.18, -0.10));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[2].m2[1], vec4(-8.42, 9.78, 1.67, -8.64));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[2].m2[2], vec4(0.98, 1.85, 6.36, -7.37));
	v_vtxOut *= compare_vec4(u_var3.m2[2].m1[2].m2[3], vec4(-9.16, -8.61, 8.78, 4.75));
	v_vtxOut *= compare_vec4(texture2D(u_var3.m2[2].m2[0], vec2(0.0)), vec4(0.09, 0.39, 0.18, 0.50));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m0, vec4(-5.48, -3.07, -6.52, 1.50));
	v_vtxOut *= compare_bvec4(u_var3.m2[3].m1[0].m0, bvec4(false, true, true, false));
	v_vtxOut *= compare_ivec2(u_var3.m2[3].m1[0].m1, ivec2(5, -10));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[0].m2[0], vec4(0.01, -6.71, 1.33, 8.43));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[0].m2[1], vec4(7.74, 9.59, -4.14, -7.62));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[0].m2[2], vec4(-8.14, 9.38, -6.10, -8.65));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[0].m2[3], vec4(-2.35, -8.76, -2.57, 3.09));
	v_vtxOut *= compare_bvec4(u_var3.m2[3].m1[1].m0, bvec4(true, false, false, true));
	v_vtxOut *= compare_ivec2(u_var3.m2[3].m1[1].m1, ivec2(-10, -1));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[1].m2[0], vec4(5.66, -1.86, 4.19, -6.28));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[1].m2[1], vec4(-2.51, -3.10, -9.58, -7.75));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[1].m2[2], vec4(-2.13, 5.21, 8.88, -5.03));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[1].m2[3], vec4(4.13, 8.32, 1.61, 7.93));
	v_vtxOut *= compare_bvec4(u_var3.m2[3].m1[2].m0, bvec4(true, false, true, false));
	v_vtxOut *= compare_ivec2(u_var3.m2[3].m1[2].m1, ivec2(-9, -10));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[2].m2[0], vec4(3.10, -1.59, 7.54, -9.54));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[2].m2[1], vec4(-3.21, 4.29, -7.99, -0.48));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[2].m2[2], vec4(6.48, 2.35, -3.67, 1.85));
	v_vtxOut *= compare_vec4(u_var3.m2[3].m1[2].m2[3], vec4(2.91, 6.61, -0.59, 4.63));
	v_vtxOut *= compare_vec4(texture2D(u_var3.m2[3].m2[0], vec2(0.0)), vec4(0.41, 0.62, 0.74, 0.82));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m0, vec4(-9.41, -8.15, 7.13, 4.70));
	v_vtxOut *= compare_bvec4(u_var3.m2[4].m1[0].m0, bvec4(false, true, false, true));
	v_vtxOut *= compare_ivec2(u_var3.m2[4].m1[0].m1, ivec2(-10, 6));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[0].m2[0], vec4(-4.17, 8.82, 5.38, 4.15));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[0].m2[1], vec4(4.66, -2.48, 9.80, -0.37));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[0].m2[2], vec4(-9.21, -1.10, -6.00, -6.71));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[0].m2[3], vec4(9.79, 6.87, 7.43, 8.46));
	v_vtxOut *= compare_bvec4(u_var3.m2[4].m1[1].m0, bvec4(true, false, false, false));
	v_vtxOut *= compare_ivec2(u_var3.m2[4].m1[1].m1, ivec2(10, 1));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[1].m2[0], vec4(-6.57, 8.06, -5.30, 8.39));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[1].m2[1], vec4(3.41, 1.74, -9.74, -5.52));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[1].m2[2], vec4(2.19, 2.91, 4.14, -7.53));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[1].m2[3], vec4(4.99, 0.92, -9.69, 5.17));
	v_vtxOut *= compare_bvec4(u_var3.m2[4].m1[2].m0, bvec4(true, false, true, false));
	v_vtxOut *= compare_ivec2(u_var3.m2[4].m1[2].m1, ivec2(-10, -5));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[2].m2[0], vec4(-4.25, 4.43, -0.85, -8.75));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[2].m2[1], vec4(-9.29, 4.32, 0.47, 1.90));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[2].m2[2], vec4(4.35, 5.79, 9.14, 2.58));
	v_vtxOut *= compare_vec4(u_var3.m2[4].m1[2].m2[3], vec4(-2.48, 1.20, -8.76, -9.20));
	v_vtxOut *= compare_vec4(texture2D(u_var3.m2[4].m2[0], vec2(0.0)), vec4(0.58, 0.68, 0.48, 0.17));
	v_vtxOut *= compare_ivec2(u_var4.m0[0].m0[0], ivec2(3, -4));
	v_vtxOut *= compare_ivec2(u_var4.m0[0].m0[1], ivec2(-2, -7));
	v_vtxOut *= compare_ivec2(u_var4.m0[0].m0[2], ivec2(-8, 6));
	v_vtxOut *= compare_ivec2(u_var4.m0[0].m0[3], ivec2(5, -5));
	v_vtxOut *= compare_ivec2(u_var4.m0[0].m0[4], ivec2(-2, -9));
	v_vtxOut *= compare_vec4(textureCube(u_var4.m0[0].m1[0], vec3(0.0)), vec4(0.06, 0.67, 0.14, 0.48));
	v_vtxOut *= compare_vec4(textureCube(u_var4.m0[0].m1[1], vec3(0.0)), vec4(0.76, 0.51, 0.81, 0.65));
	v_vtxOut *= compare_vec4(textureCube(u_var4.m0[0].m1[2], vec3(0.0)), vec4(0.76, 0.06, 0.18, 0.91));
	v_vtxOut *= compare_bool(u_var4.m0[0].m2.m0, true);
	v_vtxOut *= compare_mat3(u_var4.m0[0].m2.m1[0], mat3(-8.15, 3.94, -2.38, 2.42, -0.67, 5.38, -9.39, 6.27, -9.88));
	v_vtxOut *= compare_mat3(u_var4.m0[0].m2.m1[1], mat3(-1.13, -8.71, 2.73, 0.93, 4.59, 3.46, 1.34, -4.82, 6.63));
	v_vtxOut *= compare_bvec2(u_var4.m0[0].m3[0], bvec2(false, false));
	v_vtxOut *= compare_bvec2(u_var4.m0[0].m3[1], bvec2(false, false));
	v_vtxOut *= compare_bvec2(u_var4.m0[0].m3[2], bvec2(false, false));
	v_vtxOut *= compare_ivec2(u_var4.m0[1].m0[0], ivec2(2, -5));
	v_vtxOut *= compare_ivec2(u_var4.m0[1].m0[1], ivec2(3, 7));
	v_vtxOut *= compare_ivec2(u_var4.m0[1].m0[2], ivec2(-2, 7));
	v_vtxOut *= compare_ivec2(u_var4.m0[1].m0[3], ivec2(-1, 1));
	v_vtxOut *= compare_ivec2(u_var4.m0[1].m0[4], ivec2(10, 7));
	v_vtxOut *= compare_vec4(textureCube(u_var4.m0[1].m1[0], vec3(0.0)), vec4(0.53, 0.67, 0.47, 0.03));
	v_vtxOut *= compare_vec4(textureCube(u_var4.m0[1].m1[1], vec3(0.0)), vec4(0.04, 0.34, 0.32, 0.04));
	v_vtxOut *= compare_vec4(textureCube(u_var4.m0[1].m1[2], vec3(0.0)), vec4(0.10, 0.49, 0.96, 0.37));
	v_vtxOut *= compare_bool(u_var4.m0[1].m2.m0, false);
	v_vtxOut *= compare_mat3(u_var4.m0[1].m2.m1[0], mat3(-0.34, -7.47, 1.10, -3.98, 7.22, -9.63, -6.94, 1.18, -4.11));
	v_vtxOut *= compare_mat3(u_var4.m0[1].m2.m1[1], mat3(-2.88, -2.40, 9.85, -9.56, 7.91, -6.32, -4.17, 9.70, -3.07));
	v_vtxOut *= compare_bvec2(u_var4.m0[1].m3[0], bvec2(true, true));
	v_vtxOut *= compare_bvec2(u_var4.m0[1].m3[1], bvec2(true, true));
	v_vtxOut *= compare_bvec2(u_var4.m0[1].m3[2], bvec2(false, true));
	v_vtxOut *= compare_bvec3(u_var4.m1, bvec3(true, true, false));
	v_vtxOut *= compare_mat2(u_var4.m2[0], mat2(-6.83, -0.25, 2.21, -8.36));
	v_vtxOut *= compare_mat2(u_var4.m2[1], mat2(-4.93, 5.54, -8.07, 9.49));
	v_vtxOut *= compare_mat2(u_var4.m2[2], mat2(-5.41, 2.96, -0.50, -6.48));
	v_vtxOut *= compare_mat2(u_var4.m2[3], mat2(-2.83, -2.17, -6.24, -5.88));
}
"#;

/// random.3's fragment shader: a vertex case's, which only passes the result on.
const FS: &str = r#"varying mediump float v_vtxOut;


void main (void)
{
    mediump float result = v_vtxOut;
    gl_FragColor = vec4(result, result, result, 1.0);
}
"#;

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
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

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"gc-es2ctx-random3".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
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
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one ES 2.0 context, the online compiler behind it; serialises the tests.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "the shader compiles");
        s
    }
}

/// The program's info log.
fn program_log(p: GLuint) -> String {
    let mut buf = [0 as c_char; 1024];
    let mut len: GLsizei = 0;
    // SAFETY: a buffer of the size announced; GL NUL-terminates the log.
    unsafe {
        glGetProgramInfoLog(p, buf.len() as GLsizei, &mut len, buf.as_mut_ptr());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// One stage through the compiler the context uses (`glCompileShader`'s).
fn compile(stage: u32, src: &str) -> ShcShader {
    let (mut sh, mut log) = (ShcShader::ZERO, [0 as c_char; 1024]);
    let text = CString::new(src).unwrap();
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, text.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    // SAFETY: the compiler NUL-terminates its log.
    let msg = unsafe { CStr::from_ptr(log.as_ptr()) }.to_string_lossy();
    assert_eq!(rc, 0, "the compile fails: {msg}");
    sh
}

/// What dEQP does (UniformCase::iterate): compile, link; the case fails with
/// "Compile failed" when the program is not ok.
#[test]
fn random3_links_in_an_es2_context() {
    let _g = context();
    // SAFETY: the context's static version string.
    let version = unsafe { CStr::from_ptr(glGetString(GL_VERSION) as *const c_char) }.to_string_lossy().into_owned();
    assert!(version.starts_with("OpenGL ES 2.0"), "an ES 2.0 context: {version}");
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    // SAFETY: plain GL calls on names made above.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "random.3 links: {}", program_log(p));
        let mut active: GLint = 0;
        glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &mut active);
        assert!(active > 0, "its uniforms are active");
        assert_eq!(glGetError(), GL_NO_ERROR);
        glDeleteShader(v);
        glDeleteShader(f);
        glDeleteProgram(p);
    }
}

/// The count behind the link: the vertex constant file, uniforms and
/// literals, is at most the 320 registers the two stages share (217 since
/// this fix; 275 before 6e498505, 461 after it).
#[test]
fn random3_vertex_constant_file_fits_the_gc7000_file() {
    let _g = context();
    let vs = compile(SHC_STAGE_VERTEX, VS);
    let mut fs = compile(SHC_STAGE_FRAGMENT, FS);
    let (v, f) = (constant_registers(&vs), constant_registers(&fs));
    assert!(v + f <= GC_NUM_CONSTANTS, "vertex {v} + fragment {f} constant registers, past {GC_NUM_CONSTANTS}");
    assert!(v <= 217, "vertex {v} constant registers: the literals no longer pack four to a register");
    let mut log = [0 as c_char; 1024];
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    // SAFETY: two compiled stages and a log of the size announced.
    let rc = unsafe { link(&vs, &mut fs, log.as_mut_ptr(), log.len()) };
    // SAFETY: the compiler NUL-terminates its log.
    let msg = unsafe { CStr::from_ptr(log.as_ptr()) }.to_string_lossy();
    assert_eq!(rc, 0, "{msg}");
}

/// The rule itself, small: three scalar literals no f20 immediate holds, each
/// read alone, share one constant register after the uniform's.
#[test]
fn scalar_literals_share_a_constant_register() {
    let _g = context();
    let vs = compile(
        SHC_STAGE_VERTEX,
        "attribute vec4 p;\nuniform float u;\nvoid main() { float a = u + 0.97; float b = a * 0.27; \
         float c = b - 0.21; gl_Position = p * c; }\n",
    );
    assert_eq!(constant_registers(&vs), 2, "u's register and one register of literals");
}
