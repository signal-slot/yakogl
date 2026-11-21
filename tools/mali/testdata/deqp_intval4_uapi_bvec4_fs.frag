varying mediump float v_vtxOut;

uniform bvec4 u_var;

mediump float compare_bvec4 (bvec4 a, bvec4 b) { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bvec4(u_var, bvec4(false, true, true, false));
    gl_FragColor = vec4(result, result, result, 1.0);
}
