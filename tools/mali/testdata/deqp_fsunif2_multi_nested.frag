varying mediump float v_vtxOut;

struct subSubStructType_0
{
	mediump float mss0;
	mediump vec4 mss1;
};
struct subStructType_0
{
	mediump vec4 ms0;
	mediump float ms1[2];
	subSubStructType_0 ms2[2];
};
struct structType_0
{
	mediump float m0;
	subStructType_0 m1;
	mediump vec4 m2;
};
struct subSubStructType_1
{
	mediump int mss0;
	mediump ivec4 mss1;
};
struct subStructType_1
{
	mediump ivec4 ms0;
	mediump int ms1[2];
	subSubStructType_1 ms2[2];
};
struct structType_1
{
	mediump int m0;
	subStructType_1 m1;
	mediump ivec4 m2;
};
struct subSubStructType_2
{
	bvec4 mss0;
	bool mss1;
};
struct subStructType_2
{
	bool ms0;
	bvec4 ms1[2];
	subSubStructType_2 ms2[2];
};
struct structType_2
{
	bvec4 m0;
	subStructType_2 m1;
	bool m2;
};
uniform structType_0 u_var_0;
uniform structType_1 u_var_1;
uniform structType_2 u_var_2;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_float(u_var_0.m0, 6.46);
	result *= compare_vec4(u_var_0.m1.ms0, vec4(0.44, -3.63, 3.08, -3.22));
	result *= compare_float(u_var_0.m1.ms1[0], 7.48);
	result *= compare_float(u_var_0.m1.ms1[1], 2.03);
	result *= compare_float(u_var_0.m1.ms2[0].mss0, -9.15);
	result *= compare_vec4(u_var_0.m1.ms2[0].mss1, vec4(8.64, -3.88, 5.92, 0.81));
	result *= compare_float(u_var_0.m1.ms2[1].mss0, -5.38);
	result *= compare_vec4(u_var_0.m1.ms2[1].mss1, vec4(-7.00, -3.42, 1.89, -4.08));
	result *= compare_vec4(u_var_0.m2, vec4(6.29, 1.77, -7.69, 1.08));
	result *= compare_int(u_var_1.m0, -2);
	result *= compare_ivec4(u_var_1.m1.ms0, ivec4(-9, 10, -4, 2));
	result *= compare_int(u_var_1.m1.ms1[0], -6);
	result *= compare_int(u_var_1.m1.ms1[1], 8);
	result *= compare_int(u_var_1.m1.ms2[0].mss0, 8);
	result *= compare_ivec4(u_var_1.m1.ms2[0].mss1, ivec4(10, -4, -7, 4));
	result *= compare_int(u_var_1.m1.ms2[1].mss0, 10);
	result *= compare_ivec4(u_var_1.m1.ms2[1].mss1, ivec4(9, 5, 5, 1));
	result *= compare_ivec4(u_var_1.m2, ivec4(-6, -9, -7, 5));
	result *= compare_bvec4(u_var_2.m0, bvec4(true, true, false, true));
	result *= compare_bool(u_var_2.m1.ms0, false);
	result *= compare_bvec4(u_var_2.m1.ms1[0], bvec4(true, false, false, false));
	result *= compare_bvec4(u_var_2.m1.ms1[1], bvec4(true, true, false, true));
	result *= compare_bvec4(u_var_2.m1.ms2[0].mss0, bvec4(false, true, true, true));
	result *= compare_bool(u_var_2.m1.ms2[0].mss1, false);
	result *= compare_bvec4(u_var_2.m1.ms2[1].mss0, bvec4(true, false, true, true));
	result *= compare_bool(u_var_2.m1.ms2[1].mss1, false);
	result *= compare_bool(u_var_2.m2, false);
    gl_FragColor = vec4(result, result, result, 1.0);
}
