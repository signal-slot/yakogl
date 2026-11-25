varying mediump float v_vtxOut;

uniform mediump ivec4 u_var[3];

mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_ivec4(u_var[0], ivec4(-7, -2, -5, -5));
	result *= compare_ivec4(u_var[1], ivec4(-6, -5, 1, -5));
	result *= compare_ivec4(u_var[2], ivec4(-6, -1, 3, 7));
    gl_FragColor = vec4(result, result, result, 1.0);
}
