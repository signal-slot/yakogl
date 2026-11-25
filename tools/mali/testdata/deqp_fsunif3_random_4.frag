varying mediump float v_vtxOut;

struct structType0
{
	bvec4 m0;
};
uniform structType0 u_var0;
uniform bvec4 u_var1[4];
uniform mediump mat4 u_var2;
uniform mediump mat3 u_var3[2];
uniform bvec2 u_var4;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bvec4(u_var0.m0, bvec4(true, true, true, true));
	// UNUSED: u_var1[0]
	// UNUSED: u_var1[1]
	result *= compare_bvec4(u_var1[2], bvec4(true, false, false, false));
	// UNUSED: u_var1[3]
	result *= compare_mat4(u_var2, mat4(-8.03, 1.82, 0.25, 7.08, 0.14, -3.04, 7.26, 2.76, 8.51, 6.51, 0.13, -5.96, 5.05, 9.58, -5.85, 6.09));
	// UNUSED: u_var3[0]
	result *= compare_mat3(u_var3[1], mat3(6.36, -5.68, 0.33, 7.12, 5.23, 7.68, 1.48, 4.59, -5.28));
	result *= compare_bvec2(u_var4, bvec2(false, false));
    gl_FragColor = vec4(result, result, result, 1.0);
}
