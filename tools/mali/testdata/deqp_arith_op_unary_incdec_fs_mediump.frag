varying mediump vec4 v_in0;

void main()
{
	mediump vec3 in0 = v_in0.zxy;
	mediump vec3 res = vec3(0.0);

	res = in0;
	res--;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
	color = color * 0.50 + 1.00;
    gl_FragColor = color;
}
