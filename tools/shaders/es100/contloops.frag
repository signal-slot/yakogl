/* `continue` in each loop form, the shapes of dEQP loops.*: only a
 * continue, a conditional one, and break and continue mixed, in `for`,
 * `while` and `do`/`while`. Each loop carries a vector that the lanes of
 * the colour depend on, so a continue that skips the wrong work -- or a
 * do/while continue that skips the test -- changes the picture, and the
 * trip counts come from a varying, so nothing folds away. */
precision mediump float;
varying vec4 v_color;
void main()
{
	vec4 res = v_color;
	int n = int(v_color.w * 4.0 + 3.5);	/* 3 .. 7 across the quad */

	for (int i = 0; i < 8; i++) {
		if (i >= n)
			break;
		if (i == 1)
			continue;
		res = res.yzwx;
	}
	int k = 0;
	while (k < n) {
		k++;
		if (k == 2)
			continue;
		res.x += 0.125;
	}
	int j = 0;
	do {
		continue;
	} while (++j < 3);
	int m = 0;
	do {
		if (m == 4)
			break;
		if (m == 1)
			continue;
		res.y += 0.0625;
	} while (++m < n);
	gl_FragColor = vec4(res.xyz, float(j) * 0.25);
}
