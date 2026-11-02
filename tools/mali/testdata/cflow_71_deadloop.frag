/* Work nobody reads, before a loop and inside it, around a loop that must
 * still run its three trips. `unused` is an if with an early return (joined
 * by selects, the shape femtovg's scissorMask takes when the variant does not
 * use it) and `waste` is carried round the loop and never read after it. The
 * online back end deletes both, which renumbers every instruction after them:
 * the break's target, the back edge's target and the back-edge list the ABI
 * checks. With u_k = (1, 0.25, 1, 0): red = 3 * 0.25 = 0.75. */
precision mediump float;
uniform vec4 u_k;
float unused(vec2 p)
{
	if (u_k.z > 0.0) {
		return p.x * 2.0 + p.y;
	}
	return p.y * 3.0;
}
void main()
{
	float dead = unused(u_k.xy);
	float n = ceil(u_k.x * 3.0);
	float acc = 0.0;
	float waste = 0.0;
	for (float i = 0.0; i < 8.0; i += 1.0) {
		if (i >= n) {
			break;
		}
		waste = waste * 3.0 + i;
		acc += u_k.y;
	}
	gl_FragColor = vec4(acc, 0.0, 0.0, 1.0);
}
