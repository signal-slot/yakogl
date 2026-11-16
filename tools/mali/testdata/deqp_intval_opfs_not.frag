varying mediump vec4 v_in0;

void main()
{
	bool in0 = (v_in0.z > 0.0);
	bool res = bool(0.0);

	res = !in0;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    gl_FragColor = color;
}
