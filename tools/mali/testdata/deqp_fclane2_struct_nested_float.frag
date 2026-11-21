varying mediump float v_vtxOut;

struct subSubStructType
{
	mediump float mss0;
	mediump vec4 mss1;
};
struct subStructType
{
	mediump vec4 ms0;
	mediump float ms1[2];
	subSubStructType ms2[2];
};
struct structType
{
	mediump float m0;
	subStructType m1;
	mediump vec4 m2;
};
uniform structType u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_float(u_var.m0, -7.21);
	result *= compare_vec4(u_var.m1.ms0, vec4(6.01, -2.88, -8.15, 6.77));
	result *= compare_float(u_var.m1.ms1[0], 4.83);
	result *= compare_float(u_var.m1.ms1[1], -2.23);
	result *= compare_float(u_var.m1.ms2[0].mss0, 6.20);
	result *= compare_vec4(u_var.m1.ms2[0].mss1, vec4(2.02, 6.73, -0.34, 9.35));
	result *= compare_float(u_var.m1.ms2[1].mss0, -6.03);
	result *= compare_vec4(u_var.m1.ms2[1].mss1, vec4(4.55, 5.51, 8.74, 6.78));
	result *= compare_vec4(u_var.m2, vec4(0.77, -6.66, -9.88, 3.16));
    gl_FragColor = vec4(result, result, result, 1.0);
}
