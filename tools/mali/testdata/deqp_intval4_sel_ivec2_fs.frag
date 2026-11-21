varying mediump vec4 v_in0;
varying mediump vec4 v_in1;
varying mediump vec4 v_in2;

void main()
{
	bool in0 = (v_in0.z > 0.0);
	mediump ivec2 in1 = ivec2(v_in1.xw);
	mediump ivec2 in2 = ivec2(v_in2.yz);
	mediump ivec2 res = ivec2(0.0);

	res = in0 ? in1 : in2;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xy = vec2(res);
	color = color * 0.50 + 0.50;
    gl_FragColor = color;
}
