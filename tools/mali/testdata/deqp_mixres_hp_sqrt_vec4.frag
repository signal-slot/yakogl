varying highp vec4 v_in0;

void main()
{
	highp vec4 in0 = v_in0.yzwx;
	highp vec4 res = vec4(0.0);

	res = sqrt(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyzw = res;
	color = color * 0.30;
    gl_FragColor = color;
}
