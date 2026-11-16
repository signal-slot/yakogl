varying mediump float v_vtxOut;

uniform mediump vec3 u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec3(u_var, vec3(-0.56, -6.37, 3.12));
    gl_FragColor = vec4(result, result, result, 1.0);
}
