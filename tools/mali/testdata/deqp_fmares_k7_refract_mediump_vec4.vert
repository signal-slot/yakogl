attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
attribute mediump vec4 a_in1;
attribute mediump vec4 a_in2;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	mediump vec4 in0 = a_in0.yzwx;
	mediump vec4 in1 = a_in1.wzyx;
	mediump float in2 = a_in2.y;
	mediump vec4 res = vec4(0.0);

	res = refract(in0, in1, in2);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyzw = res;
	color = color * 0.50 + 0.50;
    v_color = color;
}
