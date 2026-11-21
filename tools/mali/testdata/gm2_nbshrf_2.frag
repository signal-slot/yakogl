/* gm2_nbshrf_2 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan #367
 * study-next-shadow-refract): BOOL_AND with one comparison on a texel, the
 * chosen constant scaling a varying's lanes (shadow fs cf4b's shape after its
 * divide).
 * An original reduction, no glmark2 text. */
precision mediump float;
uniform sampler2D m;
varying vec4 sc;
varying vec4 col;
void main()
{
	float d = texture2D(m, vec2(0.5, 0.5)).x;
	float s = 1.0;
	if (sc.w > 0.0 && d < sc.z)
		s = 0.5;
	gl_FragColor = vec4(s * col.rgb, 1.0);
}
