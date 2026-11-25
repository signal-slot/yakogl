varying mediump float v_vtxOut;

struct structType2
{
	mediump int m0;
	mediump mat2 m1;
	mediump vec2 m2;
};
struct structType1
{
	mediump float m0;
	bool m1;
	mediump mat3 m2[2];
	structType2 m3[2];
};
struct structType0
{
	structType1 m0;
	mediump mat2 m1;
};
uniform mediump float u_var0;
uniform structType0 u_var1;
uniform mediump sampler2D u_var2;
uniform mediump mat3 u_var3;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }
mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_float(u_var0, 0.00);
	result *= compare_float(u_var1.m0.m0, 0.00);
	result *= compare_bool(u_var1.m0.m1, false);
	result *= compare_mat3(u_var1.m0.m2[0], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat3(u_var1.m0.m2[1], mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_int(u_var1.m0.m3[0].m0, 0);
	result *= compare_mat2(u_var1.m0.m3[0].m1, mat2(0.00, 0.00, 0.00, 0.00));
	result *= compare_vec2(u_var1.m0.m3[0].m2, vec2(0.00, 0.00));
	result *= compare_int(u_var1.m0.m3[1].m0, 0);
	result *= compare_mat2(u_var1.m0.m3[1].m1, mat2(0.00, 0.00, 0.00, 0.00));
	result *= compare_vec2(u_var1.m0.m3[1].m2, vec2(0.00, 0.00));
	result *= compare_mat2(u_var1.m1, mat2(0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(texture2D(u_var2, vec2(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_mat3(u_var3, mat3(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
    gl_FragColor = vec4(result, result, result, 1.0);
}
