varying highp vec4 v_in0;
varying highp vec4 v_in1;

void main()
{
	highp vec3 in0 = v_in0.zxy;
	highp vec3 in1 = v_in1.yzx;
	bvec3 res = bvec3(0.0);

	res = equal(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = vec3(res);
    gl_FragColor = color;
}
