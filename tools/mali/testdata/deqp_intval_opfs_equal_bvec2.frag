varying mediump vec4 v_in0;
varying mediump vec4 v_in1;

void main()
{
	bvec2 in0 = greaterThan(v_in0.wy, vec2(0.0));
	bvec2 in1 = greaterThan(v_in1.yx, vec2(0.0));
	bool res = bool(0.0);

	res = in0 == in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    gl_FragColor = color;
}
