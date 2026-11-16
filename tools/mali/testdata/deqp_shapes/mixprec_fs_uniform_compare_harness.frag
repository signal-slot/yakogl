varying mediump float v_vtxOut;

struct structType
{
	mediump sampler2D m0;
	mediump samplerCube m1;
	mediump sampler2D m2[3];
	mediump samplerCube m3[3];
};
uniform structType u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec4(texture2D(u_var.m0, vec2(0.0)), vec4(0.69, 0.07, 0.62, 0.03));
	result *= compare_vec4(textureCube(u_var.m1, vec3(0.0)), vec4(0.70, 0.90, 0.16, 0.41));
	result *= compare_vec4(texture2D(u_var.m2[0], vec2(0.0)), vec4(0.98, 0.30, 0.69, 0.77));
	result *= compare_vec4(texture2D(u_var.m2[1], vec2(0.0)), vec4(0.51, 0.30, 0.42, 0.41));
	result *= compare_vec4(texture2D(u_var.m2[2], vec2(0.0)), vec4(0.05, 0.11, 0.98, 0.20));
	result *= compare_vec4(textureCube(u_var.m3[0], vec3(0.0)), vec4(0.38, 0.48, 0.09, 0.98));
	result *= compare_vec4(textureCube(u_var.m3[1], vec3(0.0)), vec4(0.11, 0.71, 0.73, 0.22));
	result *= compare_vec4(textureCube(u_var.m3[2], vec3(0.0)), vec4(0.26, 0.71, 0.72, 0.80));
    gl_FragColor = vec4(result, result, result, 1.0);
}
