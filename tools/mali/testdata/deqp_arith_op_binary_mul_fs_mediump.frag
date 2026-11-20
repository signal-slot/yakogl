varying mediump vec4 v_in0;
varying mediump vec4 v_in1;

void main()
{
	mediump float in0 = v_in0.z;
	mediump vec3 in1 = v_in1.yzx;
	mediump vec3 res = vec3(0.0);

	res = in0 * in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
    gl_FragColor = color;
}
