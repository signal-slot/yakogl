varying mediump float v_vtxOut;

struct structType2
{
	mediump vec2 m0;
	mediump float m1;
};
struct structType3
{
	mediump ivec4 m0[2];
	bool m1;
	mediump int m2[1];
};
struct structType1
{
	mediump vec2 m0[2];
	mediump int m1;
	structType2 m2[1];
	structType3 m3;
};
struct structType0
{
	structType1 m0;
	mediump int m1[1];
};
uniform structType0 u_var0;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec2(u_var0.m0.m0[0], vec2(-3.19, 4.10));
	// UNUSED: u_var0.m0.m0[1]
	result *= compare_int(u_var0.m0.m1, 1);
	// UNUSED: u_var0.m0.m2[0].m0
	// UNUSED: u_var0.m0.m2[0].m1
	// UNUSED: u_var0.m0.m3.m0[0]
	result *= compare_ivec4(u_var0.m0.m3.m0[1], ivec4(-4, -4, -5, 1));
	// UNUSED: u_var0.m0.m3.m1
	result *= compare_int(u_var0.m0.m3.m2[0], 4);
	// UNUSED: u_var0.m1[0]
    gl_FragColor = vec4(result, result, result, 1.0);
}
