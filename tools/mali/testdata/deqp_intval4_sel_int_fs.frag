varying mediump vec4 v_in0;
varying mediump vec4 v_in1;
varying mediump vec4 v_in2;

void main()
{
	bool in0 = (v_in0.z > 0.0);
	mediump int in1 = int(v_in1.x);
	mediump int in2 = int(v_in2.y);
	mediump int res = int(0.0);

	res = in0 ? in1 : in2;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
	color = color * 0.50 + 0.50;
    gl_FragColor = color;
}
