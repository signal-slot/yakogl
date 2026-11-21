attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform bvec3 u_var;

mediump float compare_bvec3    (bvec3 a, bvec3 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_bvec3(u_var, bvec3(true, false, true));
}
