attribute highp vec4 a_position;
attribute highp vec4 a_in0;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	highp vec2 in0 = a_in0.wy;
	highp vec2 res = vec2(0.0);

	res = fract(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = res;
	color = color * 0.80 + 0.10;
    v_color = color;
}
