varying mediump vec4 v_in0;
varying mediump vec4 v_in1;
varying mediump vec4 v_in2;

void main()
{
	mediump float in0 = v_in0.z;
	mediump float in1 = v_in1.x;
	mediump float in2 = v_in2.y;
	mediump float res = float(0.0);

	res = smoothstep(in0, in1, in2);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
	color = color * 0.50 + 0.50;
    gl_FragColor = color;
}
