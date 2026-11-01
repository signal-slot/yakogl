/* A loop that reads, on every trip, two values made before it: the bound `n`
 * and the addend `k`, then makes a value of its own (`t`). The online
 * allocator ended both ranges at their last mention inside the body and gave
 * `t` their register, so from the second trip the test and the sum read `t`.
 * With u_k = (1, 0.25): n = 3, three trips, red = 3 * 0.25 = 0.75. */
precision mediump float;
uniform vec4 u_k;
void main()
{
	float n = ceil(u_k.x * 3.0);
	float k = u_k.y;
	float acc = 0.0;
	for (float i = 0.0; i < 8.0; i += 1.0) {
		if (i >= n) {
			break;
		}
		acc += k;
		float t = acc * 2.0;
		acc = t * 0.5;
	}
	gl_FragColor = vec4(acc, 0.0, 0.0, 1.0);
}
