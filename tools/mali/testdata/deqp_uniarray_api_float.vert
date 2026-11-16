attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump float u_var[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_float(u_var[0], -0.03);
	v_vtxOut *= compare_float(u_var[1], -5.46);
	v_vtxOut *= compare_float(u_var[2], 0.74);
}
