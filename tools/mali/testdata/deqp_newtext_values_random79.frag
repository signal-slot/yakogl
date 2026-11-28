varying mediump float v_vtxOut;

struct structType1
{
	mediump samplerCube m0;
	mediump ivec4 m1[3];
	mediump ivec4 m2;
};
struct structType3
{
	bvec2 m0;
	mediump mat3 m1[2];
	mediump samplerCube m2;
};
struct structType2
{
	mediump mat4 m0;
	mediump vec2 m1;
	structType3 m2[5];
};
struct structType0
{
	mediump vec2 m0[3];
	structType1 m1[4];
	bool m2;
	mediump vec4 m3;
	structType2 m4[2];
};
uniform structType0 u_var0;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec2(u_var0.m0[0], vec2(0.00, 0.00));
	result *= compare_vec2(u_var0.m0[1], vec2(0.00, 0.00));
	result *= compare_vec2(u_var0.m0[2], vec2(0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m1[0].m0, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_ivec4(u_var0.m1[0].m1[0], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[0].m1[1], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[0].m1[2], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[0].m2, ivec4(0, 0, 0, 0));
	result *= compare_vec4(textureCube(u_var0.m1[1].m0, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_ivec4(u_var0.m1[1].m1[0], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[1].m1[1], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[1].m1[2], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[1].m2, ivec4(0, 0, 0, 0));
	result *= compare_vec4(textureCube(u_var0.m1[2].m0, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_ivec4(u_var0.m1[2].m1[0], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[2].m1[1], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[2].m1[2], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[2].m2, ivec4(0, 0, 0, 0));
	result *= compare_vec4(textureCube(u_var0.m1[3].m0, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_ivec4(u_var0.m1[3].m1[0], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[3].m1[1], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[3].m1[2], ivec4(0, 0, 0, 0));
	result *= compare_ivec4(u_var0.m1[3].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m2, false);
	result *= compare_vec4(u_var0.m3, vec4(0.00, 0.00, 0.00, 0.00));
	result *= compare_mat4(u_var0.m4[0].m0, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec2(u_var0.m4[0].m1, vec2(0.00, 0.00));
	result *= compare_bvec2(u_var0.m4[0].m2[0].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[0].m2[0].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[0].m2[0].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[0].m2[0].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec2(u_var0.m4[0].m2[1].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[0].m2[1].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[0].m2[1].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[0].m2[1].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec2(u_var0.m4[0].m2[2].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[0].m2[2].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[0].m2[2].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[0].m2[2].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec2(u_var0.m4[0].m2[3].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[0].m2[3].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[0].m2[3].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[0].m2[3].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec2(u_var0.m4[0].m2[4].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[0].m2[4].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[0].m2[4].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[0].m2[4].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_mat4(u_var0.m4[1].m0, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec2(u_var0.m4[1].m1, vec2(0.00, 0.00));
	result *= compare_bvec2(u_var0.m4[1].m2[0].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[1].m2[0].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[1].m2[0].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[1].m2[0].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec2(u_var0.m4[1].m2[1].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[1].m2[1].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[1].m2[1].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[1].m2[1].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec2(u_var0.m4[1].m2[2].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[1].m2[2].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[1].m2[2].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[1].m2[2].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec2(u_var0.m4[1].m2[3].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[1].m2[3].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[1].m2[3].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[1].m2[3].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec2(u_var0.m4[1].m2[4].m0, bvec2(false, false));
	result *= compare_mat3(u_var0.m4[1].m2[4].m1[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var0.m4[1].m2[4].m1[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m4[1].m2[4].m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
    gl_FragColor = vec4(result, result, result, 1.0);
}
