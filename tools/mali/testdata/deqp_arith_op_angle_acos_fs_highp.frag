varying highp vec4 v_in0;

void main()
{
	highp float in0 = v_in0.z;
	highp float res = float(0.0);

	res = acos(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
    gl_FragColor = color;
}
