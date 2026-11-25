varying lowp vec4 v_in0;
varying lowp vec4 v_in1;

void main()
{
	lowp vec2 in0 = v_in0.wy;
	lowp vec2 in1 = v_in1.yx;
	lowp vec2 res = vec2(0.0);

	res = step(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = res;
	color = color * 0.50 + 0.25;
    gl_FragColor = color;
}
