varying mediump vec4 v_in0;
varying mediump vec4 v_in1;
varying highp vec4 v_in2;

void main()
{
	bool in0 = (v_in0.z > 0.0);
	bool in1 = (v_in1.x > 0.0);
	highp vec2 in2 = v_in2.zy;
	highp ivec2 res = ivec2(0.0);

	res = (in1 = in0, in2++, in2 = in2 + vec2(in1), ivec2(in2));

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = vec2(res);
	color = color * 0.50;
    gl_FragColor = color;
}
