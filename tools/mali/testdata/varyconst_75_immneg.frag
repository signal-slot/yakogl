/* Literals subtracted from uniforms: u_k - 0.5 and u_k - 1.0 in two lanes,
 * and a literal minus a uniform (a register negate, which was always right)
 * in the third. With u_k = (1.0, 1.25, 0.25): (0.5, 0.25, 0.75). With -0.5
 * and -1.0 encoded as 2.7e-20 and 5.4e-20 the first two lanes saturate to
 * 1.0 instead. Case 75. */
precision mediump float;
uniform vec4 u_k;
void main()
{
	gl_FragColor = vec4(u_k.x - 0.5, u_k.y - 1.0, 1.0 - u_k.z, 1.0);
}
