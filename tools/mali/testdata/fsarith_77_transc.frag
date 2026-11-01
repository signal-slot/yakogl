/* The transcendentals, one lane each, at arguments whose results are exact
 * in eight bits: with u_a = (pi/6, pi/3, 2.0, -1.0) the colour is
 * (sin, cos, log2 / 4, exp2) = (0.5, 0.5, 0.25, 0.5). On this core sin and
 * cos take their argument in units of pi, and sin, cos and log write a pair
 * of lanes whose product is the result (Mesa's etna_lower_alu); a plain
 * `sin` of radians computes something else. gl_test case 77. */
precision mediump float;
uniform vec4 u_a;
void main()
{
	gl_FragColor = vec4(sin(u_a.x), cos(u_a.y), log2(u_a.z) * 0.25, exp2(u_a.w));
}
