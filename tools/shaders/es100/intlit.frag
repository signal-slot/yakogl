#version 100
/* Integer literals are integers: gl_test case 72 (intlit).
 *
 * Until 2026-09-23 the in-process compiler lowered every integer literal as a
 * FLOAT constant, and the integer operations it met read the float's bits:
 * `i < 3` compared against 1077936128 and `i = i + 1` added 1065353216. Each
 * channel below is one shape that went wrong, chosen so that the old answer
 * and the right one are different colours:
 *
 *   red    an int-counted loop, three passes of 0.25: 0.75 (the old lowering
 *          ran it twice: 0.5)
 *   green  an int compare against a literal, FemtoVG's `texType == 2`: 1.0
 *          when int(u_in.x) is 2, 0.0 otherwise (the old lowering: never 1.0)
 *   blue   ivec2 and bvec2 constructors of constants: 1.0 (the old lowering
 *          folded both to float constants: 0.0)
 */
precision mediump float;
uniform vec4 u_in;
void main()
{
	float r = 0.0;
	for (int i = 0; i < 3; i = i + 1) {
		r += 0.25;
	}
	int t = int(u_in.x);
	float g = (t == 2) ? 1.0 : 0.0;
	ivec2 iv = ivec2(1, 2);
	bvec2 bv = bvec2(1, 0);
	float b = (iv.x + iv.y == 3 && bv.x && !bv.y) ? 1.0 : 0.0;
	gl_FragColor = vec4(r, g, b, 1.0);
}
