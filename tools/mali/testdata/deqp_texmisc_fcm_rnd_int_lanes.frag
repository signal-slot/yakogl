precision mediump float;
const float a = 2.0;
const vec4 c = vec4(ivec4(true, a, true, 4).a, 10 - int(7.0), 4.25 + float(-1.0), int(-1) * int(1));
varying mediump vec3 d;

void main()
{
	gl_FragColor = vec4(-3.5, -3.5, 7, -3.5).zxyw.zwyx;
	gl_FragColor = (vec4(-3.0, -2.5, -5.5, 9) - gl_FragColor);
	float b = (d.y);
	b = float(int(int(b)));
	gl_FragColor = c * (gl_FragColor);
	gl_FragColor = gl_FragColor + vec4(float(b), float(-2.5), int(7.0), a);
}
