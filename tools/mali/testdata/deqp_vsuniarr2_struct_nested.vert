attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType1
{
	bool m0;
	mediump vec3 m1;
	bvec4 m2[2];
	mediump mat3 m3[4];
	mediump ivec3 m4;
};
struct structType0
{
	structType1 m0[2];
};
uniform mediump float u_var0;
uniform bool u_var1[4];
uniform mediump float u_var2[4];
uniform structType0 u_var3;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_float(u_var0, 0.00);
	// UNUSED: u_var1[0]
	// UNUSED: u_var1[1]
	// UNUSED: u_var1[2]
	// UNUSED: u_var1[3]
	// UNUSED: u_var2[0]
	// UNUSED: u_var2[1]
	// UNUSED: u_var2[2]
	// UNUSED: u_var2[3]
	// UNUSED: u_var3.m0[0].m0
	// UNUSED: u_var3.m0[0].m1
	// UNUSED: u_var3.m0[0].m2[0]
	// UNUSED: u_var3.m0[0].m2[1]
	// UNUSED: u_var3.m0[0].m3[0]
	// UNUSED: u_var3.m0[0].m3[1]
	// UNUSED: u_var3.m0[0].m3[2]
	// UNUSED: u_var3.m0[0].m3[3]
	// UNUSED: u_var3.m0[0].m4
	v_vtxOut *= compare_bool(u_var3.m0[1].m0, false);
	// UNUSED: u_var3.m0[1].m1
	// UNUSED: u_var3.m0[1].m2[0]
	// UNUSED: u_var3.m0[1].m2[1]
	// UNUSED: u_var3.m0[1].m3[0]
	// UNUSED: u_var3.m0[1].m3[1]
	v_vtxOut *= compare_mat3(u_var3.m0[1].m3[2], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var3.m0[1].m3[3]
	v_vtxOut *= compare_ivec3(u_var3.m0[1].m4, ivec3(0, 0, 0));
}
