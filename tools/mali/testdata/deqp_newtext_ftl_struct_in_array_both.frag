varying mediump float v_vtxOut;

struct structType
{
	mediump mat4 m0;
	mediump mat2 m1;
};
uniform structType u_var[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat4(u_var[0].m0, mat4(0.06, -2.44, -5.92, -1.02, -1.55, -0.51, -1.57, -4.93, -1.41, 9.84, -6.45, -2.33, 2.58, -5.73, 4.27, -0.83));
	result *= compare_mat2(u_var[0].m1, mat2(-3.24, 4.51, -6.93, 9.61));
	result *= compare_mat4(u_var[1].m0, mat4(-4.73, 6.87, 3.80, -1.06, 6.56, 5.42, -4.81, 2.96, 4.17, -2.63, 0.88, -0.85, -4.02, -2.14, 0.78, -8.66));
	result *= compare_mat2(u_var[1].m1, mat2(-1.96, 0.88, -5.33, 3.62));
	result *= compare_mat4(u_var[2].m0, mat4(1.63, 3.18, -0.00, -6.16, -8.35, 2.85, -2.29, -6.32, 7.43, -9.44, 5.88, 0.10, 0.13, -3.89, -9.21, -0.27));
	result *= compare_mat2(u_var[2].m1, mat2(4.29, -1.30, 3.41, 7.85));
    gl_FragColor = vec4(result, result, result, 1.0);
}
