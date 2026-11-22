precision mediump float;
const bool e = ((abs(exp(float(0) + float(-2)) - ((float(0)))) < 0.001) == true);
varying mediump float d;
const int c = (ivec3(-7, 8, -4) * ivec3(5, 6.75, 1.0)).p - int(float(-5.5) + 0.5) * ivec4(true, true, 1.0, 0.25).wyxz.a;
int b = (c * c);
uniform sampler2D a;

void main()
{
	gl_FragColor = texture2DProj(a, exp2(vec4(e, d, c, b).ywx).xyz);
}
