attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump sampler2D u_s2d;
uniform mediump samplerCube u_scube;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_vec4(texture2D(u_s2d, vec2(0.0)), vec4(0.15, 0.52, 0.26, 0.35));
	v_vtxOut *= compare_vec4(textureCube(u_scube, vec3(0.0)), vec4(0.88, 0.09, 0.30, 0.61));
}
