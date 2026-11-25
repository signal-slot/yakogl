varying mediump float v_vtxOut;

uniform mediump mat3 u_var0;
uniform mediump mat2 u_var1;
uniform mediump int u_var2[4];
uniform mediump vec3 u_var3;
uniform mediump ivec2 u_var4[2];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec2    (mediump ivec2 a, mediump ivec2 b)  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat3(u_var0, mat3(-3.19, -6.82, 1.68, 2.60, 8.07, -6.26, 7.58, 4.20, 8.05));
	result *= compare_mat2(u_var1, mat2(-9.57, -9.64, 5.12, -7.04));
	// UNUSED: u_var2[0]
	// UNUSED: u_var2[1]
	result *= compare_int(u_var2[2], 3);
	// UNUSED: u_var2[3]
	result *= compare_vec3(u_var3, vec3(8.23, 9.86, -8.35));
	// UNUSED: u_var4[0]
	result *= compare_ivec2(u_var4[1], ivec2(3, -8));
    gl_FragColor = vec4(result, result, result, 1.0);
}
