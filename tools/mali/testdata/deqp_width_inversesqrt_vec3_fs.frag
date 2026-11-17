/* Fragment inversesqrt of a mediump vec3: dEQP-GLES2.functional.shaders.
 * operator.exponential.inversesqrt.mediump_vec3_fragment, the Mesa 26.2.1
 * drm-shim reference. Three lanes: the built-in's exact width, not the four
 * the IR's `comps` holds for it. */
varying mediump vec4 v_in0;

void main()
{
	mediump vec3 in0 = v_in0.zxy;
	mediump vec3 res = vec3(0.0);

	res = inversesqrt(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
    gl_FragColor = color;
}
