attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	mediump vec4 in0 = a_in0.yzwx;
	mediump vec4 res = vec4(0.0);

	res = sqrt(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyzw = res;
	color = color * 0.30;
    v_color = color;
}
