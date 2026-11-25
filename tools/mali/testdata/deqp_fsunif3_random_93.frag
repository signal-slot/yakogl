varying mediump float v_vtxOut;

struct structType0
{
	bvec2 m0;
	mediump samplerCube m1;
	bvec4 m2;
};
uniform bvec4 u_var0[2];
uniform mediump ivec3 u_var1;
uniform mediump samplerCube u_var2;
uniform structType0 u_var3;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bvec4(u_var0[0], bvec4(false, false, false, false));
	// UNUSED: u_var0[1]
	result *= compare_ivec3(u_var1, ivec3(0, 0, 0));
	// UNUSED: u_var2
	result *= compare_bvec2(u_var3.m0, bvec2(false, false));
	// UNUSED: u_var3.m1
	result *= compare_bvec4(u_var3.m2, bvec4(false, false, false, false));
    gl_FragColor = vec4(result, result, result, 1.0);
}
