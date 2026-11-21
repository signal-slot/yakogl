attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType
{
	mediump int m0;
	mediump ivec4 m1;
};
uniform structType u_var;

mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return compare_int(a.x, b.x)*compare_int(a.y, b.y)*compare_int(a.z, b.z)*compare_int(a.w, b.w); }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_int(u_var.m0, 0);
	v_vtxOut *= compare_ivec4(u_var.m1, ivec4(0, 0, 0, 0));
}
