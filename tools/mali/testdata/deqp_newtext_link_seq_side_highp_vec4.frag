varying highp vec4 v_in0;
varying highp vec4 v_in1;
varying highp vec4 v_in2;

void main()
{
	highp vec4 in0 = v_in0.yzwx;
	highp vec4 in1 = v_in1.wzyx;
	highp vec4 in2 = v_in2.xwzy;
	highp vec4 res = vec4(0.0);

	res = (in0++, in1 = in0 + in2, in2 = in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyzw = res;
	color = color * 0.50;
    gl_FragColor = color;
}
