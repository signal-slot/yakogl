/* glmark2 ideas under-table shape (038e+23c8): a varying that is a whole
 * constant with three 0.0 lanes, read whole as the colour. */
attribute vec3 vertex;
uniform mat4 mvp;
varying vec4 color;
void main()
{
	gl_Position = mvp * vec4(vertex, 1.0);
	color = vec4(0.0, 0.0, 0.0, 1.0);
}
