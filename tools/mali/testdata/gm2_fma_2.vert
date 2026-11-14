/* The ideas text shape: the same mat4 * mat4 * vec4 with a vec2 attribute and
 * z = 0.0, so one column of the product has a constant operand. */
uniform mat4 projection;
uniform mat4 modelview;
attribute vec2 vertex;
varying vec4 v_c;
void main()
{
	vec4 curVertex = vec4(vertex.x, vertex.y, 0.0, 1.0);
	gl_Position = projection * modelview * curVertex;
	v_c = vec4(1.0);
}
