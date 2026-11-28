varying mediump float v_vtxOut;

struct structType2
{
	mediump int m0;
};
struct structType1
{
	mediump mat4 m0;
	structType2 m1;
	mediump ivec4 m2;
	mediump mat4 m3;
};
struct structType0
{
	mediump mat4 m0;
	structType1 m1[5];
};
uniform mediump ivec4 u_var0;
uniform bool u_var1;
uniform structType0 u_var2;
uniform mediump mat2 u_var3;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_ivec4(u_var0, ivec4(5, 9, 8, -4));
	result *= compare_bool(u_var1, true);
	result *= compare_mat4(u_var2.m0, mat4(9.60, -0.74, -5.40, 3.05, 4.53, -9.89, 9.04, 4.15, 8.30, 9.15, 1.97, -5.45, -4.54, 9.07, 3.43, -2.46));
	// UNUSED: u_var2.m1[0].m0
	// UNUSED: u_var2.m1[0].m1.m0
	// UNUSED: u_var2.m1[0].m2
	// UNUSED: u_var2.m1[0].m3
	// UNUSED: u_var2.m1[1].m0
	// UNUSED: u_var2.m1[1].m1.m0
	// UNUSED: u_var2.m1[1].m2
	// UNUSED: u_var2.m1[1].m3
	result *= compare_mat4(u_var2.m1[2].m0, mat4(0.83, 1.95, -5.88, -4.82, -8.66, 7.01, -5.29, 7.72, -9.04, 0.25, -3.64, 0.91, 4.18, 5.55, -0.73, -3.81));
	result *= compare_int(u_var2.m1[2].m1.m0, -1);
	result *= compare_ivec4(u_var2.m1[2].m2, ivec4(4, 0, -8, -7));
	result *= compare_mat4(u_var2.m1[2].m3, mat4(-5.23, -3.19, 1.83, 2.13, -0.51, -8.38, -9.17, -7.33, -2.18, -9.20, -8.40, 9.85, -5.91, 1.13, 6.19, -3.62));
	// UNUSED: u_var2.m1[3].m0
	// UNUSED: u_var2.m1[3].m1.m0
	// UNUSED: u_var2.m1[3].m2
	// UNUSED: u_var2.m1[3].m3
	// UNUSED: u_var2.m1[4].m0
	// UNUSED: u_var2.m1[4].m1.m0
	// UNUSED: u_var2.m1[4].m2
	// UNUSED: u_var2.m1[4].m3
	result *= compare_mat2(u_var3, mat2(-4.12, -7.43, -2.51, -9.97));
    gl_FragColor = vec4(result, result, result, 1.0);
}
