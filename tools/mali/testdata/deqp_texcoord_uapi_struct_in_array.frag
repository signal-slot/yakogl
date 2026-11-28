varying mediump float v_vtxOut;

struct structType
{
	mediump sampler2D m0;
	mediump samplerCube m1;
};
uniform structType u_var[3];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec4(texture2D(u_var[0].m0, vec2(0.0)), vec4(0.99, 0.31, 0.00, 0.70));
	result *= compare_vec4(textureCube(u_var[0].m1, vec3(0.0)), vec4(0.72, 0.12, 0.50, 0.95));
	result *= compare_vec4(texture2D(u_var[1].m0, vec2(0.0)), vec4(0.88, 0.67, 0.34, 0.23));
	result *= compare_vec4(textureCube(u_var[1].m1, vec3(0.0)), vec4(0.58, 0.46, 0.15, 0.37));
	result *= compare_vec4(texture2D(u_var[2].m0, vec2(0.0)), vec4(0.15, 0.42, 0.57, 0.37));
	result *= compare_vec4(textureCube(u_var[2].m1, vec3(0.0)), vec4(0.55, 0.97, 0.73, 0.01));
    gl_FragColor = vec4(result, result, result, 1.0);
}
