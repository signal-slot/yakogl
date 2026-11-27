varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp float in0 = v_in0.z;
	highp vec3 in1 = v_in1.yzx;
	highp vec3 res = vec3(0.0);

	res = in0 + in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
    gl_FragColor = color;
}
