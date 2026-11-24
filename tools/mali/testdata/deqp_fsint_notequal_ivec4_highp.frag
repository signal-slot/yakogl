varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp ivec4 in0 = ivec4(v_in0.yzwx);
	highp ivec4 in1 = ivec4(v_in1.wzyx);
	bvec4 res = bvec4(0.0);

	res = notEqual(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyzw = vec4(res);
    gl_FragColor = color;
}
