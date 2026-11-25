attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump float u_var_0[3];
uniform mediump ivec3 u_var_1[3];
uniform bvec2 u_var_2[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_float(u_var_0[0], -3.43);
	v_vtxOut *= compare_float(u_var_0[1], -3.70);
	v_vtxOut *= compare_float(u_var_0[2], -6.13);
	v_vtxOut *= compare_ivec3(u_var_1[0], ivec3(-2, -8, 6));
	v_vtxOut *= compare_ivec3(u_var_1[1], ivec3(-2, 10, 5));
	v_vtxOut *= compare_ivec3(u_var_1[2], ivec3(8, -9, 7));
	v_vtxOut *= compare_bvec2(u_var_2[0], bvec2(true, true));
	v_vtxOut *= compare_bvec2(u_var_2[1], bvec2(true, true));
	v_vtxOut *= compare_bvec2(u_var_2[2], bvec2(false, false));
}
