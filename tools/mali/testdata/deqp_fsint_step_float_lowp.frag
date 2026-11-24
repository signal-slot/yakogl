varying lowp vec4 v_in0;
varying lowp vec4 v_in1;

void main()
{
	lowp float in0 = v_in0.z;
	lowp float in1 = v_in1.x;
	lowp float res = float(0.0);

	res = step(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
	color = color * 0.50 + 0.25;
    gl_FragColor = color;
}
