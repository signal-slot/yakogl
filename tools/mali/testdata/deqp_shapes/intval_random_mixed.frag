precision mediump float;
varying mediump float c;
varying mediump vec3 b;
uniform samplerCube a;

void main()
{
	gl_FragColor = textureCube(a, vec3(float(-2.0), bool(1), int(c)).gbr + b);
}
