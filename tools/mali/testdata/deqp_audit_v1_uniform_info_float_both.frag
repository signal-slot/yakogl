varying mediump float v_vtxOut;

uniform mediump float u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_float(u_var, 0.5);
    gl_FragColor = vec4(result, result, result, 1.0);
}
