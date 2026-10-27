/* A loop whose break point differs from pixel to pixel, and a continue taken
 * in half of them (case 87). The online PowerVR path keeps a loop like this
 * as ONE loop: its body once, every exit a flag, and a backward branch on a
 * counter that is the same in every instance. The instances therefore
 * disagree about where their loop stops while they all take the branch
 * together, and this is the case that shows the flags doing that job.
 *
 * Case 87 draws v_color = (1 - t, t, 1 - s, s), s the horizontal position and
 * t the vertical one, both 0..1 across the quad. The loop counts iterations
 * until i reaches s * 8, so band b of eight (left to right) counts b + 1;
 * above the middle it skips iteration 2, so from band 2 up it counts one
 * fewer. Red is the count over 8; blue says which half the pixel is in. */
precision highp float;
varying vec4 v_color;
void main()
{
	float s = v_color.w;
	float top = v_color.y;
	float count = 0.0;
	for (int i = 0; i < 8; i++) {
		if (float(i) >= s * 8.0) {
			break;
		}
		if (top > 0.5 && i == 2) {
			continue;
		}
		count += 1.0;
	}
	gl_FragColor = vec4(count / 8.0, 0.0, top > 0.5 ? 1.0 : 0.0, 1.0);
}
