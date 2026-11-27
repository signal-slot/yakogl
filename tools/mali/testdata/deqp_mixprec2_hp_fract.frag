varying highp vec4 v_in0;

void main()
{
	highp float in0 = v_in0.z;
	highp float res = float(0.0);

	res = fract(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
	color = color * 0.80 + 0.10;
    gl_FragColor = color;
}
