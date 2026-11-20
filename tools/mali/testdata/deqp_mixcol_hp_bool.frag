varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp vec2 in0 = v_in0.wy;
	highp vec2 in1 = v_in1.yx;
	bvec2 res = bvec2(0.0);

	res = equal(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = vec2(res);
    gl_FragColor = color;
}
