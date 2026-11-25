attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump mat3 u_var0[5];
uniform mediump ivec4 u_var1;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	// UNUSED: u_var0[0]
	// UNUSED: u_var0[1]
	v_vtxOut *= compare_mat3(u_var0[2], mat3(4.47, 9.20, 6.05, 5.92, 8.01, 4.48, 3.36, 9.52, 8.65));
	// UNUSED: u_var0[3]
	// UNUSED: u_var0[4]
	// UNUSED: u_var1
}
