attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump int u_var;

mediump float compare_int      (mediump int a, mediump int b)      { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_int(u_var, 0);
}
