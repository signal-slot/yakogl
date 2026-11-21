attribute highp vec4 a_position;
attribute lowp vec4 a_in0;
attribute lowp vec4 a_in1;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	lowp float in0 = a_in0.w;
	lowp float in1 = a_in1.y;
	bool res = bool(0.0);

	res = in0 < in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    v_color = color;
}
