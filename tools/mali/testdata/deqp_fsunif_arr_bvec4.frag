varying mediump float v_vtxOut;

uniform bvec4 u_var[3];

mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bvec4(u_var[0], bvec4(true, true, false, false));
	result *= compare_bvec4(u_var[1], bvec4(true, true, true, false));
	result *= compare_bvec4(u_var[2], bvec4(true, false, true, false));
    gl_FragColor = vec4(result, result, result, 1.0);
}
