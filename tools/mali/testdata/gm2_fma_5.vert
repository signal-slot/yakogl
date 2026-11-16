/* smoothstep with uniform edges on a vec4 (the general e0, e1 form). */
attribute vec4 a_pos;
attribute vec4 x;
uniform float e0;
uniform float e1;
varying vec4 v_c;
void main()
{
	v_c = smoothstep(e0, e1, x);
	gl_Position = a_pos;
}
