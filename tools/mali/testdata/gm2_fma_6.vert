/* mod(x, y) on two floats: x - y * floor(x / y), a fused multiply-add. */
attribute vec4 a_pos;
uniform float k;
uniform float t;
varying vec4 v_c;
void main()
{
	v_c = vec4(mod(k, t));
	gl_Position = a_pos;
}
