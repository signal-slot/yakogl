varying mediump float v_vtxOut;

struct structType1
{
	mediump vec2 m0;
	mediump mat4 m1[2];
};
struct structType0
{
	structType1 m0;
};
struct structType2
{
	mediump ivec3 m0;
	mediump ivec3 m1;
	mediump vec4 m2;
	mediump ivec3 m3[4];
};
uniform structType0 u_var0;
uniform mediump ivec2 u_var1;
uniform structType2 u_var2;
uniform mediump mat4 u_var3[1];
uniform mediump mat4 u_var4;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_ivec2    (mediump ivec2 a, mediump ivec2 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec2(u_var0.m0.m0, vec2(-3.17, -3.20));
	result *= compare_mat4(u_var0.m0.m1[0], mat4(8.42, 7.34, -9.92, 1.41, -9.94, -2.93, 7.02, 5.19, 9.24, 7.36, 3.37, 1.73, -9.62, 9.61, -5.47, -7.99));
	result *= compare_mat4(u_var0.m0.m1[1], mat4(-1.86, -0.83, -8.63, -9.42, -2.85, 8.99, -2.22, -2.32, 4.25, -0.13, -4.95, 2.06, 9.28, 5.69, -2.76, 6.49));
	result *= compare_ivec2(u_var1, ivec2(-2, -2));
	result *= compare_ivec3(u_var2.m0, ivec3(-2, 1, -9));
	result *= compare_ivec3(u_var2.m1, ivec3(10, -6, -6));
	result *= compare_vec4(u_var2.m2, vec4(7.37, 4.12, -0.65, 1.95));
	result *= compare_ivec3(u_var2.m3[0], ivec3(4, -7, -3));
	result *= compare_ivec3(u_var2.m3[1], ivec3(-8, 3, 5));
	result *= compare_ivec3(u_var2.m3[2], ivec3(-6, 4, -8));
	result *= compare_ivec3(u_var2.m3[3], ivec3(8, 9, -8));
	result *= compare_mat4(u_var3[0], mat4(8.12, -7.23, -5.60, 6.93, -9.22, 9.56, -2.48, 2.27, 9.60, -2.83, -3.69, -0.79, -4.22, 6.90, -0.43, -2.80));
	result *= compare_mat4(u_var4, mat4(1.46, 7.21, 7.09, 7.87, -2.72, 0.49, 7.20, -0.30, -0.50, 5.77, -7.57, -7.43, -3.27, 0.42, -1.71, 4.36));
    gl_FragColor = vec4(result, result, result, 1.0);
}
