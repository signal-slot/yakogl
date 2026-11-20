precision mediump float;
varying mediump vec3 c;
varying mediump vec3 b;
uniform samplerCube a;

void main()
{
	gl_FragColor = textureCube(a, c - b);
}
