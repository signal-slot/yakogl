varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp vec3 in0 = v_in0.zxy;
	highp vec3 in1 = v_in1.yzx;
	highp float res = float(0.0);

	res = dot(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.x = res;
	color = color * 0.10 + 0.50;
    gl_FragColor = color;
}
