/* smoothstep of a uniform with constant edges 0.0 and 1.0 (the front end's
 * clamp then t * t * (3 - 2 t), two multiply-adds). */
attribute vec4 a_pos;
uniform float k;
varying vec4 v_c;
void main()
{
	v_c = vec4(smoothstep(0.0, 1.0, k));
	gl_Position = a_pos;
}
