varying mediump float v_vtxOut;

struct structType
{
	bool m0;
	bvec4 m1;
	bool m2[3];
	bvec4 m3[3];
};
uniform structType u_var;

mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bool(u_var.m0, false);
	result *= compare_bvec4(u_var.m1, bvec4(true, true, true, true));
	result *= compare_bool(u_var.m2[0], false);
	result *= compare_bool(u_var.m2[1], false);
	result *= compare_bool(u_var.m2[2], true);
	result *= compare_bvec4(u_var.m3[0], bvec4(false, false, false, true));
	result *= compare_bvec4(u_var.m3[1], bvec4(true, true, true, true));
	result *= compare_bvec4(u_var.m3[2], bvec4(false, false, false, true));
    gl_FragColor = vec4(result, result, result, 1.0);
}
