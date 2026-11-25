attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump vec3 u_var0;
uniform bvec3 u_var1[5];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_bvec3    (bvec3 a, bvec3 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_vec3(u_var0, vec3(-3.19, -4.97, 9.79));
	// UNUSED: u_var1[0]
	v_vtxOut *= compare_bvec3(u_var1[1], bvec3(true, true, true));
	// UNUSED: u_var1[2]
	v_vtxOut *= compare_bvec3(u_var1[3], bvec3(true, false, true));
	// UNUSED: u_var1[4]
}
