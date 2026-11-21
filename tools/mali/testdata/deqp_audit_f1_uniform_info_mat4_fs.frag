varying mediump float v_vtxOut;

uniform mediump mat4 u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat4(u_var, mat4(0.5, -0.5, 1.5, 2.0, 0.25, 0.75, -1.0, 3.0, 1.25, -2.5, 0.5, 0.0, 2.0, 1.0, -0.5, 4.0));
    gl_FragColor = vec4(result, result, result, 1.0);
}
