varying mediump vec4 v_in0;
varying mediump vec4 v_in1;

void main()
{
	mediump vec4 in0 = v_in0.yzwx;
	mediump float in1 = v_in1.x;
	mediump vec4 res = vec4(0.0);

	res = max(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyzw = res;
	color = color * 0.50 + 0.50;
    gl_FragColor = color;
}
