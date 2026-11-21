varying mediump vec4 v_in0;
varying mediump vec4 v_in1;

void main()
{
	bvec3 in0 = greaterThan(v_in0.xyz, vec3(0.0));
	bvec3 in1 = greaterThan(v_in1.yzw, vec3(0.0));
	bool res = bool(0.0);
	bool res2 = bool(0.0);

	res = all(equal(in0, in1));
	res2 = any(notEqual(in0, in1));

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
	color.y = float(res2);
    gl_FragColor = color;
}
