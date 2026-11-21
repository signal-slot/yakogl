varying mediump float v_vtxOut;

uniform mediump ivec2 u_var;

mediump float compare_ivec2 (mediump ivec2 a, mediump ivec2 b) { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_ivec2(u_var, ivec2(-5, 7));
    gl_FragColor = vec4(result, result, result, 1.0);
}
