attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform bvec4 u_var;

mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_bvec4(u_var, bvec4(false, true, false, true));
}
