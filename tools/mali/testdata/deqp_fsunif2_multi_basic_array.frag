varying mediump float v_vtxOut;

uniform mediump float u_var_0[3];
uniform mediump ivec3 u_var_1[3];
uniform bvec2 u_var_2[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_float(u_var_0[0], 6.46);
	result *= compare_float(u_var_0[1], 0.44);
	result *= compare_float(u_var_0[2], -3.63);
	result *= compare_ivec3(u_var_1[0], ivec3(0, 0, 3));
	result *= compare_ivec3(u_var_1[1], ivec3(3, 10, -7));
	result *= compare_ivec3(u_var_1[2], ivec3(-4, -5, 10));
	result *= compare_bvec2(u_var_2[0], bvec2(false, true));
	result *= compare_bvec2(u_var_2[1], bvec2(true, false));
	result *= compare_bvec2(u_var_2[2], bvec2(false, true));
    gl_FragColor = vec4(result, result, result, 1.0);
}
