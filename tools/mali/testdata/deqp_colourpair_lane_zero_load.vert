attribute highp vec4 a_position;
attribute highp vec4 a_in0;
attribute highp vec4 a_in1;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	highp float in0 = a_in0.z;
	highp float in1 = a_in1.x;
	highp float res = float(0.0);

	res = in0 + in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
    v_color = color;
}
