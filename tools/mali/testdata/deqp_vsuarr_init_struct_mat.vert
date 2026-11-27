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
	v_vtxOut *= compare_mat4(u_var.m0, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	v_vtxOut *= compare_mat2(u_var.m1, mat2(0.00, 0.00, 0.00, 0.00));
	v_vtxOut *= compare_mat4(u_var.m2[0], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	v_vtxOut *= compare_mat4(u_var.m2[1], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	v_vtxOut *= compare_mat4(u_var.m2[2], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	v_vtxOut *= compare_mat2(u_var.m3[0], mat2(0.00, 0.00, 0.00, 0.00));
	v_vtxOut *= compare_mat2(u_var.m3[1], mat2(0.00, 0.00, 0.00, 0.00));
	v_vtxOut *= compare_mat2(u_var.m3[2], mat2(0.00, 0.00, 0.00, 0.00));
}
