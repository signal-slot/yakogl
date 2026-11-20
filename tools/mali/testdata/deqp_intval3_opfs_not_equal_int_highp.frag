varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp int in0 = int(v_in0.z);
	highp int in1 = int(v_in1.x);
	bool res = bool(0.0);

	res = in0 != in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    gl_FragColor = color;
}
