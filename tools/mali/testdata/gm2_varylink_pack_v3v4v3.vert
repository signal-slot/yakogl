/* glmark2 ideas logo/lamp shape (51b2+c565, 51b2+1aca): a vec3, a vec4 and
 * a vec3, the second vec3 computed from the vec4, all read whole. */
attribute vec4 vertex;
attribute vec3 normal;
uniform mat4 modelview;
varying vec3 vertex_normal;
varying vec4 vertex_position;
varying vec3 eye_direction;
void main()
{
	gl_Position = vertex;
	vertex_normal = normal;
	vertex_position = modelview * vertex;
	eye_direction = -vertex_position.xyz;
}
