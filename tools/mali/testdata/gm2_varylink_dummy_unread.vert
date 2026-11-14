/* glmark2 conditionals/function/loop shape (3107, a21c, 0162, 3cb0): a
 * vec4 varying stored as the constant vec4(1.0) and read by no fragment
 * shader. */
attribute vec4 position;
uniform mat4 mvp;
varying vec4 dummy;
void main()
{
	dummy = vec4(1.0);
	gl_Position = mvp * position;
}
