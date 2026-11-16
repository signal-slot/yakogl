attribute highp vec4 a_position;
attribute highp vec4 a_in0;
attribute highp vec4 a_in1;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	highp int in0 = int(a_in0.z);
	highp int in1 = int(a_in1.x);
	highp int res = int(0.0);

	res = in0 + in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
	color = color * 0.10 + 0.50;
    v_color = color;
}
