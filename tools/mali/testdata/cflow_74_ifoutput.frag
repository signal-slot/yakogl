/* The colour written in both arms of an `if` and never read. If-conversion
 * runs both arms and joins local SLOTS with a select, but an output that is
 * only written is not a slot, so the online compiler ran the two stores one
 * after the other and the else arm won whatever the condition said. (Read
 * gl_FragColor back and the front end makes it a slot, which was always
 * right -- so this shader must not.) Case 74 draws both sides. */
precision mediump float;
uniform vec4 u_c;
void main()
{
	if (u_c.x > 0.5) {
		gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);
	} else {
		gl_FragColor = vec4(0.0, 1.0, 0.0, 1.0);
	}
}
