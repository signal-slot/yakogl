precision mediump float;
varying mediump vec3 d;
float c = (float(5)) - float(float(7));
varying mediump float b;
uniform samplerCube a;

void main()
{
	gl_FragColor = textureCube(a, d * vec3(1, c, b), b - b);
}
