/* gm2_nbshrf_9 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan #367
 * study-next-shadow-refract): three chained look-ups, each coordinate computed
 * from the texels before it (TEX_DEPENDENT, the chain of glmark2 refract fs
 * 3fae with its refractions replaced by products).
 * An original reduction, no glmark2 text. */
precision mediump float;
uniform sampler2D m0;
uniform sampler2D m1;
uniform sampler2D m2;
varying vec3 vn;
varying vec4 vp;
void main()
{
	vec3 e = vp.xyz + vn * 1.5;
	float d = texture2D(m0, e.st * 0.5 + 0.5).x;
	vec3 b = vp.xyz + e * d;
	vec3 n = texture2D(m1, b.st * 0.5 + 0.5).xyz;
	vec3 r = b + n * 0.75;
	gl_FragColor = texture2D(m2, r.st * 0.5 + 0.5);
}
