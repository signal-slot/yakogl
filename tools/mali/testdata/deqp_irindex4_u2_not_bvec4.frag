varying mediump vec4 v_in0;

void main()
{
	bvec4 in0 = greaterThan(v_in0.yzwx, vec4(0.0));
	bvec4 res = bvec4(0.0);

	res = not(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyzw = vec4(res);
    gl_FragColor = color;
}
