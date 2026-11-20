varying mediump vec4 v_in0;
varying mediump vec4 v_in1;
varying mediump vec4 v_in2;

void main()
{
	mediump vec3 in0 = v_in0.zxy;
	mediump vec3 in1 = v_in1.yzx;
	mediump float in2 = v_in2.y;
	mediump vec3 res = vec3(0.0);

	res = mix(in0, in1, in2);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
	color = color * 0.50 + 0.50;
    gl_FragColor = color;
}
