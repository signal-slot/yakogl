/* gm2_nbshrf_8 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan #367
 * study-next-shadow-refract): two look-ups, the second at a coordinate
 * computed from the first one's texel (TEX_DEPENDENT).
 * An original reduction, no glmark2 text. */
precision mediump float;
uniform sampler2D m0;
uniform sampler2D m1;
varying vec3 vn;
varying vec4 vp;
void main()
{
	float d = texture2D(m0, vp.xy * 0.5 + 0.5).x;
	vec3 q = vp.xyz + vn * d;
	gl_FragColor = texture2D(m1, q.st * 0.5 + 0.5);
}
