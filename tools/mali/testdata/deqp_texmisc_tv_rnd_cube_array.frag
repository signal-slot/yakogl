varying mediump float v_vtxOut;

uniform mediump samplerCube u_var0[4];
uniform mediump vec3 u_var1;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec4(textureCube(u_var0[0], vec3(0.0)), vec4(0.34, 0.62, 0.13, 0.08));
	result *= compare_vec4(textureCube(u_var0[1], vec3(0.0)), vec4(0.34, 0.93, 0.33, 0.58));
	result *= compare_vec4(textureCube(u_var0[2], vec3(0.0)), vec4(0.70, 0.10, 0.93, 0.00));
	result *= compare_vec4(textureCube(u_var0[3], vec3(0.0)), vec4(0.44, 0.89, 0.91, 0.41));
	result *= compare_vec3(u_var1, vec3(-8.96, 2.12, 6.51));
    gl_FragColor = vec4(result, result, result, 1.0);
}
