attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump ivec3 u_var;

mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }
mediump float compare_ivec3    (mediump ivec3 a, mediump ivec3 b)  { return compare_int(a.x, b.x)*compare_int(a.y, b.y)*compare_int(a.z, b.z); }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_ivec3(u_var, ivec3(0, 0, 0));
}
