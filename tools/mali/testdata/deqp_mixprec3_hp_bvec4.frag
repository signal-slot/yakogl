varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp vec4 in0 = v_in0.yzwx;
	highp vec4 in1 = v_in1.wzyx;
	bvec4 res = bvec4(0.0);

	res = lessThan(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyzw = vec4(res);
    gl_FragColor = color;
}
