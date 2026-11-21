/* gm2_nbshrf_6 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan #367
 * study-next-shadow-refract): refract() of two normalized varyings by a
 * constant ratio (FS_REFRACT: the front end's `k < 0.0` select of a computed
 * vector).
 * An original reduction, no glmark2 text. */
precision mediump float;
varying vec3 vn;
varying vec4 vp;
void main()
{
	vec3 r = refract(normalize(vp.xyz), normalize(vn), 1.5);
	gl_FragColor = vec4(r * 0.5 + 0.5, 1.0);
}
