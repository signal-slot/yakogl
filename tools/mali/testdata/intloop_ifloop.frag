/* A one-armed `if` inside a loop, its condition an integer compare on the
 * loop counter, its arm rotating a loop-carried vector: dEQP
 * loops.*.conditional_body. With (1, 2, 3, 4) in the colour it draws
 * (3, 4, 1): two rotations of three trips. The if-conversion's select read
 * its condition as .xyzw where only lane x held the compare, so lanes y..w
 * rotated on every trip and the picture was (3, 1, 2). */
precision mediump float;
varying vec4 v_color;
void main()
{
	vec4 res = v_color;
	int i = 0;
	do {
		if (i < 2)
			res = res.yzwx;
	} while (++i < 3);
	gl_FragColor = vec4(res.rgb, 1.0);
}
