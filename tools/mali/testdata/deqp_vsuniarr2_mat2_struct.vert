attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType1
{
	mediump ivec2 m0;
	bvec2 m1;
};
struct structType0
{
	structType1 m0;
};
uniform mediump mat2 u_var0[4];
uniform mediump mat3 u_var1;
uniform structType0 u_var2;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_ivec2    (mediump ivec2 a, mediump ivec2 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_mat2(u_var0[0], mat2(0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0[1]
	v_vtxOut *= compare_mat2(u_var0[2], mat2(0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0[3]
	v_vtxOut *= compare_mat3(u_var1, mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var2.m0.m0
	v_vtxOut *= compare_bvec2(u_var2.m0.m1, bvec2(false, false));
}
