varying mediump float v_vtxOut;

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
