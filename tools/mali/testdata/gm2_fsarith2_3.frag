/* normalize of a vec3 made of two differences and a uniform, scaled and
 * biased into a colour: the normal of glmark2's terrain normal-map pass (fs
 * c549). An original reduction. */
precision mediump float;
uniform float h;
varying vec3 d;
void main()
{
	vec3 n = normalize(vec3(d.x - d.y, d.x - d.z, h));
	gl_FragColor = vec4(0.5 * n + 0.5, 1.0);
}
