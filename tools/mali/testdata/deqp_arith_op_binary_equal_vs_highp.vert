attribute highp vec4 a_position;
attribute highp vec4 a_in0;
attribute highp vec4 a_in1;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	highp ivec2 in0 = ivec2(a_in0.wy);
	highp ivec2 in1 = ivec2(a_in1.yx);
	bool res = bool(0.0);

	res = in0 == in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    v_color = color;
}
