varying mediump float v_vtxOut;

uniform mediump float u_var_0;
uniform mediump ivec3 u_var_1;
uniform mediump mat3 u_var_2;
uniform bvec2 u_var_3;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_float(u_var_0, 0.00);
	result *= compare_ivec3(u_var_1, ivec3(0, 0, 0));
	result *= compare_mat3(u_var_2, mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_bvec2(u_var_3, bvec2(false, false));
    gl_FragColor = vec4(result, result, result, 1.0);
}
