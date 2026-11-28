varying mediump float v_vtxOut;

struct structType
{
	mediump sampler2D m0;
	mediump samplerCube m1;
};
uniform structType u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_vec4(texture2D(u_var.m0, vec2(0.0)), vec4(0.69, 0.07, 0.62, 0.03));
	result *= compare_vec4(textureCube(u_var.m1, vec3(0.0)), vec4(0.70, 0.90, 0.16, 0.41));
    gl_FragColor = vec4(result, result, result, 1.0);
}
