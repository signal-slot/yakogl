attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType0
{
	mediump float m0;
	mediump mat4 m1[3];
};
uniform structType0 u_var0;
uniform mediump float u_var1;
uniform mediump ivec3 u_var2;
uniform mediump float u_var3;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_float(u_var0.m0, 0.00);
	// UNUSED: u_var0.m1[0]
	v_vtxOut *= compare_mat4(u_var0.m1[1], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	// UNUSED: u_var0.m1[2]
	v_vtxOut *= compare_float(u_var1, 0.00);
	// UNUSED: u_var2
	v_vtxOut *= compare_float(u_var3, 0.00);
}
