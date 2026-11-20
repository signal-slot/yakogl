varying mediump float v_vtxOut;

struct structType
{
	mediump int m0;
	mediump ivec4 m1;
	mediump int m2[3];
	mediump ivec4 m3[3];
};
uniform structType u_var;

mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_int(u_var.m0, 6);
	result *= compare_ivec4(u_var.m1, ivec4(3, -9, -5, 6));
	result *= compare_int(u_var.m2[0], -5);
	result *= compare_int(u_var.m2[1], -2);
	result *= compare_int(u_var.m2[2], -9);
	result *= compare_ivec4(u_var.m3[0], ivec4(1, -9, 3, -10));
	result *= compare_ivec4(u_var.m3[1], ivec4(-2, 7, 3, -6));
	result *= compare_ivec4(u_var.m3[2], ivec4(-3, 8, -7, -2));
    gl_FragColor = vec4(result, result, result, 1.0);
}
