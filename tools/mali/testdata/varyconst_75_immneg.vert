/* A position mapped from [0, 1] to clip space the way femtovg's main.vert
 * does it, by subtracting a literal: `x * 2.0 - 1.0`. The online compiler
 * encoded every negated literal by toggling the immediate's NEG field, which
 * is payload bit 17, so -1.0 became 5.4e-20 and the quad landed on [0, 2]:
 * one quadrant of the target instead of all of it. Case 75. */
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main()
{
	v_color = a_color;
	gl_Position = vec4(a_pos.xy * 2.0 - 1.0, 0.0, 1.0);
}
