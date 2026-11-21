varying mediump vec4 v_in0;

void main()
{
	mediump int in0 = int(v_in0.z);
	bool res = bool(0.0);

	res = !(in0 == 2);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    gl_FragColor = color;
}
