attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
attribute mediump vec4 a_in1;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	mediump float in0 = a_in0.z;
	mediump vec2 in1 = a_in1.yx;
	mediump vec2 res = vec2(0.0);

	res = in0 + in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = res;
    v_color = color;
}
