attribute highp vec4 a_position;
attribute lowp vec4 a_in0;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	lowp vec3 in0 = a_in0.zxy;
	lowp vec3 res = vec3(0.0);

	res = fract(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
	color = color * 0.80 + 0.10;
    v_color = color;
}
