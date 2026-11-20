varying mediump vec4 v_in0;
varying mediump vec4 v_in1;

void main()
{
	bool in0 = (v_in0.z > 0.0);
	bool in1 = (v_in1.x > 0.0);
	bool res = bool(0.0);

	res = in0 && in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    gl_FragColor = color;
}
