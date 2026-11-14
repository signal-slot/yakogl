/* glmark2 shading cel shape (e5f2+fea0): a vec3 read whole and a vec4 of
 * which the fragment shader reads xyz only. */
attribute vec4 position;
attribute vec3 normal;
uniform mat4 mvp;
uniform mat4 mv;
varying vec3 vertex_normal;
varying vec4 vertex_position;
void main()
{
	vertex_normal = normal;
	vertex_position = mv * position;
	gl_Position = mvp * position;
}
