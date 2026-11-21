varying mediump vec4 v_in0;
varying mediump vec4 v_in1;

void main()
{
	mediump ivec2 in0 = ivec2(v_in0.zx);
	mediump ivec2 in1 = ivec2(v_in1.yz);
	bool res = bool(0.0);

	res = in0 != in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    gl_FragColor = color;
}
