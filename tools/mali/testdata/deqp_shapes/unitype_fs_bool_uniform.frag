varying mediump float v_vtxOut;

uniform bool u_var[3];

mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_bool(u_var[0], true);
	result *= compare_bool(u_var[1], true);
	result *= compare_bool(u_var[2], true);
    gl_FragColor = vec4(result, result, result, 1.0);
}
