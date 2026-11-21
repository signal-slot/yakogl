/* gm2_nbshrf_1 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan #367
 * study-next-shadow-refract): a run-time `&&` of two comparisons choosing between two constants
 * (the condition of glmark2 shadow fs cf4b, BOOL_AND).
 * An original reduction, no glmark2 text. */
precision mediump float;
varying vec4 sc;
void main()
{
	float s = 1.0;
	if (sc.w > 0.0 && sc.x < sc.z)
		s = 0.5;
	gl_FragColor = vec4(s, s, s, 1.0);
}
