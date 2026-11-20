varying mediump float v_vtxOut;

uniform mediump int u_var;

mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_int(u_var, -9);
    gl_FragColor = vec4(result, result, result, 1.0);
}
