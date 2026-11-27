attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump ivec4 u_var[3];

mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_ivec4(u_var[0], ivec4(0, 0, 0, 0));
	v_vtxOut *= compare_ivec4(u_var[1], ivec4(0, 0, 0, 0));
	v_vtxOut *= compare_ivec4(u_var[2], ivec4(0, 0, 0, 0));
}
