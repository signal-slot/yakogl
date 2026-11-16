precision mediump float;
varying mediump float c;
uniform mediump vec4 b;
const float a = vec3(true, 0.25, -7.75).zyx.y;

void main()
{
	gl_FragColor = (vec4(-3, c, 0, -1.5)) + b * vec4(a, 1, -1, -2);
}
