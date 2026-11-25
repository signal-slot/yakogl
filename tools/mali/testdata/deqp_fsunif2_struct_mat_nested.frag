varying mediump float v_vtxOut;

struct subSubStructType
{
	mediump mat4 mss0;
	mediump mat2 mss1;
};
struct subStructType
{
	mediump mat2 ms0;
	mediump mat4 ms1[2];
	subSubStructType ms2[2];
};
struct structType
{
	mediump mat4 m0;
	subStructType m1;
	mediump mat2 m2;
};
uniform structType u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat4(u_var.m0, mat4(8.13, 8.85, -1.32, -5.88, -7.06, 4.65, 4.95, -2.12, 6.16, -4.03, 0.96, 0.22, -4.94, 8.25, 7.87, -4.80));
	result *= compare_mat2(u_var.m1.ms0, mat2(1.06, 5.15, -8.10, 8.97));
	result *= compare_mat4(u_var.m1.ms1[0], mat4(-6.13, -9.16, 0.74, 8.37, -8.65, -2.56, 2.30, -3.43, 9.42, -8.78, 9.26, -7.02, 6.64, 6.67, -6.14, -9.77));
	result *= compare_mat4(u_var.m1.ms1[1], mat4(2.84, 5.64, -9.13, -8.93, -1.89, 4.47, 3.61, -5.31, -2.33, 3.27, -9.28, 0.26, 7.29, 5.35, -1.25, 5.97));
	result *= compare_mat4(u_var.m1.ms2[0].mss0, mat4(1.45, 1.39, -7.41, -6.43, 4.00, -2.03, -8.52, -7.48, -0.86, 3.80, -3.42, 3.90, -4.54, 5.60, 6.95, -9.46));
	result *= compare_mat2(u_var.m1.ms2[0].mss1, mat2(-1.24, -3.11, -7.88, 9.76));
	result *= compare_mat4(u_var.m1.ms2[1].mss0, mat4(-7.62, -5.59, -7.44, 1.06, -4.84, 8.10, 4.04, -5.85, -0.76, 8.53, 2.88, -9.61, 2.55, -5.08, -3.30, -3.14));
	result *= compare_mat2(u_var.m1.ms2[1].mss1, mat2(-2.29, -2.93, -1.54, 4.76));
	result *= compare_mat2(u_var.m2, mat2(-9.19, -9.99, 4.15, -4.74));
    gl_FragColor = vec4(result, result, result, 1.0);
}
