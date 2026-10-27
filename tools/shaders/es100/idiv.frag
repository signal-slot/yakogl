/* Integer division, truncated toward zero, on both signs and in every lane
 * of a vector: the online compiler refused it (Lower(Unsupported(Idiv)),
 * 148 dEQP case-sides). Divisors are kept at 1 or more so any input is
 * defined. With u_d = (0.7, -0.7, 1.0, 1.0): n = (7, -7), d = (2, 3),
 * n / d = (3, -2), and 100 / (|7| + 1) = 12; the colour is
 * (3 / 4, (-2 + 4) / 4, 12 / 16, 1) = (0.75, 0.5, 0.75, 1.0). gl_test case 79. */
precision mediump float;
uniform vec4 u_d;
void main()
{
	ivec2 n = ivec2(int(u_d.x * 10.0), int(u_d.y * 10.0));
	ivec2 d = ivec2(int(abs(u_d.z)) + 1, int(abs(u_d.w)) + 2);
	ivec2 q = n / d;
	int big = 100 / (int(abs(u_d.x * 10.0)) + 1);
	gl_FragColor = vec4(float(q.x) / 4.0, float(q.y + 4) / 4.0, float(big) / 16.0, 1.0);
}
