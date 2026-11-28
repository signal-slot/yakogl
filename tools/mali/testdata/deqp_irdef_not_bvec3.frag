varying mediump vec4 v_in0;

void main()
{
	bvec3 in0 = greaterThan(v_in0.zxy, vec3(0.0));
	bvec3 res = bvec3(0.0);

	res = not(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = vec3(res);
    gl_FragColor = color;
}
