/* gm2_nbshrf_7 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan #367
 * study-next-shadow-refract): a constant's reciprocal that fp16 does not hold
 * exactly (`1.0 / 1.2`), scaling a varying (CONST_RCP16).
 * An original reduction, no glmark2 text. */
precision mediump float;
const float ratio = 1.2;
varying vec4 sc;
void main()
{
	gl_FragColor = vec4(sc.xyz * (1.0 / ratio), 1.0);
}
