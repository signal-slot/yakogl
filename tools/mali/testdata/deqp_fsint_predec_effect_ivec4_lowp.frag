varying lowp vec4 v_in0;

void main()
{
	lowp ivec4 in0 = ivec4(v_in0.yzwx);
	lowp ivec4 res = ivec4(0.0);

	res = in0;
	--res;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyzw = vec4(res);
	color = color * 0.10 + 0.50;
    gl_FragColor = color;
}
