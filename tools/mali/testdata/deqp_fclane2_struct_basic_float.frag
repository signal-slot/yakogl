varying mediump float v_vtxOut;

struct structType
{
	mediump float m0;
	mediump vec4 m1;
};
uniform structType u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_float(u_var.m0, -7.21);
	result *= compare_vec4(u_var.m1, vec4(6.01, -2.88, -8.15, 6.77));
    gl_FragColor = vec4(result, result, result, 1.0);
}
