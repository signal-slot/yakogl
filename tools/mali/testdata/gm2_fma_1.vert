/* glmark2 ideas shape: projection * modelview * curVertex, curVertex a local
 * vec4(x, y, z, 1.0) built from a vec3 attribute; a mat4 * mat4 * vec4. */
uniform mat4 projection;
uniform mat4 modelview;
attribute vec3 vertex;
varying vec4 v_c;
void main()
{
	vec4 curVertex = vec4(vertex.x, vertex.y, vertex.z, 1.0);
	gl_Position = projection * modelview * curVertex;
	v_c = vec4(1.0);
}
