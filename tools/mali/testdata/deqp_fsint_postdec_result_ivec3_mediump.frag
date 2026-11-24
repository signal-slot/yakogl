varying mediump vec4 v_in0;

void main()
{
	mediump ivec3 in0 = ivec3(v_in0.zxy);
	mediump ivec3 res = ivec3(0.0);

	res = in0--;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = vec3(res);
	color = color * 0.10 + 0.50;
    gl_FragColor = color;
}
