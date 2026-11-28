attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType0
{
	mediump mat4 m0[2];
	mediump ivec3 m1[5];
	mediump vec2 m2;
	mediump mat3 m3;
	bvec4 m4;
};
uniform mediump int u_var0[4];
uniform mediump samplerCube u_var1;
uniform bvec4 u_var2[2];
uniform mediump vec3 u_var3;
uniform structType0 u_var4[1];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	// UNUSED: u_var0[0]
	// UNUSED: u_var0[1]
	v_vtxOut *= compare_int(u_var0[2], 0);
	// UNUSED: u_var0[3]
	v_vtxOut *= compare_vec4(textureCube(u_var1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	// UNUSED: u_var2[0]
	v_vtxOut *= compare_bvec4(u_var2[1], bvec4(false, false, false, false));
	v_vtxOut *= compare_vec3(u_var3, vec3(0.00, 0.00, 0.00));
	// UNUSED: u_var4[0].m0[0]
	v_vtxOut *= compare_mat4(u_var4[0].m0[1], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var4[0].m1[0]
	// UNUSED: u_var4[0].m1[1]
	v_vtxOut *= compare_ivec3(u_var4[0].m1[2], ivec3(0, 0, 0));
	// UNUSED: u_var4[0].m1[3]
	// UNUSED: u_var4[0].m1[4]
	v_vtxOut *= compare_vec2(u_var4[0].m2, vec2(0.00, 0.00));
	v_vtxOut *= compare_mat3(u_var4[0].m3, mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	v_vtxOut *= compare_bvec4(u_var4[0].m4, bvec4(false, false, false, false));
}
