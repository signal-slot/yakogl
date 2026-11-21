varying mediump float v_vtxOut;

struct structType
{
	mediump int m0;
	mediump ivec4 m1;
};
uniform structType u_var;

mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_int(u_var.m0, 6);
	result *= compare_ivec4(u_var.m1, ivec4(3, -9, -5, 6));
    gl_FragColor = vec4(result, result, result, 1.0);
}
