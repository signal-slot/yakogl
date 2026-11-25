attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType
{
	bool m0;
	bvec4 m1;
	bool m2[3];
	bvec4 m3[3];
};
uniform structType u_var;

mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_bool(u_var.m0, true);
	// UNUSED: u_var.m1
	// UNUSED: u_var.m2[0]
	// UNUSED: u_var.m2[1]
	// UNUSED: u_var.m2[2]
	// UNUSED: u_var.m3[0]
	v_vtxOut *= compare_bvec4(u_var.m3[1], bvec4(false, true, true, true));
	// UNUSED: u_var.m3[2]
}
