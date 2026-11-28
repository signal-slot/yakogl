attribute vec4 dEQP_Position;
bool g = bool(int(0));
attribute vec3 h;
uniform mediump int i;

void main()
{
	gl_Position = dEQP_Position;
	g = vec3(-3, g, i) != h;
}
