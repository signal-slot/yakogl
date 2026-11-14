/* reflect(I, N) = I - 2 * dot(N, I) * N on vec4, a built-in with a
 * multiply-add per lane. */
attribute vec4 a_pos;
attribute vec4 aI;
uniform vec4 aN;
varying vec4 v_c;
void main()
{
	v_c = reflect(aI, aN);
	gl_Position = a_pos;
}
