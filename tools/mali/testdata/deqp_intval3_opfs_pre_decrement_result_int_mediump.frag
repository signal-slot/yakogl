varying mediump vec4 v_in0;

void main()
{
	mediump int in0 = int(v_in0.z);
	mediump int res = int(0.0);

	res = --in0;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
	color = color * 0.10 + 0.50;
    gl_FragColor = color;
}
