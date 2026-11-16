varying lowp vec4 v_in0;

void main()
{
	lowp float in0 = v_in0.z;
	lowp float res = float(0.0);

	res = -in0;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
	color = color * 0.50 + 0.50;
    gl_FragColor = color;
}
