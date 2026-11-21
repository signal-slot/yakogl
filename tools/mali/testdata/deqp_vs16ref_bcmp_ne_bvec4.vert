attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
attribute mediump vec4 a_in1;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	bvec4 in0 = greaterThan(a_in0.wzyx, vec4(0.0));
	bvec4 in1 = greaterThan(a_in1.yzwx, vec4(0.0));
	bool res = bool(0.0);

	res = in0 != in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = float(res);
    v_color = color;
}
