varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp int in0 = int(v_in0.z);
	highp ivec2 in1 = ivec2(v_in1.yx);
	highp ivec2 res = ivec2(0.0);

	res = in0 * in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = vec2(res);
	color = color * 0.10 + 0.50;
    gl_FragColor = color;
}
