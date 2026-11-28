varying mediump vec4 v_in0;

void main()
{
	mediump vec2 in0 = v_in0.xy;
	mediump float res = float(0.0);

	res = 1.0 / (in0.x - in0.y);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
	color = color * 0.50 + 0.50;
    gl_FragColor = color;
}
