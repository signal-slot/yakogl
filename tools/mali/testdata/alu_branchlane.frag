/* Two discards, each behind a test of a lane that is not x: u_c.y, then
 * u_c.w. The online compiler fed each branch its condition as `.xyzw` where
 * only one lane held the comparison, so the other lanes -- whatever the
 * register held -- took part in the test. The reference feeds a branch one
 * lane, splatted (all 208 of its branch_unary). Case 73 draws the three
 * sides: neither test true (green), the y test true, the w test true (both
 * discarded, the clear colour stays). */
precision mediump float;
uniform vec4 u_c;
void main()
{
	if (u_c.y > 0.5) {
		discard;
	}
	vec4 col = vec4(0.0, 1.0, 0.0, 1.0);
	if (u_c.w > 0.5) {
		discard;
	}
	gl_FragColor = col;
}
