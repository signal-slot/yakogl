/* Shadow map: a vec4 divided by its w, an add on z, a depth look-up and a
 * compare against it in a condition, at mediump. An original reduction of the
 * fragment shader of glmark2's shadow scene. */
precision mediump float;
varying vec4 col;
varying vec4 sc;
uniform sampler2D shadowmap;
void main()
{
	vec4 p = sc / sc.w;
	p.z += 0.1505;
	float dist = texture2D(shadowmap, p.st).x;
	float shadow = 1.0;
	if (sc.w > 0.0 && dist < p.z) {
		shadow = 0.5;
	}
	gl_FragColor = vec4(shadow * col.rgb, 1.0);
}
