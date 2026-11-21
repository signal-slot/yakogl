varying mediump float v_vtxOut;

struct structType
{
	mediump float m0;
	mediump vec4 m1;
};
uniform structType u_var[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_float(u_var[0].m0, -7.21);
	result *= compare_vec4(u_var[0].m1, vec4(6.01, -2.88, -8.15, 6.77));
	result *= compare_float(u_var[1].m0, 4.83);
	result *= compare_vec4(u_var[1].m1, vec4(2.02, 6.73, -0.34, 9.35));
	result *= compare_float(u_var[2].m0, -2.23);
	result *= compare_vec4(u_var[2].m1, vec4(4.55, 5.51, 8.74, 6.78));
    gl_FragColor = vec4(result, result, result, 1.0);
}
