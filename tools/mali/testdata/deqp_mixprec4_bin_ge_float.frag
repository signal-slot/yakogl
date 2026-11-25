varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp float in0 = v_in0.x;
	highp float in1 = v_in1.y;
	bool res = false;

	res = in0 >= in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = vec2(res);
    gl_FragColor = color;
}
