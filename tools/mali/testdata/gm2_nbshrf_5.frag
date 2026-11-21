/* gm2_nbshrf_5 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan #367
 * study-next-shadow-refract): the whole projective shadow test: divide by w,
 * a depth bias, the look-up at the divided lanes, a two-sided `&&` and the
 * chosen constant scaling a varying colour (BOOL_AND and TEX_COORD2 together).
 * An original reduction, no glmark2 text. */
precision mediump float;
uniform sampler2D m;
varying vec4 sc;
varying vec4 col;
void main()
{
	vec4 p = sc / sc.w;
	p.z += 0.25;
	float d = texture2D(m, p.st).x;
	float s = 1.0;
	if (sc.w > 0.0 && d < p.z)
		s = 0.5;
	gl_FragColor = vec4(s * col.rgb, 1.0);
}
