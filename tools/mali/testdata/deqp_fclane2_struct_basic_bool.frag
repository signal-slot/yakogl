varying mediump float v_vtxOut;

struct structType
{
	bool m0;
	bvec4 m1;
};
uniform structType u_var;

mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bool(u_var.m0, false);
	result *= compare_bvec4(u_var.m1, bvec4(true, true, true, true));
    gl_FragColor = vec4(result, result, result, 1.0);
}
