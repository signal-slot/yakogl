varying mediump float v_vtxOut;

uniform bvec3 u_var;

mediump float compare_bvec3 (bvec3 a, bvec3 b) { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bvec3(u_var, bvec3(true, false, true));
    gl_FragColor = vec4(result, result, result, 1.0);
}
