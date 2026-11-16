varying mediump float v_vtxOut;

uniform mediump mat2 u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat2(u_var, mat2(-7.48, 6.25, -0.95, 4.59));
    gl_FragColor = vec4(result, result, result, 1.0);
}
