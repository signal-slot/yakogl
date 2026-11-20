varying mediump vec4 v_in0;

void main()
{
	mediump float in0 = v_in0.z;
	mediump float res = float(0.0);

	res = asin(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
    gl_FragColor = color;
}
