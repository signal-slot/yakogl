varying mediump float v_vtxOut;

uniform mediump vec2 u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec2(u_var, vec2(-0.35, 7.81));
    gl_FragColor = vec4(result, result, result, 1.0);
}
