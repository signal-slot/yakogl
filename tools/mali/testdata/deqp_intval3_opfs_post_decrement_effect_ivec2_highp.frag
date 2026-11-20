varying highp vec4 v_in0;

void main()
{
	highp ivec2 in0 = ivec2(v_in0.wy);
	highp ivec2 res = ivec2(0.0);

	res = in0;
	res--;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = vec2(res);
	color = color * 0.10 + 0.50;
    gl_FragColor = color;
}
