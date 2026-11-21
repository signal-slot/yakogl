varying mediump float v_vtxOut;

uniform mediump ivec4 u_var;

mediump float compare_ivec4 (mediump ivec4 a, mediump ivec4 b) { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_ivec4(u_var, ivec4(-1, 8, -4, 10));
    gl_FragColor = vec4(result, result, result, 1.0);
}
