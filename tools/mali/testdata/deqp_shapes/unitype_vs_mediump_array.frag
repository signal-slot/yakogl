varying mediump float v_vtxOut;

uniform mediump vec4 u_var[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec4(u_var[0], vec4(-8.74, 4.08, -6.25, 3.71));
	result *= compare_vec4(u_var[1], vec4(-7.02, 0.86, -6.45, -3.41));
	result *= compare_vec4(u_var[2], vec4(7.90, 1.20, 3.60, -4.79));
    gl_FragColor = vec4(result, result, result, 1.0);
}
