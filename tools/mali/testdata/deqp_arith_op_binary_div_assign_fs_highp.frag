varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp float in0 = v_in0.z;
	highp float in1 = v_in1.x;
	highp float res = float(0.0);

	res = in0;
	res /= in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
	color = color * 0.25 + 0.50;
    gl_FragColor = color;
}
