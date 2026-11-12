/* Two equal varying lanes: z is x (the vertex shader of fsload_dup). */
attribute vec4 a_pos;
attribute vec4 a_a;
varying vec4 a;
void main()
{
	gl_Position = a_pos;
	a = vec4(a_a.xy, a_a.x, 1.0);
}
