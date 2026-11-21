/* pow of a clamped dot product by a mediump uniform: the specular weight of
 * glmark2's terrain lighting (fs 4d26). An original reduction. */
precision mediump float;
uniform float shin;
varying vec3 n;
varying vec3 h;
void main()
{
	float d = max(dot(normalize(n), normalize(h)), 0.0);
	float s = pow(d, shin);
	gl_FragColor = vec4(s, s, s, 1.0);
}
