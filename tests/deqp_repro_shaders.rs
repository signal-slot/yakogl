// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Shaders dEQP generated that the in-process compiler must survive, kept as
//! the text dEQP handed glShaderSource. The generated cases (random.*,
//! operator, struct, loops) are not in dEQP's .test corpus, and a .qpa holds
//! no source for a case that aborted the process, so each one here was read
//! out of glShaderSource under gdb from the dEQP-GLES2 binary the host run
//! uses. dEQP (VK-GL-CTS) is Apache-2.0.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// dEQP-GLES2.functional.shaders.random.all_features.fragment.64, vertex.
const RANDOM_AF_FRAG64_VS: &str = r#"attribute vec4 dEQP_Position;
attribute vec3 a_b;
attribute float a_r;
attribute float a_f;
attribute float a_e;
attribute vec4 a_d;
attribute vec4 a_c;
varying mediump vec3 b;
varying mediump float r;
varying mediump float f;
varying mediump float e;
varying mediump vec4 d;
varying mediump vec4 c;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	r = a_r;
	f = a_f;
	e = a_e;
	d = a_d;
	c = a_c;
}
"#;

/// The same case's fragment shader: 660 values, past the allocator's 512,
/// and the copy coalescer indexed its table with one before checking it
/// (alloc.rs, `coalesce_copies`), which panicked inside glCompileShader and
/// aborted dEQP.
const RANDOM_AF_FRAG64_FS: &str = r#"precision mediump float;
uniform samplerCube a;
varying mediump vec3 b;
const float z = (7.0) - log2(16.0);
const int y = ivec2(sqrt(vec2(z, 4).r), float(float(-1.5) - float(-2.5))).g;
const int w = (ivec3(z, 14, -21) + ivec3(z, z, -5)).zxy.s;
const int v = (ivec4(z, 1, true, w).tsp * (ivec3(1.0, y, y))).r;
const float u = vec3(int(sqrt(float(z))), vec2(-0.25, z).r * log2(float(2)), (int(3) < 4)).g;
uniform mediump float t;
const float s = -17.8 * (exp(float(-5.0) + 5.0));
varying mediump float r;
const float q = (-1.0);
uniform mediump vec4 p;
float o = float((ivec2(2.5, 6.75) * ivec2(u, 1)).g);
const int n = ((ivec2(10, true))).rg.x;
const bool m = exp(float(ivec4(n, v, u, 0.0).a)) + 0.001 >= inversesqrt(float(8.625) - 2.0 + (-6.5));
const bool l = bool(int(sqrt(exp2(float(m)))));
const int k = (ivec4(true, u, -22, l) * ivec4(z, l, 1.0, q)).xzy.t;
const int h = ivec3(l, -6.0, l).ts.x - ((-4)) * (ivec2(l, n)).s;
const int g = ivec2(-7.0, tan(tan(float(u)))).x;
varying mediump float f;
varying mediump float e;
varying mediump vec4 d;
varying mediump vec4 c;
uniform mediump int aa;
const bool ab = vec4(z, 12, 4, -8.5).abg + vec3(3.5, -7.5, -6) + vec3(float(int(6)), vec4(4.0, m, s, u).s, 1.0 + float(g)) == sqrt(vec4(y, 0.25, 1, m).zwyx).spt;
const ivec4 ac = ivec4(int(true), true, z, int(q)) - ivec4(l, 5.0, 10.0, g).brga + (ivec4(h, true, -2.0, q)).qspt - (ivec4(v, 4.75, g, g).xwyz + ivec4(z, v, -1, u) * ivec4(ab, ab, true, y));
vec4 ad = vec4((ivec3(v, true, 5)).z, log(float(n)) <= vec4(m, m, g, v).w + 0.001, ivec4(u, g, true, true).pst.p, 0.5 * float(-1.0) * 1.0).xzyw;

void main()
{
	gl_FragColor = ad;
	int x = 2 + ac.p;
	if (((ab)))
		o = (vec2(aa, l).xy.r);
	else 
	{
		int j = w;
		vec4 i = vec4(float(int(z) * 1 * int(y)), -7 - int((float(x))), float(w * v) * u, t);
		i = vec4(s + r, (r), 3.7, 20.1 * q) + (p + i) + vec4(tan(1.25), (-1), vec2(o, j).x, exp(float(l))).spqt;
		i = vec4(n - int(6), (vec4(e, m, -7, l).rgab.q), int(k - j * 1), -0.5);
		gl_FragColor = (vec4(h, g, f, e).zywx - (d)) * ((c));
	}
	gl_FragColor = textureCube(a, b + gl_FragColor.bagr.zwy);
	{
	}
}
"#;

fn compile(stage: u32, src: &str) -> (i32, String) {
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    (rc, log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect())
}

/// dEQP-GLES2.functional.shaders.loops.do_while_constant_iterations
/// .only_continue_fragment: a `continue` in a `do`/`while`, which must run
/// the test before the next trip (the front end copies it in front of the
/// `continue`) and is a second back edge of its loop, which the ABI's
/// validator must be told about or it refuses the backward branch.
const LOOPS_DO_WHILE_ONLY_CONTINUE_FS: &str = "varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    mediump int i = 0;
    do { continue; } while (++i < 3);
    gl_FragColor = vec4(res.rgb, 1.0);
}
";

/// Compiled, or refused by name -- never a panic, which aborts the
/// application. One test in this binary, because it resets the keep arena
/// (`test_rules`).
/// dEQP-GLES2.functional.uniform_api.random.23, fragment: 10.9 KB of
/// compare_* calls over nested uniform structs. Read out of the .qpa the
/// host run (tools/cts/host-deqp-run.sh) writes with shader sources on,
/// 2026-09-23 at fe2dbcd. Its 5,212 IR instructions and ~5,800 values
/// were refused by the allocator's old 512-value bound (OutOfRegisters);
/// since 9c147fb the allocator takes it, and the 2 MiB compile scratch
/// refused it next. It compiles since the allocator encodes as it builds:
/// 2,888 instructions, 4 temps, a 1,977,592-byte high-water, which is the
/// front end's before `lower::relocate` (tools/cts/RESULTS.md).
const UNIFORM_API_RANDOM_23_FS: &str = r#"varying mediump float v_vtxOut;

struct structType2
{
	mediump mat4 m0;
	mediump vec3 m1;
	mediump samplerCube m2;
	mediump mat4 m3;
};
struct structType1
{
	mediump mat2 m0[3];
	mediump sampler2D m1;
	bvec4 m2[4];
	structType2 m3;
};
struct structType4
{
	mediump vec3 m0[1];
	mediump samplerCube m1;
};
struct structType5
{
	bool m0;
	mediump mat4 m1;
	mediump ivec4 m2;
};
struct structType3
{
	structType4 m0[3];
	structType5 m1[4];
};
struct structType6
{
	bool m0[5];
	mediump sampler2D m1;
};
struct structType0
{
	structType1 m0;
	mediump mat4 m1[4];
	mediump vec4 m2;
	structType3 m3[4];
	structType6 m4;
};
uniform structType0 u_var0;
uniform mediump float u_var1[5];
uniform bvec2 u_var2[2];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat2(u_var0.m0.m0[0], mat2(0.00, 0.00, 0.00, 0.00));
	result *= compare_mat2(u_var0.m0.m0[1], mat2(0.00, 0.00, 0.00, 0.00));
	result *= compare_mat2(u_var0.m0.m0[2], mat2(0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(texture2D(u_var0.m0.m1, vec2(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec4(u_var0.m0.m2[0], bvec4(false, false, false, false));
	result *= compare_bvec4(u_var0.m0.m2[1], bvec4(false, false, false, false));
	result *= compare_bvec4(u_var0.m0.m2[2], bvec4(false, false, false, false));
	result *= compare_bvec4(u_var0.m0.m2[3], bvec4(false, false, false, false));
	result *= compare_mat4(u_var0.m0.m3.m0, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec3(u_var0.m0.m3.m1, vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m0.m3.m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_mat4(u_var0.m0.m3.m3, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat4(u_var0.m1[0], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat4(u_var0.m1[1], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat4(u_var0.m1[2], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat4(u_var0.m1[3], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(u_var0.m2, vec4(0.00, 0.00, 0.00, 0.00));
	result *= compare_vec3(u_var0.m3[0].m0[0].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[0].m0[0].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[0].m0[1].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[0].m0[1].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[0].m0[2].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[0].m0[2].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bool(u_var0.m3[0].m1[0].m0, false);
	result *= compare_mat4(u_var0.m3[0].m1[0].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[0].m1[0].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[0].m1[1].m0, false);
	result *= compare_mat4(u_var0.m3[0].m1[1].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[0].m1[1].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[0].m1[2].m0, false);
	result *= compare_mat4(u_var0.m3[0].m1[2].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[0].m1[2].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[0].m1[3].m0, false);
	result *= compare_mat4(u_var0.m3[0].m1[3].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[0].m1[3].m2, ivec4(0, 0, 0, 0));
	result *= compare_vec3(u_var0.m3[1].m0[0].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[1].m0[0].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[1].m0[1].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[1].m0[1].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[1].m0[2].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[1].m0[2].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bool(u_var0.m3[1].m1[0].m0, false);
	result *= compare_mat4(u_var0.m3[1].m1[0].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[1].m1[0].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[1].m1[1].m0, false);
	result *= compare_mat4(u_var0.m3[1].m1[1].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[1].m1[1].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[1].m1[2].m0, false);
	result *= compare_mat4(u_var0.m3[1].m1[2].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[1].m1[2].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[1].m1[3].m0, false);
	result *= compare_mat4(u_var0.m3[1].m1[3].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[1].m1[3].m2, ivec4(0, 0, 0, 0));
	result *= compare_vec3(u_var0.m3[2].m0[0].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[2].m0[0].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[2].m0[1].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[2].m0[1].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[2].m0[2].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[2].m0[2].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bool(u_var0.m3[2].m1[0].m0, false);
	result *= compare_mat4(u_var0.m3[2].m1[0].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[2].m1[0].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[2].m1[1].m0, false);
	result *= compare_mat4(u_var0.m3[2].m1[1].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[2].m1[1].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[2].m1[2].m0, false);
	result *= compare_mat4(u_var0.m3[2].m1[2].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[2].m1[2].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[2].m1[3].m0, false);
	result *= compare_mat4(u_var0.m3[2].m1[3].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[2].m1[3].m2, ivec4(0, 0, 0, 0));
	result *= compare_vec3(u_var0.m3[3].m0[0].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[3].m0[0].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[3].m0[1].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[3].m0[1].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[3].m0[2].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[3].m0[2].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bool(u_var0.m3[3].m1[0].m0, false);
	result *= compare_mat4(u_var0.m3[3].m1[0].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[3].m1[0].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[3].m1[1].m0, false);
	result *= compare_mat4(u_var0.m3[3].m1[1].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[3].m1[1].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[3].m1[2].m0, false);
	result *= compare_mat4(u_var0.m3[3].m1[2].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[3].m1[2].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[3].m1[3].m0, false);
	result *= compare_mat4(u_var0.m3[3].m1[3].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[3].m1[3].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m4.m0[0], false);
	result *= compare_bool(u_var0.m4.m0[1], false);
	result *= compare_bool(u_var0.m4.m0[2], false);
	result *= compare_bool(u_var0.m4.m0[3], false);
	result *= compare_bool(u_var0.m4.m0[4], false);
	result *= compare_vec4(texture2D(u_var0.m4.m1, vec2(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_float(u_var1[0], 0.00);
	result *= compare_float(u_var1[1], 0.00);
	result *= compare_float(u_var1[2], 0.00);
	result *= compare_float(u_var1[3], 0.00);
	result *= compare_float(u_var1[4], 0.00);
	result *= compare_bvec2(u_var2[0], bvec2(false, false));
	result *= compare_bvec2(u_var2[1], bvec2(false, false));
    gl_FragColor = vec4(result, result, result, 1.0);
}
"#;

#[test]
fn deqp_generated_shaders_compile_or_are_refused() {
    let (rc, msg) = compile(SHC_STAGE_VERTEX, RANDOM_AF_FRAG64_VS);
    assert_eq!(rc, 0, "the vertex shader is small and must compile: {msg}");

    /* A refusal today, and the reason is stated rather than assumed: the
     * program needs more values than the allocator tracks. When selection or
     * folding brings it under the bound this becomes a compile, and the test
     * accepts that too. */
    let (rc, msg) = compile(SHC_STAGE_FRAGMENT, RANDOM_AF_FRAG64_FS);
    assert!(rc == 0 || msg.contains("OutOfRegisters"), "refused, but not for the known reason: rc {rc}, {msg}");

    /* `continue`, through the whole product chain: selection, allocation,
     * encoding and the ABI, whose validator refuses a backward branch that
     * is not a listed back edge. */
    /* uniform_api.random.23 compiles in the product's 2 MiB scratch since the
     * allocator encodes as it builds (no 160-byte `Instr` per instruction):
     * its peak went from 2,251,896 bytes -- 154,744 over, refused as "out of
     * compiler memory" -- to 1,977,592, which is now the FRONT END's
     * high-water before `lower::relocate` gives its garbage back; the back
     * end ends at 1,776,712. The bound is the arena less headroom, so the day
     * the front end or the back end grows back toward the edge fails here,
     * by name and with the number, before a dEQP run refuses it. */
    let (rc, msg) = compile(SHC_STAGE_FRAGMENT, UNIFORM_API_RANDOM_23_FS);
    assert_eq!(rc, 0, "uniform_api.random.23 must compile in the product scratch: {msg}");
    let peak = yakogl::shaderc::online::store::last_scratch_peak();
    println!("uniform_api.random.23: scratch high-water {peak} of {}", yakogl::shaderc::online::store::SCRATCH_BYTES);
    assert!(
        (1_000_000..=2_000_000).contains(&peak),
        "uniform_api.random.23's scratch high-water is {peak}: above 2,000,000 it is back within 95 KiB of the 2 MiB \
         arena it was refused by, and below 1,000,000 the instrument is not measuring this compile"
    );

    /* Four draw buffers (ES 3.00 layout(location = n) out): every colour
     * output its own register, listed as output rows by location, and the
     * first row's register is ps_color_out_reg -- what the driver programs
     * PS_OUTPUT_REG from, one byte per render target. GLSL ES 3.00, which a
     * report-es2 build refuses (pp.rs out_of_scope). */
    #[cfg(not(feature = "report-es2"))]
    four_draw_buffers();

    /* shaders.builtin_variable.pointcoord's fragment shader shape: it was
     * the last "not selected yet" refusal on this GPU's host dEQP list.
     * gl_PointCoord is the fragment input row after the varyings, two
     * components, named for the link and the driver. */
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let text = "precision mediump float;\nvarying vec4 v_color;\nvoid main() { gl_FragColor = vec4(gl_PointCoord, 0.0, 1.0) * v_color; }\n\0";
    yakogl::shaderc::online::backend::forget_compiled();
    let online = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc =
        unsafe { online(SHC_STAGE_FRAGMENT, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "gl_PointCoord compiles: {msg}");
    let rows: Vec<(u8, u8, String)> = out.viv.in_[..out.viv.num_in as usize]
        .iter()
        .map(|r| (r.reg, r.components, r.name().iter().map(|c| *c as u8 as char).collect()))
        .collect();
    assert_eq!(
        rows,
        [(1, 4, "v_color".to_string()), (2, 2, yakogl::shaderc::SHC_VIV_POINT_COORD.to_string())],
        "the varying, then gl_PointCoord"
    );

    for (name, src) in [
        ("loops.do_while_constant_iterations.only_continue", LOOPS_DO_WHILE_ONLY_CONTINUE_FS),
        ("es100/contloops.frag", include_str!("../tools/shaders/es100/contloops.frag")),
    ] {
        let (rc, msg) = compile(SHC_STAGE_FRAGMENT, src);
        assert_eq!(rc, 0, "{name} must compile: {msg}");
    }
}

#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
fn four_draw_buffers() {
    let mrt = "#version 300 es\nprecision mediump float;\n\
               layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
               layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
               uniform vec4 u;\n\
               void main() { o0 = u; o1 = u.yzwx; o2 = u.zwxy; o3 = u.wxyz; }\n\0";
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    yakogl::shaderc::online::backend::forget_compiled();
    let online = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { online(SHC_STAGE_FRAGMENT, mrt.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) };
    let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "four colour outputs compile: {msg}");
    let outs: Vec<(u8, u8)> = sh.viv.out[..sh.viv.num_out as usize].iter().map(|r| (r.slot, r.reg)).collect();
    assert_eq!(
        outs.iter().map(|o| o.0).collect::<Vec<_>>(),
        [4, 5, 6, 7],
        "one row per location, slot FRAG_RESULT_DATA0 + n: {outs:?}"
    );
    let mut regs: Vec<u8> = outs.iter().map(|o| o.1).collect();
    assert_eq!(regs[0], sh.viv.ps_color_out_reg, "row 0 is the first colour: {outs:?}");
    regs.sort_unstable();
    regs.dedup();
    assert_eq!(regs.len(), 4, "four distinct registers: {outs:?}");
}
