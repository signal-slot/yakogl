/* The colour attribute declared BEFORE the position, so it arrives in r0 and
 * the position in r1 -- while gl_Position goes to r0 and v_color to r1. The
 * online compiler wrote v_color (a copy of r0) into r1 first and then read r1
 * as the position: every vertex landed at its own colour. The scale keeps the
 * position an instruction rather than a register copy. */
attribute vec4 a_color;
attribute vec4 a_pos;
varying vec4 v_color;
void main()
{
	v_color = a_color;
	gl_Position = vec4(a_pos.xy * 0.75, 0.0, 1.0);
}
