/* A vec4 varying divided by its w at mediump, the xyz stored: the divide of
 * glmark2's shadow scene (fs cf4b) with no texture look-up behind it. An
 * original reduction. */
precision mediump float;
varying vec4 sc;
void main()
{
	vec4 p = sc / sc.w;
	gl_FragColor = vec4(p.xyz, 1.0);
}
