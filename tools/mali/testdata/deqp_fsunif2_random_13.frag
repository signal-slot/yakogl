varying mediump float v_vtxOut;

struct structType2
{
	mediump float m0[5];
	mediump vec2 m1;
	mediump mat4 m2[4];
	bvec2 m3;
	mediump mat2 m4;
};
struct structType1
{
	bvec2 m0;
	structType2 m1;
};
struct structType0
{
	bool m0[1];
	mediump mat3 m1;
	structType1 m2;
	mediump ivec3 m3;
};
struct structType4
{
	bvec3 m0[3];
	bvec3 m1[3];
	mediump ivec2 m2;
};
struct structType3
{
	mediump sampler2D m0;
	structType4 m1[2];
};
struct structType6
{
	mediump vec2 m0;
	bvec3 m1[1];
	mediump float m2;
};
struct structType5
{
	structType6 m0;
	bool m1[4];
	mediump ivec3 m2[5];
	mediump ivec2 m3[2];
};
uniform mediump mat3 u_var0;
uniform structType0 u_var1;
uniform structType3 u_var2;
uniform structType5 u_var3;
uniform mediump float u_var4[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_ivec2    (mediump ivec2 a, mediump ivec2 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec3    (bvec3 a, bvec3 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat3(u_var0, mat3(-3.14, 0.67, -5.81, -9.53, -4.10, -8.75, -4.65, -7.61, 8.10));
	// UNUSED: u_var1.m0[0]
	result *= compare_mat3(u_var1.m1, mat3(9.28, -5.20, 6.59, 8.21, -8.27, -4.96, -2.05, -6.05, 4.92));
	// UNUSED: u_var1.m2.m0
	result *= compare_float(u_var1.m2.m1.m0[0], -5.11);
	// UNUSED: u_var1.m2.m1.m0[1]
	result *= compare_float(u_var1.m2.m1.m0[2], 8.28);
	// UNUSED: u_var1.m2.m1.m0[3]
	result *= compare_float(u_var1.m2.m1.m0[4], 9.87);
	// UNUSED: u_var1.m2.m1.m1
	result *= compare_mat4(u_var1.m2.m1.m2[0], mat4(-7.54, -7.50, -9.18, -0.55, -8.76, 2.37, 0.85, 7.20, 5.17, 8.52, -4.48, 3.99, 5.96, 8.49, -2.57, 1.33));
	// UNUSED: u_var1.m2.m1.m2[1]
	result *= compare_mat4(u_var1.m2.m1.m2[2], mat4(5.33, -5.89, -9.82, 2.00, 2.40, 5.46, -5.13, -4.56, 0.59, -1.13, -4.30, 6.51, -6.46, 0.28, -6.12, 8.33));
	// UNUSED: u_var1.m2.m1.m2[3]
	result *= compare_bvec2(u_var1.m2.m1.m3, bvec2(true, false));
	// UNUSED: u_var1.m2.m1.m4
	result *= compare_ivec3(u_var1.m3, ivec3(8, -10, 4));
	// UNUSED: u_var2.m0
	result *= compare_bvec3(u_var2.m1[0].m0[0], bvec3(true, true, true));
	// UNUSED: u_var2.m1[0].m0[1]
	result *= compare_bvec3(u_var2.m1[0].m0[2], bvec3(true, true, false));
	// UNUSED: u_var2.m1[0].m1[0]
	result *= compare_bvec3(u_var2.m1[0].m1[1], bvec3(false, true, true));
	// UNUSED: u_var2.m1[0].m1[2]
	result *= compare_ivec2(u_var2.m1[0].m2, ivec2(-3, -2));
	// UNUSED: u_var2.m1[1].m0[0]
	// UNUSED: u_var2.m1[1].m0[1]
	// UNUSED: u_var2.m1[1].m0[2]
	// UNUSED: u_var2.m1[1].m1[0]
	// UNUSED: u_var2.m1[1].m1[1]
	// UNUSED: u_var2.m1[1].m1[2]
	// UNUSED: u_var2.m1[1].m2
	result *= compare_vec2(u_var3.m0.m0, vec2(4.55, 9.67));
	// UNUSED: u_var3.m0.m1[0]
	result *= compare_float(u_var3.m0.m2, 8.84);
	// UNUSED: u_var3.m1[0]
	result *= compare_bool(u_var3.m1[1], true);
	// UNUSED: u_var3.m1[2]
	result *= compare_bool(u_var3.m1[3], true);
	// UNUSED: u_var3.m2[0]
	result *= compare_ivec3(u_var3.m2[1], ivec3(-10, 2, -5));
	// UNUSED: u_var3.m2[2]
	result *= compare_ivec3(u_var3.m2[3], ivec3(-10, -2, -6));
	// UNUSED: u_var3.m2[4]
	result *= compare_ivec2(u_var3.m3[0], ivec2(8, 1));
	// UNUSED: u_var3.m3[1]
	result *= compare_float(u_var4[0], 9.00);
	// UNUSED: u_var4[1]
	result *= compare_float(u_var4[2], 0.33);
    gl_FragColor = vec4(result, result, result, 1.0);
}
