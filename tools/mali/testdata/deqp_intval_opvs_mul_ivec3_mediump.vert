attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
attribute mediump vec4 a_in1;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	mediump ivec3 in0 = ivec3(a_in0.zxy);
	mediump ivec3 in1 = ivec3(a_in1.yzx);
	mediump ivec3 res = ivec3(0.0);

	res = in0 * in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = vec3(res);
	color = color * 0.10 + 0.50;
    v_color = color;
}
