attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType
{
	mediump mat4 m0;
	mediump mat2 m1;
	mediump mat4 m2[3];
	mediump mat2 m3[3];
};
uniform structType u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_mat4(u_var.m0, mat4(9.19, 6.40, -4.10, 0.30, -1.97, -9.58, 7.48, 2.61, 7.85, -2.04, -5.64, 0.48, -7.21, -2.85, -6.60, 1.90));
	v_vtxOut *= compare_mat2(u_var.m1, mat2(4.89, -2.34, 5.38, 5.84));
	v_vtxOut *= compare_mat4(u_var.m2[0], mat4(7.25, 9.94, -4.97, -8.87, -8.13, 8.77, 4.77, 2.56, 2.30, 0.61, 3.87, 6.58, 2.05, 8.53, 1.90, -8.98));
	v_vtxOut *= compare_mat4(u_var.m2[1], mat4(-6.80, 5.06, 7.83, 4.21, -3.19, 2.77, 8.26, 9.42, 1.13, -9.63, 7.67, -7.31, 2.12, -2.01, 4.13, -2.80));
	v_vtxOut *= compare_mat4(u_var.m2[2], mat4(-2.58, 3.58, -2.81, 6.51, -7.00, -1.74, -7.91, 1.16, 4.46, -8.19, -9.88, 9.74, -1.04, -0.49, -3.79, -8.31));
	v_vtxOut *= compare_mat2(u_var.m3[0], mat2(7.90, -8.99, -8.34, -1.91));
	v_vtxOut *= compare_mat2(u_var.m3[1], mat2(4.30, -8.60, -6.48, -2.83));
	v_vtxOut *= compare_mat2(u_var.m3[2], mat2(-5.58, 7.65, -6.77, -1.98));
}
