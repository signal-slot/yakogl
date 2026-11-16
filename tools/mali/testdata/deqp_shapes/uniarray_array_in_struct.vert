attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType
{
	mediump float m0;
	mediump vec4 m1;
	mediump float m2[3];
	mediump vec4 m3[3];
};
uniform structType u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_float(u_var.m0, -7.21);
	v_vtxOut *= compare_vec4(u_var.m1, vec4(6.01, -2.88, -8.15, 6.77));
	v_vtxOut *= compare_float(u_var.m2[0], 4.83);
	v_vtxOut *= compare_float(u_var.m2[1], -2.23);
	v_vtxOut *= compare_float(u_var.m2[2], 6.20);
	v_vtxOut *= compare_vec4(u_var.m3[0], vec4(2.02, 6.73, -0.34, 9.35));
	v_vtxOut *= compare_vec4(u_var.m3[1], vec4(4.55, 5.51, 8.74, 6.78));
	v_vtxOut *= compare_vec4(u_var.m3[2], vec4(0.77, -6.66, -9.88, 3.16));
}
