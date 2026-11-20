varying mediump vec4 v_in0;
varying mediump vec4 v_in1;

void main()
{
	mediump ivec3 in0 = ivec3(v_in0.zxy);
	mediump ivec3 in1 = ivec3(v_in1.yzx);
	bvec3 res = bvec3(0.0);

	res = notEqual(in0, in1);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = vec3(res);
    gl_FragColor = color;
}
