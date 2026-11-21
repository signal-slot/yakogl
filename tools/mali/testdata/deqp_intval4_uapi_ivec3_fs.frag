varying mediump float v_vtxOut;

uniform mediump ivec3 u_var;

mediump float compare_ivec3 (mediump ivec3 a, mediump ivec3 b) { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_ivec3(u_var, ivec3(4, -12, 3));
    gl_FragColor = vec4(result, result, result, 1.0);
}
