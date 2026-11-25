varying mediump float v_vtxOut;

struct structType0
{
	mediump mat3 m0;
	bvec2 m1[2];
	mediump float m2;
	mediump mat2 m3;
};
uniform structType0 u_var0;
uniform mediump samplerCube u_var1;
uniform mediump float u_var2;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat3(u_var0.m0, mat3(-3.20, -0.38, 5.48, 9.00, -6.82, -9.63, -6.47, 4.76, 1.90));
	// UNUSED: u_var0.m1[0]
	result *= compare_bvec2(u_var0.m1[1], bvec2(true, false));
	// UNUSED: u_var0.m2
	result *= compare_mat2(u_var0.m3, mat2(-7.62, 9.88, -6.09, 1.88));
	// UNUSED: u_var1
	result *= compare_float(u_var2, 1.29);
    gl_FragColor = vec4(result, result, result, 1.0);
}
