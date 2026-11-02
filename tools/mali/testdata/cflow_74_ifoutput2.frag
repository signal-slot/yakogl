/* The colour written before an `if` and again inside it, never read: the
 * store in the arm ran unconditionally, so the colour was red whatever the
 * condition said. Case 74's second program. */
precision mediump float;
uniform vec4 u_c;
void main()
{
	gl_FragColor = vec4(0.0, 1.0, 0.0, 1.0);
	if (u_c.x > 0.5) {
		gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);
	}
}
