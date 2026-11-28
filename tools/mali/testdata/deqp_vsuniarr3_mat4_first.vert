attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump mat4 u_var[3];
uniform mediump float u_var1;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_mat4(u_var[0], mat4(-1.64, -6.48, 9.24, 9.52, -7.98, -7.82, -3.77, -8.95, -2.57, 7.36, -9.86, -9.96, -7.05, 5.40, -1.93, 8.71));
	v_vtxOut *= compare_float(u_var1, 3.50);
	// UNUSED: u_var[1]
	// UNUSED: u_var[2]
}
