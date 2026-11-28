attribute highp vec4 a_position;
attribute highp vec4 a_in0;
attribute highp vec4 a_in1;
attribute highp vec4 a_in2;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	highp vec3 in0 = a_in0.zxy;
	highp vec3 in1 = a_in1.yzx;
	highp float in2 = a_in2.y;
	highp vec3 res = vec3(0.0);

	res = refract(in0, in1, in2);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
	color = color * 0.50 + 0.50;
    v_color = color;
}
