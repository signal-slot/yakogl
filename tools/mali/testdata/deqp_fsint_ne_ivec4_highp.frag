varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp ivec4 in0 = ivec4(v_in0.yzwx);
	highp ivec4 in1 = ivec4(v_in1.wzyx);
	bool res = bool(0.0);

	res = in0 != in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    gl_FragColor = color;
}
