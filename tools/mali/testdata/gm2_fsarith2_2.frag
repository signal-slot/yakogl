/* The reciprocal of a mediump uniform's lanes added to a varying: the
 * `1.0 / resolution.x` of glmark2's terrain normal-map pass (fs c549), whose
 * sum is a look-up coordinate there; here it is the stored colour. An
 * original reduction. */
precision mediump float;
uniform vec2 res;
varying vec2 uv;
void main()
{
	vec2 st = vec2(1.0 / res.x, 1.0 / res.y);
	gl_FragColor = vec4(uv + st, 0.0, 1.0);
}
