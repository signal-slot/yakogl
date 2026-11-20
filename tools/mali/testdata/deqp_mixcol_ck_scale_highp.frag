varying highp vec4 v_in0;

void main()
{
	highp vec2 in0 = v_in0.wy;
	highp vec2 res = vec2(0.0);

	res = +in0;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = res;
	color = color * 0.50 + 0.50;
    gl_FragColor = color;
}
