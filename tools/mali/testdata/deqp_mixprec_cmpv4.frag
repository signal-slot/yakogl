varying mediump float v_vtxOut;

uniform mediump vec4 u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec4(u_var, vec4(-1.12, -8.66, 1.74, -5.62));
    gl_FragColor = vec4(result, result, result, 1.0);
}
