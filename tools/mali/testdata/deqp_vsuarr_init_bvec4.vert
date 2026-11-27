attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform bvec4 u_var[3];

mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_bvec4(u_var[0], bvec4(false, false, false, false));
	v_vtxOut *= compare_bvec4(u_var[1], bvec4(false, false, false, false));
	v_vtxOut *= compare_bvec4(u_var[2], bvec4(false, false, false, false));
}
