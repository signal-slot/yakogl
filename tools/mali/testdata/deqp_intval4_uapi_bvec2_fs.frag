varying mediump float v_vtxOut;

uniform bvec2 u_var;

mediump float compare_bvec2 (bvec2 a, bvec2 b) { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bvec2(u_var, bvec2(false, true));
    gl_FragColor = vec4(result, result, result, 1.0);
}
