/* gl_FragCoord, which is window-relative with its origin at the BOTTOM left
 * (ES 1.00 7.2), into the colour: red = x / 64, green = y / 64 on a 64x64
 * target. gl_test case 84 draws it to the default framebuffer, which this
 * driver stores top-down and draws with the viewport mirrored (flip_y), and
 * to a framebuffer object, which it does not: the same pixel must read the
 * same value in both. */
precision mediump float;
void main()
{
	gl_FragColor = vec4(gl_FragCoord.x / 64.0, gl_FragCoord.y / 64.0, 0.0, 1.0);
}
