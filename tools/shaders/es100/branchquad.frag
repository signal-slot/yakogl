/* Two discards behind tests of lanes y and w of a VARYING, so the condition
 * differs from pixel to pixel. Case 73 took its conditions from a uniform:
 * every pixel took the same branch, and on silicon it could not tell a
 * branch that read the right lane from one that did not.
 *
 * Case 81 draws v_color = (1 - t, t, 1 - s, s), s the horizontal position
 * and t the vertical one, both 0..1 across the quad. The y test discards the
 * top half and the w test the right half, so only the bottom-left quadrant is
 * green. A test that reads lane x instead of y discards the bottom half; one
 * that reads z instead of w discards the left half: every mistake moves the
 * green quadrant. */
precision mediump float;
varying vec4 v_color;
void main()
{
	if (v_color.y > 0.5) {
		discard;
	}
	vec4 col = vec4(0.0, 1.0, 0.0, 1.0);
	if (v_color.w > 0.5) {
		discard;
	}
	gl_FragColor = col;
}
