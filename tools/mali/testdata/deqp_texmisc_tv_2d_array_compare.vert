attribute highp vec4 a_position;
varying mediump float v_vtxOut;

uniform mediump sampler2D u_var[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_vec4(texture2D(u_var[0], vec2(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	v_vtxOut *= compare_vec4(texture2D(u_var[1], vec2(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	v_vtxOut *= compare_vec4(texture2D(u_var[2], vec2(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
}
