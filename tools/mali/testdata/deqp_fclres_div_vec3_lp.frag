varying lowp vec4 v_in0;
varying lowp vec4 v_in1;

void main()
{
	lowp vec3 in0 = v_in0.zxy;
	lowp float in1 = v_in1.x;
	lowp vec3 res = vec3(0.0);

	res = in0;
	res /= in1;

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
	color = color * 0.25 + 0.50;
    gl_FragColor = color;
}
