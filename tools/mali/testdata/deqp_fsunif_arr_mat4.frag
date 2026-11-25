varying mediump float v_vtxOut;

uniform mediump mat4 u_var[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat4(u_var[0], mat4(-6.54, 3.69, -6.68, -9.62, 8.82, -4.14, -5.67, -9.65, -9.94, 6.12, 4.66, -8.50, 1.28, -4.42, 6.49, -1.35));
	result *= compare_mat4(u_var[1], mat4(3.75, 7.94, 4.66, 5.12, 1.78, -1.00, -9.94, 1.55, 3.52, -1.43, -0.07, -9.72, 0.59, -9.13, 3.87, 6.58));
	result *= compare_mat4(u_var[2], mat4(2.10, 5.79, -6.21, 0.39, -7.80, 8.61, -8.61, 1.54, -7.88, -3.80, 1.82, -6.40, 1.54, -6.67, 3.67, -9.96));
    gl_FragColor = vec4(result, result, result, 1.0);
}
