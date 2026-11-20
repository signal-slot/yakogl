attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
attribute mediump vec4 a_in1;
attribute mediump vec4 a_in2;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	mediump float in0 = a_in0.z;
	mediump float in1 = a_in1.x;
	mediump float in2 = a_in2.y;
	mediump float res = float(0.0);

	res = clamp(in0, in1, in2);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
	color = color * 0.50 + 0.50;
    v_color = color;
}
