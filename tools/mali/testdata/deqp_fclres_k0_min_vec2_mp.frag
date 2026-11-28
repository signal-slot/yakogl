varying mediump vec4 v_in0;
varying mediump vec4 v_in1;

void main()
{
	mediump vec2 in0 = v_in0.wy;
	mediump float in1 = v_in1.x;
	mediump vec2 res = vec2(0.0);

	res = min(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = res;
	color = color * 0.50 + 0.50;
    gl_FragColor = color;
}
