varying mediump vec4 v_in0;

void main()
{
	mediump vec3 in0 = v_in0.zxy;
	mediump vec3 res = vec3(0.0);

	res = exp2(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
    gl_FragColor = color;
}
