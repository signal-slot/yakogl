/* Case 70's loop (loopcarry.frag) with `acc * 3.0`: each pass is
 * `acc + k * 1.5` once the constants fold, a product no other lane pairs
 * with. */
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
		float t = acc * 3.0;
		acc = t * 0.5;
	}
	gl_FragColor = vec4(acc, 0.0, 0.0, 1.0);
}
