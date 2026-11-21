/* gm2_nbshrf_4 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan #367
 * study-next-shadow-refract): a lone look-up at two computed lanes of a vec4
 * varying, with no divide (TEX_COORD2 without the ARITH in front of it).
 * An original reduction, no glmark2 text. */
precision mediump float;
uniform sampler2D m;
varying vec4 sc;
void main()
{
	gl_FragColor = texture2D(m, sc.st * 0.5 + 0.5);
}
