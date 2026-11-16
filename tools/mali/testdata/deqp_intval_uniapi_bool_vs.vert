attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform bool u_var;

mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_bool(u_var, true);
}
