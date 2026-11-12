/* Two vec4 varyings (the vertex shader of fsload_hp_var2). */
attribute vec4 a_pos;
attribute vec4 a_a;
attribute vec4 a_b;
varying vec4 a;
varying vec4 b;
void main()
{
	gl_Position = a_pos;
	a = a_a;
	b = a_b;
}
