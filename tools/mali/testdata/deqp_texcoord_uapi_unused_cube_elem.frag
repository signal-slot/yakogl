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
	result *= compare_vec4(texture2D(u_var.m0, vec2(0.0)), vec4(0.99, 0.31, 0.00, 0.70));
	// UNUSED: u_var.m1
	// UNUSED: u_var.m2[0]
	// UNUSED: u_var.m2[1]
	// UNUSED: u_var.m2[2]
	// UNUSED: u_var.m3[0]
	result *= compare_vec4(textureCube(u_var.m3[1], vec3(0.0)), vec4(0.70, 0.51, 0.23, 0.35));
	// UNUSED: u_var.m3[2]
    gl_FragColor = vec4(result, result, result, 1.0);
}
