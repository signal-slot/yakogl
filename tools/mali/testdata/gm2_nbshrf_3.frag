/* gm2_nbshrf_3 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan #367
 * study-next-shadow-refract): a lone look-up at two lanes of a vec4 varying
 * divided by its w (a projective shadow-map coordinate, TEX_COORD2).
 * An original reduction, no glmark2 text. */
precision mediump float;
uniform sampler2D m;
varying vec4 sc;
void main()
{
	vec4 p = sc / sc.w;
	gl_FragColor = texture2D(m, p.st);
}
