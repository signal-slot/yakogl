/* Scalar operations applied to vectors. One scalar instruction computes one
 * lane, so `sin(v)` emitted once was sin(v.x) in every lane. With
 * u_v = (pi/6, pi/2, 0.0, 4.0): sin of the first three is (0.5, 1.0, 0.0),
 * and the SECOND lane of inversesqrt(vec2(1.0, 4.0)) is 0.5 -- a broadcast
 * of the first lane would give 1.0 there. gl_test case 77. */
precision mediump float;
uniform vec4 u_v;
void main()
{
	vec3 s = sin(u_v.xyz);
	vec2 r = inversesqrt(vec2(1.0, u_v.w));
	gl_FragColor = vec4(s, r.y);
}
